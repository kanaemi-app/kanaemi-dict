/**
 * `manifest.jsonl`: the append-only log of fetches, one JSON record per line.
 */
import { dirname } from "@std/path";

export type Record = {
  source_id: string;
  url: string;
  sha256: string;
  bytes: number;
  retrieved_at: string;
};

export class ManifestError extends Error {
  /** Counts from 1. */
  constructor(readonly line: number, message: string) {
    super(`line ${line}: ${message}`);
    this.name = "ManifestError";
  }
}

/** Reads every record. A missing file means nothing has been fetched yet. */
export async function loadManifest(path: string): Promise<Record[]> {
  let file: Deno.FsFile;
  try {
    file = await Deno.open(path, { read: true });
  } catch (e) {
    if (e instanceof Deno.errors.NotFound) return [];
    throw e;
  }
  let text: string;
  try {
    // Shared with other readers, excluded by appendRecord, so a line being
    // appended is never read half-written.
    await file.lock(false);
    text = await new Response(file.readable).text();
  } finally {
    try {
      file.close();
    } catch {
      // Reading to the end of `readable` already closed it.
    }
  }
  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines.map((line, i) => parseRecord(line, i + 1));
}

function parseRecord(line: string, lineNo: number): Record {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch (e) {
    throw new ManifestError(lineNo, String(e));
  }
  const r = value as Partial<Record>;
  const fields = ["source_id", "url", "sha256", "retrieved_at"] as const;
  if (typeof r !== "object" || r === null) throw new ManifestError(lineNo, "not an object");
  for (const field of fields) {
    if (typeof r[field] !== "string") throw new ManifestError(lineNo, `missing ${field}`);
  }
  if (typeof r.bytes !== "number") throw new ManifestError(lineNo, "missing bytes");
  return r as Record;
}

/** Adds `record` as a new last line. Existing lines are never rewritten. */
export async function appendRecord(path: string, record: Record): Promise<void> {
  await Deno.mkdir(dirname(path), { recursive: true });
  using file = await Deno.open(path, { read: true, append: true, create: true });
  // Held until the file closes, so two appends never both see a missing
  // trailing newline and each add one, leaving a blank line.
  await file.lock(true);
  const prefix = (await endsWithNewline(file)) ? "" : "\n";
  let bytes = new TextEncoder().encode(`${prefix}${JSON.stringify(record)}\n`);
  // A write may take only part of the buffer; a half-written line would make
  // every later load fail.
  while (bytes.length > 0) bytes = bytes.subarray(await file.write(bytes));
}

async function endsWithNewline(file: Deno.FsFile): Promise<boolean> {
  const size = (await file.stat()).size;
  if (size === 0) return true;
  await file.seek(-1, Deno.SeekMode.End);
  const last = new Uint8Array(1);
  await file.read(last);
  return last[0] === 0x0a;
}

/** The record that decides whether `url` needs fetching again. */
export function latestForUrl(records: Record[], url: string): Record | undefined {
  return records.findLast((r) => r.url === url);
}

/**
 * The records the build reads: the last record of each source ID, ordered by
 * source ID. A source ID names one URL at a time, so a source whose URL
 * changed is read from its new URL.
 */
export function forBuild(records: Record[]): Record[] {
  const latest = new Map<string, Record>();
  for (const r of records) latest.set(r.source_id, r);
  return [...latest.values()].sort((a, b) => compare(a.source_id, b.source_id));
}

function compare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}
