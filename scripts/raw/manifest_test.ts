import { assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import {
  appendRecord,
  forBuild,
  latestForUrl,
  loadManifest,
  ManifestError,
  type Record,
} from "./manifest.ts";

function record(source_id: string, url: string, sha: string): Record {
  return {
    source_id,
    url,
    sha256: sha.repeat(64),
    bytes: 1,
    retrieved_at: "2026-10-02T11:07:41+0900",
  };
}

function picked(records: Record[]): [string, string, string][] {
  return forBuild(records).map((r) => [r.source_id, r.url, r.sha256[0]]);
}

Deno.test("loadManifest reads records however their JSON is spaced", async () => {
  const tmp = await Deno.makeTempDir();
  const path = join(tmp, "manifest.jsonl");
  await Deno.writeTextFile(
    path,
    '{"source_id": "aozora-index", "url": "https://www.aozora.gr.jp/a.zip", "sha256": "3dd8360217c0e82ed0ea520411117f236a5090096e885957374f5d286adbbfef", "bytes": 2065, "retrieved_at": "2026-10-02T11:07:41+0900"}\n' +
      '{"source_id":"fineweb2-jpn","url":"https://huggingface.co/f.parquet","sha256":"08fa1c538e421867b8551587157fdedb610491a2955a886b094f507b3eccfb5c","bytes":8706,"retrieved_at":"2026-10-02T12:00:00+0900"}\n',
  );

  const records = await loadManifest(path);

  assertEquals(records.length, 2);
  assertEquals(records[0].source_id, "aozora-index");
  assertEquals(records[0].bytes, 2065);
  assertEquals(records[1].source_id, "fineweb2-jpn");
});

Deno.test("loadManifest of a missing file is empty", async () => {
  const tmp = await Deno.makeTempDir();

  assertEquals(await loadManifest(join(tmp, "manifest.jsonl")), []);
});

Deno.test("loadManifest reports the line of a broken record", async () => {
  const tmp = await Deno.makeTempDir();
  const path = join(tmp, "manifest.jsonl");
  await Deno.writeTextFile(path, `${JSON.stringify(record("a", "u", "a"))}\n{"source_id": "b"}\n`);

  const err = await assertRejects(() => loadManifest(path), ManifestError);

  assertEquals(err.line, 2);
});

Deno.test("appendRecord adds a line that loadManifest reads back", async () => {
  const tmp = await Deno.makeTempDir();
  const path = join(tmp, "build/raw/manifest.jsonl");
  const first = record("a", "https://a", "a");
  const second = record("b", "https://b", "b");

  await appendRecord(path, first);
  await appendRecord(path, second);

  assertEquals(await loadManifest(path), [first, second]);
});

Deno.test("appendRecord starts on a new line after an unterminated last line", async () => {
  const tmp = await Deno.makeTempDir();
  const path = join(tmp, "manifest.jsonl");
  const first = record("a", "https://a", "a");
  await Deno.writeTextFile(path, JSON.stringify(first));
  const second = record("b", "https://b", "b");

  await appendRecord(path, second);

  assertEquals(await loadManifest(path), [first, second]);
});

Deno.test("concurrent appends after an unterminated line stay loadable", async () => {
  const tmp = await Deno.makeTempDir();
  const path = join(tmp, "manifest.jsonl");
  await Deno.writeTextFile(path, JSON.stringify(record("a", "https://a", "a")));

  await Promise.all(
    Array.from({ length: 16 }, (_, i) => appendRecord(path, record("b", `https://b/${i}`, "b"))),
  );

  assertEquals((await loadManifest(path)).length, 17);
});

Deno.test("the later record of a URL wins", () => {
  const records = [
    record("a", "https://a", "1"),
    record("b", "https://b", "2"),
    record("a", "https://a", "3"),
  ];

  assertEquals(latestForUrl(records, "https://a")?.sha256[0], "3");
  assertEquals(latestForUrl(records, "https://b")?.sha256[0], "2");
  assertEquals(latestForUrl(records, "https://c"), undefined);
});

Deno.test("forBuild uses the last record of each source even when its URL changed", () => {
  const records = [
    record("rurema", "https://old.tar.gz", "1"),
    record("pydocs-ja", "https://p/3.13.zip", "2"),
    record("rurema", "https://new.tar.gz", "3"),
    record("pydocs-ja", "https://p/3.14.zip", "4"),
  ];

  assertEquals(picked(records), [
    ["pydocs-ja", "https://p/3.14.zip", "4"],
    ["rurema", "https://new.tar.gz", "3"],
  ]);
});

Deno.test("forBuild orders sources by their IDs", () => {
  const records = [
    record("rurema", "https://r", "1"),
    record("aozora-text", "https://t", "2"),
    record("aozora-index", "https://i", "3"),
  ];

  assertEquals(picked(records).map(([id]) => id), ["aozora-index", "aozora-text", "rurema"]);
});
