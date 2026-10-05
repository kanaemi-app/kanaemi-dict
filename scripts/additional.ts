/**
 * Gathers what the additional dictionaries are built from, as
 * docs/spec/additional.md sets.
 *
 *     deno run -A scripts/additional.ts fetch [--year YEAR] [--refresh]
 *     deno run -A scripts/additional.ts docs
 *
 * `fetch` sets up the dictionary of YEAR's new words under additional/ if
 * it is not there (this year in Japan by default), and fetches into
 * build/additional/raw the postal code data and the hot entries of every
 * year with a dictionary. `docs` writes, for each dictionary under
 * additional/, what it is built from into build/additional/NAME/: the
 * documents of its Wikipedia articles, the laws, or the year's hot entries
 * as docs.jsonl, the titles of its articles as titles.tsv, and the postal
 * code data as ken_all.csv. The Wikipedia dump and the laws come from the
 * base dictionary's build/raw.
 */
import { join } from "@std/path";
import { compareUtf8, writeJsonl } from "./docs/extract.ts";
import type { Document } from "./docs/document.ts";
import { bzip2Text } from "./docs/mediawiki.ts";
import { type FetchOptions, fetchToStore, USER_AGENT } from "./raw/fetch.ts";
import { forBuild, loadManifest, type Record } from "./raw/manifest.ts";
import { RawStore } from "./raw/store.ts";
import { robotsAllows, robotsOf, robotsPath } from "./robots.ts";
import { laws } from "./sources/law.ts";
import { zipEntries } from "./zip.ts";
import { readDictionaries } from "./additional/config.ts";
import {
  daysOf,
  HATENA_HOTENTRY,
  HATENA_INTERVAL_MS,
  HATENA_SOURCE,
  hatenaHotentry,
  yearInJapan,
} from "./additional/hatena.ts";
import { articlesOf, type Title } from "./additional/wikipedia.ts";
import { NoArticleYet, setUpYear } from "./additional/year.ts";

const ADDITIONAL = "additional";
const BUILD = join("build", "additional");
const RAW = join(BUILD, "raw");
const BASE_RAW = join("build", "raw");

/** The dictionary built from the postal code data, and the one built from every law. */
const PLACE = "place";
const LAW = "law";

const POSTAL_ID = "japanpost-ken-all";
/** Japan Post's postal code data, every row in UTF-8. */
const POSTAL_URL =
  "https://www.post.japanpost.jp/service/search/zipcode/download/utf/zip/utf_ken_all.zip";
const POSTAL_CSV = "utf_ken_all.csv";

/** Characters a field takes of its articles' text. */
const FIELD_BUDGET = 5_000_000;
/** Characters the law dictionary takes of the laws. */
const LAW_BUDGET = 30_000_000;

async function fetchAll(year: number, refresh: boolean): Promise<void> {
  const store = new RawStore(RAW);
  const manifest = join(RAW, "manifest.jsonl");
  const get = async (sourceId: string, url: string, options: FetchOptions = {}) =>
    await fetchToStore(store, manifest, sourceId, url, { ...options, refresh });
  const postal = await get(POSTAL_ID, POSTAL_URL);
  console.log(`${postal.source_id}\t${postal.bytes}`);
  try {
    if (await setUpYear(ADDITIONAL, year)) console.log(`${ADDITIONAL}/${year}: set up`);
  } catch (e) {
    if (!(e instanceof NoArticleYet)) throw e;
    console.log(`${ADDITIONAL}/${year}: not set up, ${e.message}`);
  }
  const years = (await readDictionaries(ADDITIONAL)).filter((d) => d.since !== undefined);
  if (years.length === 0) return;
  const robots = await robotsOf(new URL(HATENA_HOTENTRY).host);
  if (robots === undefined) {
    throw new Error("Hatena Bookmark's robots.txt could not be read");
  }
  for (const { name } of years) {
    const days = daysOf(Number(name), new Date());
    for (const day of days) {
      if (!robotsAllows(robots, USER_AGENT, robotsPath(`${HATENA_HOTENTRY}${day}`))) {
        throw new Error(`Hatena Bookmark's robots.txt disallows ${HATENA_HOTENTRY}${day}`);
      }
      await get(`${HATENA_SOURCE}${day}`, `${HATENA_HOTENTRY}${day}`, {
        minIntervalMs: HATENA_INTERVAL_MS,
      });
    }
    console.log(`hatena ${name}\t${days.length} days`);
  }
}

