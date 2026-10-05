import { parse } from "@std/csv";
import { inHashOrder, takeUntilBudget } from "../select.ts";
import type { Source } from "../sources.ts";
import { zipEntries } from "../zip.ts";
import { type Document, documentOf } from "../docs/document.ts";
import { finishLines } from "../docs/text.ts";

const INDEX_ID = "aozora-index";
const INDEX_URL = "https://www.aozora.gr.jp/index_pages/list_person_all_extended_utf8.zip";
const TEXT_ID = "aozora-text";
/** aozorahack's archive of the text of every Aozora Bunko work, over 200 MB. */
const TEXT_URL =
  "https://codeload.github.com/aozorahack/aozorabunko_text/zip/0984f7dc18a2270bdfd2ff99bf14fd1b3ebb4f65";

/** Characters of the works taken out. */
const BUDGET = 10_000_000;

/**
 * Aozora Bunko: its index and the text archive are fetched whole, and the
 * works are chosen when the documents are taken out, read with the index.
 */
export const source: Source = {
  name: "aozora",
  ids: [TEXT_ID],
  companions: [INDEX_ID],
  async fetch(ctx) {
    await ctx.get(INDEX_ID, INDEX_URL);
    await ctx.get(TEXT_ID, TEXT_URL, { large: true });
  },
  async documents(record, ctx) {
    const index = ctx.record(INDEX_ID);
    if (!index) throw new Error(`${INDEX_ID} is not fetched`);
    return await aozora(record.source_id, await ctx.read(index), await ctx.read(record), BUDGET);
  },
};

const NOTE = /※?［＃[^］]*］/g;
// A literal 《 is written as an annotation, so every 《…》 left is ruby,
// whatever character it is attached to.
const RUBY = /《[^《》]*》/g;

/**
 * Documents of the works the Aozora Bunko index (`indexZip`) selects, read
 * out of aozorahack's bulk text archive, taken in the order of the SHA-256 of
 * their doc IDs until their characters reach `budget`. Works the archive
 * lacks are skipped.
 */
export async function aozora(
  sourceId: string,
  indexZip: Uint8Array,
  archive: Uint8Array,
  budget = Infinity,
): Promise<Document[]> {
  const candidates = await aozoraCandidates(await indexCsv(indexZip));
  const texts = new Map<string, () => Promise<Uint8Array>>();
  for await (const entry of zipEntries(archive)) {
    // The archive holds everything under one folder named after the commit.
    texts.set(entry.name.slice(entry.name.indexOf("/") + 1), entry.bytes);
  }
  const docs: Document[] = [];
  await takeUntilBudget(candidates, budget, async (work) => {
    const bytes = texts.get(archivePath(work.url));
    if (!bytes) return undefined;
    const doc = documentOf(work.docId, sourceId, bodyLines(await bytes()));
    if (!doc) return undefined;
    docs.push(doc);
    return [...doc.text].length;
  });
  return docs;
}

export type Candidate = { docId: string; workId: string; url: string };

const AOZORA_HOST = "www.aozora.gr.jp";

/**
 * Works of the Aozora Bunko index in modern kana with their text zip on
 * Aozora Bunko, once each, in doc ID hash order.
 */
export async function aozoraCandidates(csv: string): Promise<Candidate[]> {
  const rows = parse(csv.replace(/^﻿/, ""), { skipFirstRow: true });
  const works = new Map<string, Candidate>();
  for (const row of rows) {
    const workId = row["作品ID"];
    const url = row["テキストファイルURL"] ?? "";
    const wanted = row["文字遣い種別"] === "新字新仮名" && url.endsWith(".zip") &&
      hostOf(url) === AOZORA_HOST;
    if (wanted && !works.has(workId)) works.set(workId, { docId: `aozora:${workId}`, workId, url });
  }
  return await inHashOrder([...works.values()], (c) => c.docId);
}

function hostOf(url: string): string | undefined {
  try {
    return new URL(url).host;
  } catch {
    return undefined;
  }
}

async function indexCsv(zip: Uint8Array): Promise<string> {
  for await (const entry of zipEntries(zip)) {
    if (entry.name.endsWith(".csv")) return new TextDecoder().decode(await entry.bytes());
  }
  throw new Error("the Aozora Bunko index has no CSV");
}

/**
 * Where the archive keeps the text of a work's zip: the zip's first text
 * file, renamed after the zip, in a folder named after the zip.
 * `https://www.aozora.gr.jp/cards/001403/files/49985_ruby_37632.zip` is at
 * `cards/001403/files/49985_ruby_37632/49985_ruby_37632.txt`.
 */
function archivePath(url: string): string {
  const path = new URL(url).pathname.slice(1).replace(/\.zip$/, "");
  return `${path}/${path.slice(path.lastIndexOf("/") + 1)}.txt`;
}

/** The finished lines of a work's Shift_JIS text, without its legend, colophon, notes and ruby. */
function bodyLines(sjis: Uint8Array): string[] {
  const lines = new TextDecoder("shift_jis").decode(sjis).replaceAll("\r\n", "\n").split("\n");
  const body = [];
  for (const line of lines.slice(bodyStart(lines))) {
    if (line.startsWith("底本") || line.startsWith("入力者注")) break;
    body.push(stripRuby(line).replace(NOTE, ""));
  }
  return finishLines(body, false);
}

/**
 * The line after the second `-----` rule within the first 80 lines, which
 * closes the legend of notation; without one, the third line.
 */
function bodyStart(lines: string[]): number {
  const rules = lines.slice(0, 80).flatMap((line, i) => line.startsWith("-----") ? [i] : []);
  return rules.length >= 2 ? rules[1] + 1 : 2;
}

function stripRuby(line: string): string {
  return line.replace(RUBY, "").replaceAll("｜", "");
}
