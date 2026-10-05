import { SaxesParser } from "saxes";
import { inHashOrder, takeUntilBudget } from "../select.ts";
import { zipEntries, type ZipEntry } from "../zip.ts";
import { type Document, documentOf } from "../docs/document.ts";
import { finishLines } from "../docs/text.ts";
import type { Source } from "../sources.ts";

const ID = "egov-all-xml";
/** e-Gov's zip of every law in XML, hundreds of megabytes. */
const SOURCE_URL = "https://laws.e-gov.go.jp/bulkdownload?file_section=1&only_xml_flag=true";

/** Characters of the laws taken out. */
const BUDGET = 2_500_000;

export const source: Source = {
  name: "law",
  ids: [ID],
  async fetch(ctx) {
    await ctx.get(ID, SOURCE_URL, { large: true });
  },
  async documents(record, ctx) {
    return await laws(record.source_id, await ctx.read(record), BUDGET);
  },
};

const LINE_BREAK = /\s*\n\s*/g;

/**
 * Documents of laws in e-Gov's all-laws XML zip, one per law, taken in the
 * order of the SHA-256 of their doc IDs until their characters reach `budget`.
 *
 * The zip holds a law's revisions yet to take effect beside the one in force,
 * each in a folder `<law ID>_<enforcement date>_<revision ID>`; only the
 * earliest enforced, the one in force, is read.
 */
export async function laws(
  sourceId: string,
  zip: Uint8Array,
  budget = Infinity,
): Promise<Document[]> {
  const byLaw = new Map<string, { folder: string; docId: string; entry: ZipEntry }>();
  for await (const entry of zipEntries(zip)) {
    if (!entry.name.endsWith(".xml")) continue;
    const folder = entry.name.split("/")[0];
    const lawId = folder.split("_")[0];
    const known = byLaw.get(lawId);
    if (!known || folder < known.folder) {
      byLaw.set(lawId, { folder, docId: `law:${folder}`, entry });
    }
  }
  const files = [...byLaw.values()];
  const docs: Document[] = [];
  await takeUntilBudget(await inHashOrder(files, (f) => f.docId), budget, async (file) => {
    const xml = new TextDecoder().decode(await file.entry.bytes());
    const doc = documentOf(file.docId, sourceId, finishLines(sentences(xml), false));
    if (!doc) return undefined;
    docs.push(doc);
    return [...doc.text].length;
  });
  return docs;
}

const PROVISIONS = new Set(["MainProvision", "SupplProvision"]);

/**
 * The text of every `Sentence` in the main and supplementary provisions,
 * ruby readings left out. A nested `Sentence` stays inside the outermost one,
 * which alone becomes a line.
 */
function sentences(xml: string): string[] {
  const parser = new SaxesParser();
  let sentence = "";
  let sentenceDepth = 0;
  const out: string[] = [];
  let provisionDepth = 0;
  let rubyReadingDepth = 0;
  const append = (text: string) => {
    if (sentenceDepth > 0 && rubyReadingDepth === 0) sentence += text;
  };
  parser.on("opentag", (tag) => {
    if (PROVISIONS.has(tag.name)) provisionDepth++;
    else if (tag.name === "Rt") rubyReadingDepth++;
    else if (tag.name === "Sentence" && provisionDepth > 0) {
      if (sentenceDepth === 0) sentence = "";
      sentenceDepth++;
    }
  });
  parser.on("closetag", (tag) => {
    if (PROVISIONS.has(tag.name)) provisionDepth--;
    else if (tag.name === "Rt") rubyReadingDepth--;
    else if (tag.name === "Sentence" && sentenceDepth > 0) {
      sentenceDepth--;
      if (sentenceDepth === 0) out.push(sentence.replace(LINE_BREAK, ""));
    }
  });
  parser.on("text", append);
  parser.on("cdata", append);
  parser.write(xml).close();
  return out;
}