async function writeDocs(): Promise<void> {
  const dictionaries = await readDictionaries(ADDITIONAL);
  const names = new Set(dictionaries.map((d) => d.name));
  const base = new RawStore(BASE_RAW);
  const baseRecords = forBuild(await loadManifest(join(BASE_RAW, "manifest.jsonl")));
  const own = new RawStore(RAW);
  const ownRecords = forBuild(await loadManifest(join(RAW, "manifest.jsonl")));
  const recordOf = (records: Record[], sourceId: string, where: string) => {
    const record = records.find((r) => r.source_id === sourceId);
    if (!record) throw new Error(`no ${sourceId} in ${where}; fetch it first`);
    return record;
  };

  if (dictionaries.some((d) => d.patterns !== undefined)) {
    const dump = recordOf(baseRecords, "wikipedia-ja", BASE_RAW);
    const found = await articlesOf(
      bzip2Text(base.path(dump.sha256)),
      dictionaries,
      dump.source_id,
      FIELD_BUDGET,
    );
    const years = new Set(dictionaries.filter((d) => d.since !== undefined).map((d) => d.name));
    for (const [name, { docs, titles }] of found) {
      // A year's documents are its hot entries, written below.
      if (!years.has(name)) await writeDocuments(name, docs);
      await writeTitles(name, titles);
    }
  }
  if (names.has(LAW)) {
    const egov = recordOf(baseRecords, "egov-all-xml", BASE_RAW);
    await writeDocuments(LAW, await laws(egov.source_id, await base.read(egov.sha256), LAW_BUDGET));
  }
  for (const { name } of dictionaries.filter((d) => d.since !== undefined)) {
    const docs: Document[] = [];
    for (const record of ownRecords) {
      if (record.source_id.startsWith(`${HATENA_SOURCE}${name}`)) {
        docs.push(...hatenaHotentry(record.source_id, await own.read(record.sha256)));
      }
    }
    await writeDocuments(name, docs);
  }
  if (names.has(PLACE)) {
    const postal = recordOf(ownRecords, POSTAL_ID, RAW);
    let csv: Uint8Array | undefined;
    for await (const entry of zipEntries(await own.read(postal.sha256))) {
      if (entry.name === POSTAL_CSV) csv = await entry.bytes();
    }
    if (!csv) throw new Error(`${POSTAL_URL} has no ${POSTAL_CSV}`);
    await Deno.mkdir(join(BUILD, PLACE), { recursive: true });
    await Deno.writeFile(join(BUILD, PLACE, "ken_all.csv"), csv);
    console.log(`${PLACE}\trows: ${new TextDecoder().decode(csv).split("\n").length - 1}`);
  }
}

/** Writes a dictionary's documents sorted by doc ID, as the units are cut. */
async function writeDocuments(name: string, docs: Document[]): Promise<void> {
  docs.sort((a, b) => compareUtf8(a.doc_id, b.doc_id));
  const out = join(BUILD, name, "docs.jsonl");
  await writeJsonl(out, docs);
  const chars = docs.reduce((n, d) => n + [...d.text].length, 0);
  console.log(`${name}\tdocs: ${docs.length}\tchars: ${chars}\tout: ${out}`);
}

async function writeTitles(name: string, titles: Title[]): Promise<void> {
  const out = join(BUILD, name, "titles.tsv");
  await Deno.mkdir(join(BUILD, name), { recursive: true });
  const lines = titles.map((t) => `${t.doc_id}\t${t.reading}\t${t.surface}\n`).sort();
  await Deno.writeTextFile(out, lines.join(""));
  console.log(`${name}\ttitles: ${titles.length}\tout: ${out}`);
}

if (import.meta.main) {
  const [command, ...rest] = Deno.args;
  if (command === "fetch") {
    let year = yearInJapan(new Date());
    let refresh = false;
    for (let i = 0; i < rest.length; i++) {
      if (rest[i] === "--year") year = Number(rest[++i]);
      else if (rest[i] === "--refresh") refresh = true;
      else {
        console.error(`unknown argument: ${rest[i]}`);
        Deno.exit(2);
      }
    }
    if (!Number.isSafeInteger(year) || year < 2001) {
      console.error("--year takes a year from 2001 on");
      Deno.exit(2);
    }
    await fetchAll(year, refresh);
  } else if (command === "docs" && rest.length === 0) {
    await writeDocs();
  } else {
    console.error("usage: additional.ts fetch [--year YEAR] [--refresh] | additional.ts docs");
    Deno.exit(2);
  }
}
