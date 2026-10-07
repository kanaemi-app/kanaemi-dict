import { appendRecord, latestForUrl, loadManifest, type Record } from "./manifest.ts";
import type { RawStore } from "./store.ts";

export const USER_AGENT = "kanaemi-dict-fetch/0.1 (+https://github.com/kanaemi-app/kanaemi-dict)";

export type FetchOptions = {
  /** Fetch again even when the manifest already has the URL. */
  refresh?: boolean;
  /** Attempts after the first one. */
  retries?: number;
  /** Wait before the first retry; doubled for each later one. */
  backoffMs?: number;
  /**
   * Limit for one attempt, response body included. Defaults to two minutes,
   * or an hour for a large download.
   */
  timeoutMs?: number;
  /**
   * Least time between two requests to the URL's host, retries included.
   * Defaults to a second.
   */
  minIntervalMs?: number;
  /**
   * Whether to keep a response of a content type from the URL it was served
   * from, after any redirects; others are not stored.
   */
  accept?: (contentType: string, servedFrom: string) => boolean;
  /** Once this fires no new download starts; a URL already stored still comes back. */
  stop?: AbortSignal;
  /** Streams the download into the store instead of holding it whole. */
  large?: boolean;
};

/** A response whose content type the fetch did not accept. */
export class NotAccepted extends Error {
  constructor(readonly url: string, readonly contentType: string) {
    super(`${contentType} is not accepted for ${url}`);
    this.name = "NotAccepted";
  }
}

/** A download not started because the stop signal had fired. */
export class Stopped extends Error {
  constructor(readonly url: string) {
    super(`stopped before ${url}`);
    this.name = "Stopped";
  }
}

/** A response with an error status. Client errors other than 408 and 429 are not retried. */
export class HttpError extends Error {
  constructor(readonly url: string, readonly status: number) {
    super(`HTTP ${status} for ${url}`);
    this.name = "HttpError";
  }

  get retriable(): boolean {
    return this.status >= 500 || this.status === 408 || this.status === 429;
  }
}

/** The gap kept between requests to a host unless the caller sets one. */
const POLITE_INTERVAL_MS = 1000;
/** How long a download of hundreds of megabytes may take. */
const LARGE_TIMEOUT_MS = 60 * 60 * 1000;
/** When the latest request to each host started, or is due to start. */
const requestSlots = new Map<string, number>();

/**
 * Fetches `url` into the store and records it in the manifest under
 * `sourceId`. A URL whose content is already stored is not fetched again
 * unless `refresh` is set; under another source ID it is only recorded.
 */
export async function fetchToStore(
  store: RawStore,
  manifestPath: string,
  sourceId: string,
  url: string,
  options: FetchOptions = {},
): Promise<Record> {
  const {
    refresh = false,
    retries = 3,
    backoffMs = 1000,
    timeoutMs = options.large ? LARGE_TIMEOUT_MS : 120_000,
  } = options;
  const records = await manifestOf(manifestPath);
  const known = latestForUrl(records, url);
  let record: Record;
  if (!refresh && known && await store.contains(known.sha256)) {
    const current = records.findLast((r) => r.source_id === sourceId);
    if (known.source_id === sourceId && current === known) return known;
    record = { ...known, source_id: sourceId };
  } else {
    if (options.stop?.aborted) throw new Stopped(url);
    const minIntervalMs = options.minIntervalMs ?? POLITE_INTERVAL_MS;
    const get = {
      retries,
      backoffMs,
      timeoutMs,
      minIntervalMs,
      accept: options.accept,
      stop: options.stop,
    };
    const stored = options.large
      ? await getWithRetries(url, get, (body) => store.putStream(body))
      : await getWithRetries(url, get, async (body) => {
        const data = new Uint8Array(await new Response(body).arrayBuffer());
        return { sha256: await store.put(data), bytes: data.length };
      });
    record = {
      source_id: sourceId,
      url,
      sha256: stored.sha256,
      bytes: stored.bytes,
      retrieved_at: timestamp(new Date()),
    };
  }
  await appendRecord(manifestPath, record);
  await remember(manifestPath, record);
  return record;
}

/**
 * Manifests read in this process, with the size they had. A manifest is read
 * again only when its size no longer matches, as when another writer appended.
 */
const manifests = new Map<string, { size: number; records: Record[] }>();

async function manifestOf(path: string): Promise<Record[]> {
  const size = await sizeOf(path);
  const known = manifests.get(path);
  if (known && known.size === size) return known.records;
  const records = await loadManifest(path);
  manifests.set(path, { size, records });
  return records;
}

/** Adds a record this process appended, unless another writer appended too. */
async function remember(path: string, record: Record): Promise<void> {
  const known = manifests.get(path);
  const size = await sizeOf(path);
  const line = new TextEncoder().encode(JSON.stringify(record) + "\n").length;
  if (known && known.size + line === size) {
    known.records.push(record);
    known.size = size;
  } else {
    manifests.delete(path);
  }
}

async function sizeOf(path: string): Promise<number> {
  try {
    return (await Deno.stat(path)).size;
  } catch (e) {
    if (e instanceof Deno.errors.NotFound) return 0;
    throw e;
  }
}

async function getWithRetries<T>(
  url: string,
  { retries, backoffMs, timeoutMs, minIntervalMs, accept, stop }: {
    retries: number;
    backoffMs: number;
    timeoutMs: number;
    minIntervalMs: number;
    accept?: (contentType: string, servedFrom: string) => boolean;
    stop?: AbortSignal;
  },
  consume: (body: ReadableStream<Uint8Array>) => Promise<T>,
): Promise<T> {
  const host = new URL(url).host;
  for (let attempt = 0;; attempt++) {
    // The slot is taken before waiting for it, so requests to one host that
    // wait at the same time still go out one interval apart.
    const now = performance.now();
    const slot = Math.max(now, (requestSlots.get(host) ?? -Infinity) + minIntervalMs);
    requestSlots.set(host, slot);
    if (slot > now) await sleep(slot - now);
    if (stop?.aborted) throw new Stopped(url);
    try {
      const signal = AbortSignal.timeout(timeoutMs);
      const res = await fetch(url, { headers: { "user-agent": USER_AGENT }, signal });
      if (!res.ok) {
        await res.body?.cancel();
        throw new HttpError(url, res.status);
      }
      const contentType = res.headers.get("content-type") ?? "";
      if (accept && !accept(contentType, res.url || url)) {
        await res.body?.cancel();
        throw new NotAccepted(url, contentType);
      }
      if (!res.body) throw new Error(`no body for ${url}`);
      return await consume(res.body);
    } catch (e) {
      if (e instanceof NotAccepted || (e instanceof HttpError && !e.retriable)) throw e;
      if (stop?.aborted) throw new Stopped(url);
      if (attempt >= retries) throw e;
      await sleep(backoffMs * 2 ** attempt);
    }
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Local time with its offset, as `2026-10-02T11:07:41+0900`. */
function timestamp(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  const offset = -date.getTimezoneOffset();
  const sign = offset >= 0 ? "+" : "-";
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${
    pad(date.getHours())
  }:${pad(date.getMinutes())}:${pad(date.getSeconds())}${sign}${
    pad(Math.floor(Math.abs(offset) / 60))
  }${pad(Math.abs(offset) % 60)}`;
}
