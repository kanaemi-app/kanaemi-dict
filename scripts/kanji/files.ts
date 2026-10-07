/**
 * The files the base dictionary takes its single kanji readings from: Mozc's
 * table of single kanji and Unicode's Unihan readings, both at pinned
 * versions. They are lists of readings, not documents, so their records live
 * in a raw store of their own.
 */
import { basename, dirname, join } from "@std/path";
import type { FetchOptions } from "../raw/fetch.ts";
import { forBuild, loadManifest, type Record } from "../raw/manifest.ts";
import { RawStore } from "../raw/store.ts";
import { zipEntries } from "../zip.ts";

export type KanjiFile = {
  sourceId: string;
  /** A pinned commit or version, so that the same build reads the same list. */
  url: string;
  /** The name the file is written out under, and its name in the zip. */
  name: string;
  /** Whether the URL serves a zip holding the file rather than the file. */
  zipped: boolean;
};

export const KANJI_FILES: KanjiFile[] = [
  {
    sourceId: "mozc-single-kanji",
    url:
      "https://raw.githubusercontent.com/google/mozc/23366530244c9c85a7c0c0e0e766e04fde8e0d4d/src/data/single_kanji/single_kanji.tsv",
    name: "single_kanji.tsv",
    zipped: false,
  },
  {
    sourceId: "unicode-unihan",
    url: "https://www.unicode.org/Public/18.0.0/ucd/Unihan.zip",
    name: "Unihan_Readings.txt",
    zipped: true,
  },
];

/** Fetches `url` into the raw store as a record of `sourceId`, unless it is stored already. */
export type Get = (sourceId: string, url: string, options?: FetchOptions) => Promise<Record>;

/** Fetches every kanji file. */
export async function fetchKanji(get: Get, files: KanjiFile[] = KANJI_FILES): Promise<void> {
  for (const file of files) await get(file.sourceId, file.url);
}

/**
 * Writes the file of the latest record of `file.sourceId` in the raw store
 * under `rawDir` to `outDir`, taking it out of the zip when the record is
 * one, and returns where it went. The file is written beside its place and
 * renamed into it, so a failure never leaves part of a file for the build to
 * read.
 */
export async function extractKanjiFile(
  rawDir: string,
  file: KanjiFile,
  outDir: string,
): Promise<string> {
  const records = await loadManifest(join(rawDir, "manifest.jsonl"));
  const record = forBuild(records).find((r) => r.source_id === file.sourceId);
  if (!record) throw new Error(`no ${file.sourceId} record in ${rawDir}`);
  const stored = await new RawStore(rawDir).read(record.sha256);
  const bytes = file.zipped ? await entryOf(stored, file.name, record.url) : stored;
  const out = join(outDir, file.name);
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

async function entryOf(zip: Uint8Array, name: string, url: string): Promise<Uint8Array> {
  for await (const entry of zipEntries(zip)) {
    if (basename(entry.name) === name) return await entry.bytes();
  }
  throw new Error(`no ${name} in ${url}`);
}
