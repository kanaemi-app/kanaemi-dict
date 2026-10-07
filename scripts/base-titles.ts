/**
 * Writes the titles of the Wikipedia articles into build/base-titles.tsv, with
 * the readings their leads give, and the names of every article and redirect
 * into build/base-names.txt, for the base dictionary to take those its
 * documents use, as docs/spec/dictionary.md sets.
 *
 *     deno run -A scripts/base-titles.ts
 *
 * The Wikipedia dump comes from build/raw.
 */
import { join } from "@std/path";
import { bzip2Text } from "./docs/mediawiki.ts";
import { forBuild, loadManifest } from "./raw/manifest.ts";
import { RawStore } from "./raw/store.ts";
import { baseTitlesOf, writeTitlesTo } from "./additional/wikipedia.ts";

const TITLES = join("build", "base-titles.tsv");
const NAMES = join("build", "base-names.txt");
const RAW = join("build", "raw");

async function writeBaseTitles(): Promise<void> {
  const records = forBuild(await loadManifest(join(RAW, "manifest.jsonl")));
  const dump = records.find((r) => r.source_id === "wikipedia-ja");
  if (!dump) throw new Error(`no wikipedia-ja in ${RAW}; fetch it first`);
  const { titles, names } = await baseTitlesOf(bzip2Text(new RawStore(RAW).path(dump.sha256)));
  await writeTitlesTo(TITLES, titles);
  await Deno.writeTextFile(NAMES, names.sort().map((n) => `${n}\n`).join(""));
  console.log(`titles: ${titles.length}\tnames: ${names.length}\tout: ${TITLES}, ${NAMES}`);
}

if (import.meta.main) {
  await writeBaseTitles();
}
