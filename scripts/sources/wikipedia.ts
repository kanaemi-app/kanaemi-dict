import { type Document, documentOf } from "../docs/document.ts";
import { bzip2Text, dumpPages, isDisambiguation } from "../docs/mediawiki.ts";
import { finishLines } from "../docs/text.ts";
import { stripWikitext } from "../docs/wikitext.ts";
import { HashBudget, keyHash } from "../select.ts";
import type { Source } from "../sources.ts";

const ID = "wikipedia-ja";
/** The latest dump of Japanese Wikipedia's articles. */
const SOURCE_URL = "https://dumps.wikimedia.org/jawiki/latest/jawiki-latest-pages-articles.xml.bz2";
/** Characters taken from the articles' text. */
const BUDGET = 20_000_000;

export const source: Source = {
  name: "wikipedia",
  ids: [ID],
  async fetch(ctx) {
    await ctx.get(ID, SOURCE_URL, { large: true });
  },
  async documents(record, ctx) {
    return await wikipedia(record.source_id, bzip2Text(ctx.path(record)), BUDGET);
  },
};

/**
 * Documents of the articles of a Wikipedia dump's XML: the main namespace,
 * neither redirects nor disambiguation pages, of their Japanese lines, taken
 * in the SHA-256 order of their doc IDs until their characters reach
 * `budget`. An article past those already taken is never stripped.
 */
export async function wikipedia(
  sourceId: string,
  xml: AsyncIterable<string>,
  budget: number,
): Promise<Document[]> {
  const taken = new HashBudget<Document>(budget);
  const wanted = (h: { ns: string; id: string; redirect: boolean }) =>
    h.ns === "0" && !h.redirect && taken.wants(keyHash(docIdOf(h.id)));
  for await (const page of dumpPages(xml, wanted)) {
    if (isDisambiguation(page.text)) continue;
    const docId = docIdOf(page.id);
    const doc = documentOf(
      docId,
      sourceId,
      finishLines(stripWikitext(page.text).split("\n"), true),
    );
    if (doc) taken.offer(keyHash(docId), [...doc.text].length, doc);
  }
  return taken.items();
}

function docIdOf(pageId: string): string {
  return `wikipedia:${pageId}`;
}
