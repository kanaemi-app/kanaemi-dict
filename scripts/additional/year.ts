/**
 * Setting up a year's dictionary of new words: its candidates are the
 * articles created from the year on, the first of which Wikipedia's creation
 * log names once, so that building needs no network.
 */
import { join } from "@std/path";
import { USER_AGENT } from "../raw/fetch.ts";

const API = "https://ja.wikipedia.org/w/api.php";

/** The query for the first article created on or after the year's first moment. */
export function firstPageQuery(year: number): string {
  const params = new URLSearchParams({
    action: "query",
    list: "logevents",
    letype: "create",
    lenamespace: "0",
    lestart: `${year}-01-01T00:00:00Z`,
    ledir: "newer",
    lelimit: "1",
    format: "json",
  });
  return `${API}?${params}`;
}

export type LogAnswer = {
  query?: { logevents?: { logpage?: number; pageid?: number; timestamp?: string }[] };
};

/**
 * The page ID the year's first created article had when created, from the
 * answer to [`firstPageQuery`]; the page may have been deleted or moved
 * since. Articles are created every day, so a first creation after January 1
 * means the log does not reach back to the year's start, and there is none.
 */
export function firstPageOfYear(year: number, answer: LogAnswer): number {
  const first = answer.query?.logevents?.[0];
  const page = first?.logpage || first?.pageid;
  const seen = first?.timestamp ? ` (first: ${first.timestamp})` : "";
  if (!page || !first?.timestamp?.startsWith(`${year}-`)) {
    throw new NoArticleYet(year, seen);
  }
  if (!first.timestamp.startsWith(`${year}-01-01`)) {
    throw new Error(`the creation log does not reach back to ${year}-01-01${seen}`);
  }
  return page;
}

export class NoArticleYet extends Error {
  constructor(readonly year: number, seen: string) {
    super(`no article created in ${year} yet${seen}`);
    this.name = "NoArticleYet";
  }
}

/** The files of a year's dictionary: every category, from the year's first page on. */
export function yearFiles(year: number, since: number): { [name: string]: string } {
  return {
    "label.txt": `${year}年の新語\n`,
    "wikipedia.txt": ".\n",
    "wikipedia-since.txt": `${since}\n`,
  };
}

/**
 * Sets up `<dir>/<year>/` unless it is there, asking the creation log for
 * the year's first page. Returns whether it was set up now.
 */
export async function setUpYear(dir: string, year: number): Promise<boolean> {
  const target = join(dir, String(year));
  try {
    await Deno.stat(target);
    return false;
  } catch (e) {
    if (!(e instanceof Deno.errors.NotFound)) throw e;
  }
  const res = await fetch(firstPageQuery(year), { headers: { "user-agent": USER_AGENT } });
  if (!res.ok) throw new Error(`${API}: HTTP ${res.status}`);
  const since = firstPageOfYear(year, await res.json());
  // Written aside and moved into place whole, so an interrupted setup is not
  // taken for a finished one.
  const aside = join(dir, `.${year}.partial`);
  await Deno.remove(aside, { recursive: true }).catch((e) => {
    if (!(e instanceof Deno.errors.NotFound)) throw e;
  });
  await Deno.mkdir(aside, { recursive: true });
  for (const [name, text] of Object.entries(yearFiles(year, since))) {
    await Deno.writeTextFile(join(aside, name), text);
  }
  await Deno.rename(aside, target);
  return true;
}
