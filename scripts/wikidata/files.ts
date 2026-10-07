/**
 * The readings in kana (P1814) of Wikidata's items, with their Japanese
 * labels and what each item is (P31), asked of QLever. They check the base
 * dictionary's readings and are never taken into a dictionary. QLever's
 * index moves, so a fetch is kept in a raw store of its own and fetched
 * again only when asked.
 */
import { dirname, join } from "@std/path";
import type { FetchOptions } from "../raw/fetch.ts";
import { forBuild, loadManifest, type Record } from "../raw/manifest.ts";
import { RawStore } from "../raw/store.ts";

const SOURCE_ID = "wikidata-readings";

const QUERY = `PREFIX wdt: <http://www.wikidata.org/prop/direct/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?item ?kana ?label ?p31 WHERE {
  ?item wdt:P1814 ?kana .
  OPTIONAL { ?item rdfs:label ?label . FILTER(LANG(?label)="ja") }
  OPTIONAL { ?item wdt:P31 ?p31 }
}`;

export const READINGS_URL = `https://qlever.dev/api/wikidata?${new URLSearchParams({
  query: QUERY,
  action: "tsv_export",
})}`;

/** Fetches `url` into the raw store as a record of `sourceId`, unless it is stored already. */
export type Get = (sourceId: string, url: string, options?: FetchOptions) => Promise<Record>;

/** Fetches the readings, some tens of megabytes. */
export async function fetchReadings(get: Get): Promise<void> {
  await get(SOURCE_ID, READINGS_URL, { large: true });
}

/**
 * Writes the latest record of the readings in the raw store under `rawDir`
 * to `outDir` as readings.tsv, and returns where it went. The file is written
 * beside its place and renamed into it.
 */
export async function extractReadings(rawDir: string, outDir: string): Promise<string> {
  const records = await loadManifest(join(rawDir, "manifest.jsonl"));
  const record = forBuild(records).find((r) => r.source_id === SOURCE_ID);
  if (!record) throw new Error(`no ${SOURCE_ID} record in ${rawDir}`);
  const bytes = await new RawStore(rawDir).read(record.sha256);
  const out = join(outDir, "readings.tsv");
  const partial = `${out}.${Deno.pid}.tmp`;
  await Deno.mkdir(dirname(out), { recursive: true });
  try {
    await Deno.writeFile(partial, bytes);
    await Deno.rename(partial, out);
  } catch (e) {
    await Deno.remove(partial).catch(() => {});
    throw e;
  }
  return out;
}
