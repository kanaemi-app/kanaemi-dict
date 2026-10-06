/**
 * The files the build takes from SudachiDict: the system dictionary of
 * SudachiDict full, which the analyzer reads, the system dictionary of
 * SudachiDict small, which checks the analyzer's readings, and the lexicon
 * file of SudachiDict small (UniDic), which the readings are checked against
 * and whose words the base dictionary takes in.
 * SudachiDict is a tool of the build, not a source of documents, so its
 * records live in a raw store of their own.
 */
import { basename, dirname, join } from "@std/path";
import type { FetchOptions } from "../raw/fetch.ts";
import { forBuild, loadManifest, type Record } from "../raw/manifest.ts";
import { RawStore } from "../raw/store.ts";
import { zipEntries } from "../zip.ts";

export type SudachiFile = {
  sourceId: string;
  /** A pinned release, so that the same build reads the same dictionary. */
  url: string;
  /** The file's name in the zip, and the name it is written out under. */
  name: string;
};

export const SUDACHI_FILES: SudachiFile[] = [
  {
    sourceId: "sudachidict-full",
    url:
      "https://d2ej7fkh96fzlu.cloudfront.net/sudachidict/v1/sudachi-dictionary-20260723-full.zip",
    name: "system_full.dic",
  },
  {
    sourceId: "sudachidict-small",
    url:
      "https://d2ej7fkh96fzlu.cloudfront.net/sudachidict/v1/sudachi-dictionary-20260723-small.zip",
    name: "system_small.dic",
  },
  {
    sourceId: "sudachidict-small-lex",
    url:
      "https://sudachi.s3.ap-northeast-1.amazonaws.com/sudachidict-raw/v1/20260723/small_lex.zip",
    name: "small_lex.csv",
  },
];

/** Fetches `url` into the raw store as a record of `sourceId`, unless it is stored already. */
export type Get = (sourceId: string, url: string, options?: FetchOptions) => Promise<Record>;

/** Fetches every SudachiDict file's zip, each a download of hundreds of megabytes. */
export async function fetchSudachi(get: Get, files: SudachiFile[] = SUDACHI_FILES): Promise<void> {
  for (const file of files) await get(file.sourceId, file.url, { large: true });
}

/**
 * Writes the file of `file.name` in the zip of the latest record of
 * `file.sourceId` in the raw store under `rawDir` to `outDir`, and returns
 * where it went. The file is written beside its place and renamed into it, so
 * a failure never leaves part of a file for the build to read.
 */
export async function extractSudachiFile(
  rawDir: string,
  file: SudachiFile,
  outDir: string,
): Promise<string> {
  const records = await loadManifest(join(rawDir, "manifest.jsonl"));
  const record = forBuild(records).find((r) => r.source_id === file.sourceId);
  if (!record) throw new Error(`no ${file.sourceId} record in ${rawDir}`);
  const zip = await new RawStore(rawDir).read(record.sha256);
  for await (const entry of zipEntries(zip)) {
    if (basename(entry.name) !== file.name) continue;
    const out = join(outDir, file.name);
    const partial = `${out}.${Deno.pid}.tmp`;
    await Deno.mkdir(dirname(out), { recursive: true });
    try {
      await Deno.writeFile(partial, await entry.bytes());
      await Deno.rename(partial, out);
    } catch (e) {
      await Deno.remove(partial).catch(() => {});
      throw e;
    }
    return out;
  }
  throw new Error(`no ${file.name} in ${record.url}`);
}
