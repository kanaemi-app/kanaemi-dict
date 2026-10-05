import { assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import { fetchToStore, NotAccepted, Stopped } from "./fetch.ts";
import { appendRecord, loadManifest } from "./manifest.ts";
import { RawStore } from "./store.ts";

async function withServer(
  handler: (req: Request) => Response | Promise<Response>,
  body: (base: string) => Promise<void>,
): Promise<void> {
  const server = Deno.serve({ port: 0, hostname: "127.0.0.1", onListen() {} }, handler);
  try {
    await body(`http://127.0.0.1:${server.addr.port}`);
  } finally {
    await server.shutdown();
  }
}

async function setup() {
  const dir = await Deno.makeTempDir();
  return { store: new RawStore(dir), manifest: join(dir, "manifest.jsonl") };
}

Deno.test("a fetch stores the content and records where it came from", async () => {
  const { store, manifest } = await setup();
  let userAgent = "";
  await withServer((req) => {
    userAgent = req.headers.get("user-agent") ?? "";
    return new Response("中身");
  }, async (base) => {
    const record = await fetchToStore(store, manifest, "pydocs-ja", `${base}/a.zip`);

    assertEquals(new TextDecoder().decode(await store.read(record.sha256)), "中身");
    assertEquals((await loadManifest(manifest)).map((r) => [r.source_id, r.url, r.bytes]), [
      ["pydocs-ja", `${base}/a.zip`, 6],
    ]);
  });
  assertEquals(userAgent.includes("https://github.com/kanaemi-app/kanaemi-dict"), true);
});

Deno.test("a URL already fetched is not fetched again unless refreshed", async () => {
  const { store, manifest } = await setup();
  let hits = 0;
  await withServer(() => {
    hits++;
    return new Response(`版${hits}`);
  }, async (base) => {
    await fetchToStore(store, manifest, "s", `${base}/a`);
    await fetchToStore(store, manifest, "s", `${base}/a`);
    assertEquals(hits, 1);

    await fetchToStore(store, manifest, "s", `${base}/a`, { refresh: true });

    assertEquals(hits, 2);
    assertEquals((await loadManifest(manifest)).length, 2);
  });
});

Deno.test("a URL known under another source is recorded for this source without refetching", async () => {
  const { store, manifest } = await setup();
  let hits = 0;
  await withServer(() => {
    hits++;
    return new Response("中身");
  }, async (base) => {
    await fetchToStore(store, manifest, "w", `${base}/a`);

    const record = await fetchToStore(store, manifest, "x", `${base}/a`);

    assertEquals(hits, 1);
    assertEquals(record.source_id, "x");
    assertEquals((await loadManifest(manifest)).map((r) => r.source_id), ["w", "x"]);
  });
});

Deno.test("switching a source back to a URL it had before records the switch", async () => {
  const { store, manifest } = await setup();
  let hits = 0;
  await withServer((req) => {
    hits++;
    return new Response(new URL(req.url).pathname);
  }, async (base) => {
    await fetchToStore(store, manifest, "pydocs-ja", `${base}/A`);
    await fetchToStore(store, manifest, "pydocs-ja", `${base}/B`);

    await fetchToStore(store, manifest, "pydocs-ja", `${base}/A`);

    assertEquals(hits, 2);
    assertEquals((await loadManifest(manifest)).at(-1)?.url, `${base}/A`);
  });
});

Deno.test("requests to one host keep its minimum interval, retries included", async () => {
  const { store, manifest } = await setup();
  const times: number[] = [];
  await withServer(() => {
    times.push(performance.now());
    return times.length === 2 ? new Response("busy", { status: 503 }) : new Response("ok");
  }, async (base) => {
    const options = { minIntervalMs: 100, backoffMs: 1 };
    await fetchToStore(store, manifest, "s", `${base}/a`, options);
    await fetchToStore(store, manifest, "s", `${base}/b`, options);
  });

  assertEquals(times.length, 3);
  for (let i = 1; i < times.length; i++) {
    assertEquals(times[i] - times[i - 1] >= 95, true, `gap ${times[i] - times[i - 1]}ms`);
  }
});

Deno.test("requests to a host that sets no interval are a second apart", async () => {
  const { store, manifest } = await setup();
  const times: number[] = [];
  await withServer(() => {
    times.push(performance.now());
    return new Response("ok");
  }, async (base) => {
    await fetchToStore(store, manifest, "s", `${base}/a`);
    await fetchToStore(store, manifest, "s", `${base}/b`);
  });

  assertEquals(times.length, 2);
  assertEquals(times[1] - times[0] >= 950, true, `gap ${times[1] - times[0]}ms`);
});

Deno.test("concurrent requests to one host still keep its minimum interval", async () => {
  const { store, manifest } = await setup();
  const times: number[] = [];
  await withServer(() => {
    times.push(performance.now());
    return new Response("ok");
  }, async (base) => {
    await Promise.all(
      ["a", "b", "c"].map((path) =>
        fetchToStore(store, manifest, "s", `${base}/${path}`, { minIntervalMs: 100 })
      ),
    );
  });

  assertEquals(times.length, 3);
  for (let i = 1; i < times.length; i++) {
    assertEquals(times[i] - times[i - 1] >= 95, true, `gap ${times[i] - times[i - 1]}ms`);
  }
});

Deno.test("a response that stalls times out", async () => {
  const { store, manifest } = await setup();
  await withServer(
    () => new Response(new ReadableStream({ start() {} })),
    async (base) => {
      await assertRejects(() =>
        fetchToStore(store, manifest, "s", `${base}/stall`, { retries: 0, timeoutMs: 50 })
      );
    },
  );
});

Deno.test("failed fetches are retried, then given up", async () => {
  const { store, manifest } = await setup();
  let hits = 0;
  await withServer((req) => {
    if (req.url.endsWith("/never")) return new Response("down", { status: 503 });
    hits++;
    return hits < 3 ? new Response("busy", { status: 503 }) : new Response("ok");
  }, async (base) => {
    await fetchToStore(store, manifest, "s", `${base}/flaky`, { retries: 3, backoffMs: 1 });
    assertEquals(hits, 3);

    await assertRejects(() =>
      fetchToStore(store, manifest, "s", `${base}/never`, { retries: 1, backoffMs: 1 })
    );
  });
});

Deno.test("a response of a type not accepted is neither stored nor recorded", async () => {
  const { store, manifest } = await setup();
  await withServer(
    (req) =>
      new Response("x", {
        headers: { "content-type": req.url.endsWith(".html") ? "text/html" : "application/pdf" },
      }),
    async (base) => {
      const accept = (type: string) => type.startsWith("text/html");

      await assertRejects(
        () => fetchToStore(store, manifest, "s", `${base}/a.pdf`, { accept }),
        NotAccepted,
      );
      await fetchToStore(store, manifest, "s", `${base}/a.html`, { accept });

      assertEquals((await loadManifest(manifest)).map((r) => r.url), [`${base}/a.html`]);
    },
  );
});

Deno.test("a missing page is given up at once", async () => {
  const { store, manifest } = await setup();
  let hits = 0;
  await withServer(() => {
    hits++;
    return new Response("gone", { status: 404 });
  }, async (base) => {
    await assertRejects(() =>
      fetchToStore(store, manifest, "s", `${base}/gone`, { retries: 3, backoffMs: 1 })
    );
  });

  assertEquals(hits, 1);
});

Deno.test("no download starts once the stop signal fires, but stored URLs still come back", async () => {
  const { store, manifest } = await setup();
  let hits = 0;
  await withServer(() => {
    hits++;
    return new Response("ok");
  }, async (base) => {
    const stop = new AbortController();
    await fetchToStore(store, manifest, "s", `${base}/a`, { stop: stop.signal });
    stop.abort();

    const again = await fetchToStore(store, manifest, "s", `${base}/a`, { stop: stop.signal });
    await assertRejects(
      () => fetchToStore(store, manifest, "s", `${base}/b`, { stop: stop.signal }),
      Stopped,
    );

    assertEquals(again.url, `${base}/a`);
  });

  assertEquals(hits, 1);
});

Deno.test("records another writer appends between fetches are seen", async () => {
  const { store, manifest } = await setup();
  let hits = 0;
  await withServer(() => {
    hits++;
    return new Response("ok");
  }, async (base) => {
    await fetchToStore(store, manifest, "s", `${base}/a`);
    const data = new TextEncoder().encode("b");
    await appendRecord(manifest, {
      source_id: "s",
      url: `${base}/b`,
      sha256: await store.put(data),
      bytes: data.length,
      retrieved_at: "2026-10-04T00:00:00+0900",
    });

    await fetchToStore(store, manifest, "s", `${base}/b`);
  });

  assertEquals(hits, 1);
});

Deno.test("no retry starts once the stop signal fires", async () => {
  const { store, manifest } = await setup();
  let hits = 0;
  const stop = new AbortController();
  await withServer(() => {
    hits++;
    stop.abort();
    return new Response("busy", { status: 503 });
  }, async (base) => {
    await assertRejects(
      () =>
        fetchToStore(store, manifest, "s", `${base}/a`, {
          retries: 3,
          backoffMs: 1,
          stop: stop.signal,
        }),
      Stopped,
    );
  });

  assertEquals(hits, 1);
});

Deno.test("acceptance sees where a redirect ended", async () => {
  const { store, manifest } = await setup();
  const seen: string[] = [];
  await withServer(
    (req) =>
      req.url.endsWith("/old")
        ? Response.redirect(new URL("/private/new", req.url), 302)
        : new Response("x", { headers: { "content-type": "text/html" } }),
    async (base) => {
      const accept = (_type: string, url: string) => {
        seen.push(new URL(url).pathname);
        return !url.includes("/private/");
      };

      await assertRejects(
        () => fetchToStore(store, manifest, "s", `${base}/old`, { accept }),
        NotAccepted,
      );
    },
  );

  assertEquals(seen, ["/private/new"]);
});

Deno.test("a large download streams into the store", async () => {
  const { store, manifest } = await setup();
  await withServer(() => new Response("大きな中身"), async (base) => {
    const record = await fetchToStore(store, manifest, "s", `${base}/big`, { large: true });

    assertEquals(new TextDecoder().decode(await store.read(record.sha256)), "大きな中身");
    assertEquals(record.bytes, 15);
  });
});
