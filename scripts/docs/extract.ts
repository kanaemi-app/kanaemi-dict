import { dirname } from "@std/path";
import { forBuild, type Record } from "../raw/manifest.ts";
import { isSha256, type RawStore } from "../raw/store.ts";
import { ownerOf, type Source, SOURCES } from "../sources.ts";
import type { Document } from "./document.ts";

export class SourceError extends Error {
  constructor(readonly sourceId: string, readonly url: string, options: ErrorOptions) {
    super(`${sourceId} (${url}): ${options.cause}`, options);
    this.name = "SourceError";
  }
}

/** A record no source owns, as of a source renamed or removed since it was fetched. */
export class UnknownSourceError extends Error {
  constructor(readonly sourceId: string) {
    super(`no source reads the records of ${sourceId}`);
    this.name = "UnknownSourceError";
  }
}

export class DuplicateIdError extends Error {
  constructor(readonly docId: string) {
    super(`two documents share the ID ${docId}`);
    this.name = "DuplicateIdError";
  }
}

/**
 * Every document the build reads, sorted by doc ID, from the build records,
 * each read by the source that owns it.
 */
export async function extractAll(
  store: RawStore,
  records: Record[],
  sources: Source[] = SOURCES,
): Promise<Document[]> {
  const build = forBuild(records);
  const bySource = new Map(build.map((r) => [r.source_id, r]));
  const sha256 = (r: Record) => {
    if (!isSha256(r.sha256)) throw new Error(`malformed sha256 in the record of ${r.source_id}`);
    return r.sha256;
  };
  const ctx = {
    read: (r: Record) => store.read(sha256(r)),
    path: (r: Record) => store.path(sha256(r)),
    record: (sourceId: string) => bySource.get(sourceId),
  };
  const docs: Document[] = [];
  for (const record of build) {
    const owner = ownerOf(record.source_id, sources);
    if (!owner) throw new UnknownSourceError(record.source_id);
    if (owner.companion) continue;
    try {
      docs.push(...await owner.source.documents(record, ctx));
    } catch (cause) {
      throw new SourceError(record.source_id, record.url, { cause });
    }
  }
  docs.sort((a, b) => compareUtf8(a.doc_id, b.doc_id));
  for (let i = 1; i < docs.length; i++) {
    if (docs[i - 1].doc_id === docs[i].doc_id) throw new DuplicateIdError(docs[i].doc_id);
  }
  return docs;
}

const encoder = new TextEncoder();

function compareUtf8(a: string, b: string): number {
  const x = encoder.encode(a);
  const y = encoder.encode(b);
  for (let i = 0; i < Math.min(x.length, y.length); i++) {
    if (x[i] !== y[i]) return x[i] - y[i];
  }
  return x.length - y.length;
}

/** Writes `docs` as JSON Lines, one document per line. */
export async function writeJsonl(path: string, docs: Document[]): Promise<void> {
  await Deno.mkdir(dirname(path), { recursive: true });
  using file = await Deno.open(path, { write: true, create: true, truncate: true });
  const writer = file.writable.getWriter();
  for (const doc of docs) await writer.write(encoder.encode(`${JSON.stringify(doc)}\n`));
  await writer.close();
}
