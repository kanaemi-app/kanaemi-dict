/**
 * The sources documents are taken from. Each lives in its own module under
 * `sources/`, which says how it is fetched and how its records are read;
 * adding a source means adding a module and listing it here.
 */
import type { Document } from "./docs/document.ts";
import type { FetchOptions } from "./raw/fetch.ts";
import type { Record } from "./raw/manifest.ts";
import { source as aozora } from "./sources/aozora.ts";
import { source as fineweb } from "./sources/fineweb.ts";
import { source as law } from "./sources/law.ts";
import { source as pydocs } from "./sources/pydocs.ts";
import { source as rurema } from "./sources/rurema.ts";
import { source as wikinews } from "./sources/wikinews.ts";
import { source as wikipedia } from "./sources/wikipedia.ts";

/** What a source's fetch is given. */
export type FetchContext = {
  /**
   * Fetches `url` into the raw store as a record of `sourceId`, unless it is
   * stored already.
   */
  get(sourceId: string, url: string, options?: FetchOptions): Promise<Record>;
  read(record: Record): Promise<Uint8Array>;
};

/** What taking the documents out of one record is given. */
export type ReadContext = {
  read(record: Record): Promise<Uint8Array>;
  /** The stored file of a record, for a reader that streams it rather than holding it. */
  path(record: Record): string;
  /** The build record of `sourceId`, as of a companion read along with this record. */
  record(sourceId: string): Record | undefined;
};

/**
 * A source: the records it fetches and the documents in them. A source ID
 * pattern is an exact source ID, or a prefix ending in `:` that stands for
 * every source ID under it.
 */
export type Source = {
  /** The name `scripts/fetch.ts` chooses the source by. */
  name: string;
  /** Patterns of the source IDs whose records hold documents. */
  ids: string[];
  /** Patterns of the source IDs whose records are read only along with the others. */
  companions?: string[];
  fetch(ctx: FetchContext): Promise<void>;
  /** The documents of one record whose source ID `ids` matches. */
  documents(record: Record, ctx: ReadContext): Promise<Document[]>;
};

export const SOURCES: Source[] = [aozora, law, wikinews, wikipedia, pydocs, rurema, fineweb];

export function matchesId(pattern: string, sourceId: string): boolean {
  return pattern.endsWith(":") ? sourceId.startsWith(pattern) : sourceId === pattern;
}

export type Owner = { source: Source; companion: boolean };

/** The source a record of `sourceId` belongs to, if any. */
export function ownerOf(sourceId: string, sources: Source[] = SOURCES): Owner | undefined {
  for (const source of sources) {
    if (source.ids.some((p) => matchesId(p, sourceId))) return { source, companion: false };
    if (source.companions?.some((p) => matchesId(p, sourceId))) return { source, companion: true };
  }
  return undefined;
}
