/**
 * Hatena Bookmark's hot entries: a day's most bookmarked pages, the year's
 * Web text a year's new words are counted in.
 */
import { type Document, documentOf } from "../docs/document.ts";
import { parseDocument } from "../docs/html.ts";
import { finishLines } from "../docs/text.ts";

/** A day's hot entries, `YYYYMMDD` appended. */
export const HATENA_HOTENTRY = "https://b.hatena.ne.jp/hotentry/all/";
/** The Crawl-delay Hatena Bookmark's robots.txt asks for. */
export const HATENA_INTERVAL_MS = 5000;
/** The source ID of a day's page, `YYYYMMDD` appended. */
export const HATENA_SOURCE = "hatena-hotentry:";

const JST_OFFSET_MS = 9 * 60 * 60 * 1000;
const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * The titles and summaries of a day's hot entries, from the day's page, as
 * one document of their Japanese lines. The page also lists recent articles
 * of the day it was fetched; those are left out.
 */
export function hatenaHotentry(sourceId: string, html: Uint8Array): Document[] {
  const day = sourceId.slice(HATENA_SOURCE.length);
  const page = parseDocument(new TextDecoder().decode(html));
  const lines = [
    ...page.querySelectorAll(
      ".js-hotentries .entrylist-contents-title a, .js-hotentries .entrylist-contents-description",
    ),
  ].map((element) => element.textContent ?? "");
  const doc = documentOf(`hatena:${day}`, sourceId, finishLines(lines, true));
  return doc ? [doc] : [];
}

/** The days of `year` before today in Japan, as `YYYYMMDD`. */
export function daysOf(year: number, now: Date): string[] {
  const today = new Date(now.getTime() + JST_OFFSET_MS);
  const end = Math.min(
    Date.UTC(year + 1, 0, 1),
    Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), today.getUTCDate()),
  );
  const days = [];
  for (let day = Date.UTC(year, 0, 1); day < end; day += DAY_MS) {
    days.push(new Date(day).toISOString().slice(0, 10).replaceAll("-", ""));
  }
  return days;
}

/** The year in Japan at `now`. */
export function yearInJapan(now: Date): number {
  return new Date(now.getTime() + JST_OFFSET_MS).getUTCFullYear();
}
