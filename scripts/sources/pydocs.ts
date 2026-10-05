import { zipEntries } from "../zip.ts";
import { type Document, documentOf } from "../docs/document.ts";
import { elementLines, parseDocument } from "../docs/html.ts";
import { finishLines } from "../docs/text.ts";
import type { Source } from "../sources.ts";

const ID = "pydocs-ja";
/** The HTML archive of the Japanese Python documentation. */
const SOURCE_URL = "https://docs.python.org/ja/3/archives/python-3.14-docs-html.zip";

export const source: Source = {
  name: "pydocs",
  ids: [ID],
  async fetch(ctx) {
    await ctx.get(ID, SOURCE_URL);
  },
  async documents(record, ctx) {
    return await pydocs(record.source_id, await ctx.read(record));
  },
};

const NOT_PROSE = ["div.highlight", "pre", "dt.sig", "a.headerlink"].join(", ");
const INDEX_PAGE = /(?:^|\/)(?:genindex(?:-[^/]*)?|py-modindex|search)\.html$/;

/**
 * One document per page of the Japanese Python documentation's HTML archive:
 * the page's main content without code examples, API signatures and heading
 * marks. Index and search pages are skipped.
 */
export async function pydocs(sourceId: string, zip: Uint8Array): Promise<Document[]> {
  const files: [string, Uint8Array][] = [];
  for await (const entry of zipEntries(zip)) {
    if (entry.name.endsWith(".html") && !INDEX_PAGE.test(entry.name)) {
      files.push([entry.name, await entry.bytes()]);
    }
  }
  files.sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
  return files.flatMap(([name, bytes]) => {
    const main = parseDocument(new TextDecoder().decode(bytes)).querySelector('[role="main"]');
    if (!main) return [];
    for (const element of main.querySelectorAll(NOT_PROSE)) element.remove();
    const path = name.slice(name.indexOf("/") + 1);
    return documentOf(
      `pydocs:${path}`,
      sourceId,
      finishLines(elementLines(main, { keepTables: true }), true),
    ) ?? [];
  });
}
