/**
 * Fetches the sources into the raw store, as docs/spec/sources.md sets.
 *
 *     deno run -A scripts/fetch.ts [aozora|law|wikinews|wikipedia|pydocs|rurema|fineweb ...] [--dir DIR] [--minutes N] [--refresh]
 *
 * DIR defaults to build/raw. Without a source name every source is fetched. A
 * URL already stored is not fetched again, so a run picks up where the last
 * one stopped; with --refresh it is fetched again. With --minutes no download
 * starts after N minutes, and the run exits with 75 when some source is left
 * unfinished.
 */
import { join } from "@std/path";
import { type FetchOptions, fetchToStore, Stopped } from "./raw/fetch.ts";
import { RawStore } from "./raw/store.ts";
import { type FetchContext, SOURCES } from "./sources.ts";

/** Exit status of a run that stopped before every source was done. */
const UNFINISHED = 75;

/** The longest delay a timer keeps without wrapping around. */
const MAX_TIMER_MS = 2 ** 31 - 1;

/**
 * Fetches into the raw store, starting no download once the run's time is up,
 * and reports each record's size.
 */
function fetchContext(
  store: RawStore,
  manifest: string,
  stop: AbortSignal | undefined,
  refresh: boolean,
): FetchContext {
  return {
    async get(sourceId: string, url: string, options: FetchOptions = {}) {
      const record = await fetchToStore(store, manifest, sourceId, url, {
        ...options,
        stop,
        refresh,
      });
      console.log(`${record.source_id}\t${record.bytes}`);
      return record;
    },
    read: (record) => store.read(record.sha256),
  };
}

if (import.meta.main) {
  let dir = "build/raw";
  let minutes: number | undefined;
  let refresh = false;
  const names: string[] = [];
  for (let i = 0; i < Deno.args.length; i++) {
    if (Deno.args[i] === "--dir") {
      const value = Deno.args[++i];
      if (!value) {
        console.error("--dir takes a directory");
        Deno.exit(2);
      }
      dir = value;
    } else if (Deno.args[i] === "--minutes") minutes = Number(Deno.args[++i]);
    else if (Deno.args[i] === "--refresh") refresh = true;
    else names.push(Deno.args[i]);
  }
  // A timer longer than 2^31 - 1 ms wraps around and fires at once, so the
  // run would stop before its first download.
  if (minutes !== undefined && !(minutes >= 0 && minutes * 60_000 <= MAX_TIMER_MS)) {
    console.error(
      `--minutes takes a number of minutes up to ${Math.floor(MAX_TIMER_MS / 60_000)}`,
    );
    Deno.exit(2);
  }
  const known = SOURCES.map((s) => s.name);
  const unknown = names.filter((n) => !known.includes(n));
  if (unknown.length > 0) {
    console.error(`unknown sources: ${unknown.join(", ")} (choose from ${known.join(", ")})`);
    Deno.exit(2);
  }
  const stop = minutes === undefined ? undefined : AbortSignal.timeout(minutes * 60_000);
  const ctx = fetchContext(new RawStore(dir), join(dir, "manifest.jsonl"), stop, refresh);
  const chosen = names.length > 0 ? SOURCES.filter((s) => names.includes(s.name)) : SOURCES;
  const unfinished: string[] = [];
  // The sources run side by side; requests to a host two sources share are
  // still spaced, as the fetch spaces requests per host. A source that fails
  // lets the others finish their downloads before the failure is thrown,
  // rather than ending the process while a large download is half done.
  const results = await Promise.allSettled(chosen.map(async (source) => {
    try {
      await source.fetch(ctx);
    } catch (e) {
      if (!(e instanceof Stopped)) throw e;
      unfinished.push(source.name);
    }
  }));
  const failed = results.flatMap((r) => r.status === "rejected" ? [r.reason] : []);
  if (failed.length === 1) throw failed[0];
  if (failed.length > 1) throw new AggregateError(failed, "some sources failed");
  if (unfinished.length > 0) {
    console.log(`unfinished: ${unfinished.join(", ")}`);
    Deno.exit(UNFINISHED);
  }
}
