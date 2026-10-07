/**
 * Fetches the readings of Wikidata's items from QLever into their own raw
 * store and writes them out for `just check-wikidata`, as
 * scripts/wikidata/files.ts sets.
 *
 *     deno run -A scripts/wikidata.ts [--refresh]
 *
 * The raw store is build/wikidata/raw; the file is
 * build/wikidata/readings.tsv. The readings are fetched again only with
 * --refresh.
 */
import { join } from "@std/path";
import { fetchToStore } from "./raw/fetch.ts";
import { RawStore } from "./raw/store.ts";
import { extractReadings, fetchReadings } from "./wikidata/files.ts";

if (import.meta.main) {
  const outDir = "build/wikidata";
  const rawDir = join(outDir, "raw");
  const store = new RawStore(rawDir);
  const manifest = join(rawDir, "manifest.jsonl");
  const refresh = Deno.args.includes("--refresh");
  await fetchReadings(async (sourceId, url, options) => {
    const record = await fetchToStore(store, manifest, sourceId, url, { ...options, refresh });
    console.log(`${record.source_id}\t${record.bytes}\t${record.retrieved_at}`);
    return record;
  });
  console.log(await extractReadings(rawDir, outDir));
}
