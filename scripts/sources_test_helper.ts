import type { FetchOptions } from "./raw/fetch.ts";
import type { Record } from "./raw/manifest.ts";
import type { FetchContext } from "./sources.ts";

/** A fetch context that records what is fetched, with which options, and stores nothing; for tests. */
export function recordingFetch(): { ctx: FetchContext; fetched: [string, string, FetchOptions][] } {
  const fetched: [string, string, FetchOptions][] = [];
  const ctx: FetchContext = {
    get: (sourceId, url, options = {}) => {
      fetched.push([sourceId, url, options]);
      const record: Record = { source_id: sourceId, url, sha256: "", bytes: 0, retrieved_at: "" };
      return Promise.resolve(record);
    },
    read: () => Promise.resolve(new Uint8Array()),
  };
  return { ctx, fetched };
}
