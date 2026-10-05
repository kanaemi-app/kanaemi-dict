/**
 * Fetches SudachiDict into its own raw store and writes out the files the
 * build reads, as scripts/sudachi/files.ts sets.
 *
 *     deno run -A scripts/sudachi.ts
 *
 * The raw store is build/sudachi/raw; the files are
 * build/sudachi/system_full.dic and build/sudachi/small_lex.csv. A URL
 * already stored is not fetched again.
 */
import { join } from "@std/path";
import { fetchToStore } from "./raw/fetch.ts";
import { RawStore } from "./raw/store.ts";
import { extractSudachiFile, fetchSudachi, SUDACHI_FILES } from "./sudachi/files.ts";

if (import.meta.main) {
  const outDir = "build/sudachi";
  const rawDir = join(outDir, "raw");
  const store = new RawStore(rawDir);
  const manifest = join(rawDir, "manifest.jsonl");
  await fetchSudachi(async (sourceId, url, options) => {
    const record = await fetchToStore(store, manifest, sourceId, url, options);
    console.log(`${record.source_id}\t${record.bytes}`);
    return record;
  });
  for (const file of SUDACHI_FILES) console.log(await extractSudachiFile(rawDir, file, outDir));
}
