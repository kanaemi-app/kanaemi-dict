import { type Document, documentOf } from "../docs/document.ts";
import { bzip2Text, dumpPages } from "../docs/mediawiki.ts";
import { finishLines } from "../docs/text.ts";
import { stripWikitext } from "../docs/wikitext.ts";
import type { Source } from "../sources.ts";

const ID = "wikinews-ja";
/** The latest dump of Japanese Wikinews. */
const SOURCE_URL =
  "https://dumps.wikimedia.org/jawikinews/latest/jawikinews-latest-pages-articles.xml.bz2";

export const source: Source = {
  name: "wikinews",
  ids: [ID],
  async fetch(ctx) {
    await ctx.get(ID, SOURCE_URL);
  },
  async documents(record, ctx) {
    return await wikinews(record.source_id, ctx.path(record));
  },
};

/**
 * One document per article of the Japanese Wikinews dump at `path`
 * (bzip2): the main namespace, redirects left out.
 */
export async function wikinews(sourceId: string, path: string): Promise<Document[]> {
  const docs: Document[] = [];
  const articles = dumpPages(bzip2Text(path), (h) => h.ns === "0" && !h.redirect);
  for await (const page of articles) {
    const lines = finishLines(stripWikitext(page.text).split("\n"), true);
    const doc = documentOf(`wikinews:${page.id}`, sourceId, lines);
    if (doc) docs.push(doc);
  }
  return docs;
}
