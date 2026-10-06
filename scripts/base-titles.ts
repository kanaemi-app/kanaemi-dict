/**
 * Writes the titles of the Wikipedia articles in the categories of
 * base/wikipedia.txt into build/base-titles.tsv, with the readings their
 * leads give, for the base dictionary to take those its documents use, as
 * docs/spec/dictionary.md sets.
 *
 *     deno run -A scripts/base-titles.ts
 *
 * The Wikipedia dump comes from build/raw.
 */
import { join } from "@std/path";
import { bzip2Text } from "./docs/mediawiki.ts";
import { forBuild, loadManifest } from "./raw/manifest.ts";
import { RawStore } from "./raw/store.ts";
import { readPatterns } from "./additional/config.ts";
import { articlesOf, writeTitlesTo } from "./additional/wikipedia.ts";

const PATTERNS = join("base", "wikipedia.txt");
const OUT = join("build", "base-titles.tsv");
const RAW = join("build", "raw");
/** The name the titles are gathered under; no budget applies to titles alone. */
const NAME = "base";

async function writeBaseTitles(): Promise<void> {
  const patterns = await readPatterns(PATTERNS);
  if (patterns === undefined) throw new Error(`no ${PATTERNS}`);
  const records = forBuild(await loadManifest(join(RAW, "manifest.jsonl")));
  const dump = records.find((r) => r.source_id === "wikipedia-ja");
  if (!dump) throw new Error(`no wikipedia-ja in ${RAW}; fetch it first`);
  // As a year's dictionary does from its first page on: every title of the
  // categories, and no text.
  const found = await articlesOf(
    bzip2Text(new RawStore(RAW).path(dump.sha256)),
    [{ name: NAME, patterns, since: 0 }],
    dump.source_id,
    0,
  );
  const titles = found.get(NAME)?.titles ?? [];
  await writeTitlesTo(OUT, titles);
  console.log(`titles: ${titles.length}\tout: ${OUT}`);
}

if (import.meta.main) {
  await writeBaseTitles();
}
