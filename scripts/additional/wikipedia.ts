/**
 * What the additional dictionaries take out of the Wikipedia dump: the
 * articles of a field's categories with their titles, and the titles of the
 * articles of a year.
 */
import { type Document, documentOf } from "../docs/document.ts";
import { categoriesOf, dumpPages, isDisambiguation } from "../docs/mediawiki.ts";
import { finishLines, normalize } from "../docs/text.ts";
import { stripWikitext } from "../docs/wikitext.ts";
import { HashBudget, keyHash } from "../select.ts";
import type { Dictionary } from "./config.ts";

/** An article's title with the reading its lead gives. */
export type Title = { doc_id: string; reading: string; surface: string };

export type Articles = { docs: Document[]; titles: Title[] };

/**
 * The dictionaries of `dictionaries` whose patterns one of `categories`
 * matches and whose first page ID, if any, `id` reaches.
 */
export function fieldsOf(
  categories: string[],
  id: number,
  dictionaries: Dictionary[],
): string[] {
  return dictionaries
    .filter(({ patterns, since }) =>
      (since === undefined || id >= since) &&
      categories.some((c) => patterns?.some((p) => p.test(c)))
    )
    .map((d) => d.name);
}

/**
 * The articles of a dump's XML each dictionary with patterns takes:
 * disambiguation pages left out. A field takes the documents of its articles
 * in the SHA-256 order of their doc IDs until their characters reach
 * `budget`, with the titles of those articles; a year takes only the titles
 * of its articles.
 */
export async function articlesOf(
  xml: AsyncIterable<string>,
  dictionaries: Dictionary[],
  sourceId: string,
  budget: number,
): Promise<Map<string, Articles>> {
  const wanted = dictionaries.filter((d) => d.patterns !== undefined);
  const fields = new Map(
    wanted.filter((d) => d.since === undefined)
      .map((d) => [d.name, new HashBudget<{ doc: Document; title?: Title }>(budget)]),
  );
  const years = new Map(
    wanted.filter((d) => d.since !== undefined).map((d) => [d.name, [] as Title[]]),
  );
  const articles = dumpPages(xml, (h) => h.ns === "0" && !h.redirect);
  for await (const page of articles) {
    if (isDisambiguation(page.text)) continue;
    const matched = fieldsOf(categoriesOf(page.text), Number(page.id), wanted);
    if (matched.length === 0) continue;
    const docId = `wikipedia:${page.id}`;
    const hash = keyHash(docId);
    const named = titleReading(page.title, page.text);
    const title = named && { doc_id: docId, ...named };
    let doc: Document | undefined | null = null;
    for (const name of matched) {
      if (years.has(name)) {
        if (title) years.get(name)!.push(title);
        continue;
      }
      const taken = fields.get(name)!;
      if (!taken.wants(hash)) continue;
      if (doc === null) {
        doc = documentOf(docId, sourceId, finishLines(stripWikitext(page.text).split("\n"), true));
      }
      if (doc) taken.offer(hash, [...doc.text].length, { doc, title });
    }
  }
  const found = new Map<string, Articles>();
  for (const [name, taken] of fields) {
    const items = taken.items();
    found.set(name, {
      docs: items.map((i) => i.doc),
      titles: items.flatMap((i) => i.title ? [i.title] : []),
    });
  }
  for (const [name, titles] of years) found.set(name, { docs: [], titles });
  return found;
}

const COMMENT = /<!--[\s\S]*?(?:-->|$)/g;
const HIRAGANA = /^[\p{Script=Hiragana}ー]+$/u;
const KATAKANA = /^[\p{Script=Katakana}ー]+$/u;

/**
 * The article's title, without its parenthesized qualifier, with the
 * reading the lead gives right after the bold title, when that reading is
 * hiragana. A title in katakana reads as itself. Both come normalized like
 * the text they are counted in.
 */
export function titleReading(
  title: string,
  wikitext: string,
): { surface: string; reading: string } | undefined {
  const surface = title.replace(/\s*[（(][^）)]*[）)]$/, "");
  const normalized = normalize(surface);
  if (normalized === "") return undefined;
  if (KATAKANA.test(normalized)) {
    return { surface: normalized, reading: toHiragana(normalized) };
  }
  const bold = `'''${surface}'''`;
  const text = wikitext.replace(COMMENT, "");
  const at = text.indexOf(bold);
  if (at < 0) return undefined;
  const lead = text.slice(at + bold.length).match(/^\s*[（(]([^、,，）);；]+)/);
  const reading = lead?.[1].replace(/\s+/g, "");
  return reading && HIRAGANA.test(reading)
    ? { surface: normalized, reading: normalize(reading) }
    : undefined;
}

function toHiragana(katakana: string): string {
  return katakana.replace(/[ァ-ヶ]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0x60));
}
