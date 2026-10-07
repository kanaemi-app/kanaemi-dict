import { assertEquals, assertRejects, assertStringIncludes } from "@std/assert";
import { join } from "@std/path";
import type { FetchOptions } from "../raw/fetch.ts";
import { appendRecord, type Record } from "../raw/manifest.ts";
import { RawStore } from "../raw/store.ts";
import { extractReadings, fetchReadings, READINGS_URL } from "./files.ts";

async function addRecord(dir: string, url: string, text: string) {
  const bytes = new TextEncoder().encode(text);
  await appendRecord(join(dir, "manifest.jsonl"), {
    source_id: "wikidata-readings",
    url,
    sha256: await new RawStore(dir).put(bytes),
    bytes: bytes.length,
    retrieved_at: "2026-10-07T00:00:00+0900",
  });
}

Deno.test("the readings are asked of QLever as a TSV export", async () => {
  const fetched: [string, string][] = [];
  const get = (sourceId: string, url: string, _options?: FetchOptions) => {
    fetched.push([sourceId, url]);
    const record: Record = { source_id: sourceId, url, sha256: "", bytes: 0, retrieved_at: "" };
    return Promise.resolve(record);
  };

  await fetchReadings(get);

  assertEquals(fetched, [["wikidata-readings", READINGS_URL]]);
  const url = new URL(READINGS_URL);
  assertEquals(url.searchParams.get("action"), "tsv_export");
  assertStringIncludes(url.searchParams.get("query") ?? "", "wdt:P1814");
});

Deno.test("the latest record is written out as it is", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = join(await Deno.makeTempDir(), "wikidata");
  await addRecord(raw, "https://q/old", "古い");
  await addRecord(raw, "https://q/new", "?item\t?kana\n");

  const out = await extractReadings(raw, outDir);

  assertEquals(out, join(outDir, "readings.tsv"));
  assertEquals(await Deno.readTextFile(out), "?item\t?kana\n");
});

Deno.test("without a record there is nothing to write out", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = await Deno.makeTempDir();

  await assertRejects(() => extractReadings(raw, outDir), Error, "no wikidata-readings");
});
