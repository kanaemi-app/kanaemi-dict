/**
 * Fetches the lists of single kanji readings into their own raw store and
 * writes out the files the build reads, as scripts/kanji/files.ts sets.
 *
 *     deno run -A scripts/kanji.ts
 *
 * The raw store is build/kanji/raw; the files are build/kanji/single_kanji.tsv
 * and build/kanji/Unihan_Readings.txt. A URL already stored is not fetched
 * again.
 */
import { join } from "@std/path";
import { fetchToStore } from "./raw/fetch.ts";
import { RawStore } from "./raw/store.ts";
import { extractKanjiFile, fetchKanji, KANJI_FILES } from "./kanji/files.ts";

if (import.meta.main) {
  const outDir = "build/kanji";
  const rawDir = join(outDir, "raw");
  const store = new RawStore(rawDir);
  const manifest = join(rawDir, "manifest.jsonl");
  await fetchKanji(async (sourceId, url, options) => {
    const record = await fetchToStore(store, manifest, sourceId, url, options);
    console.log(`${record.source_id}\t${record.bytes}`);
    return record;
  });
  for (const file of KANJI_FILES) console.log(await extractKanjiFile(rawDir, file, outDir));
}
