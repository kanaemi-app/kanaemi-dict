import { type AsyncBuffer, parquetMetadataAsync, parquetReadObjects } from "hyparquet";
import { compressors } from "hyparquet-compressors";
import { type Document, documentOf } from "../docs/document.ts";
import { finishLines } from "../docs/text.ts";
import type { Source } from "../sources.ts";

const BASE_URL =
  "https://huggingface.co/datasets/HuggingFaceFW/fineweb-2/resolve/af9c13333eb981300149d5ca60a8e9d659b276b9/data/jpn_Jpan";
/**
 * FineWeb-2's Japanese pages, each file one zstd-compressed parquet file, with
 * the characters taken from it, and the characters of workplace documents
 * taken past them.
 */
const FILES = [
  { id: "fineweb2-jpn", url: `${BASE_URL}/test/000_00000.parquet`, budget: Infinity, business: 0 },
  {
    id: "fineweb2-jpn-train",
    url: `${BASE_URL}/train/000_00000.parquet`,
    budget: 40_000_000,
    business: 50_000_000,
  },
];

/**
 * A workplace document holds three of these words or more; the workplace
 * vocabulary that comes with them is rare in the rest of the text.
 */
const BUSINESS_WORDS = [
  "弊社",
  "御社",
  "案件",
  "エンジニア",
  "社内",
  "ミーティング",
  "タスク",
  "チーム",
];

/**
 * A workplace document holding two of these words or more is a job
 * advertisement: their words (自己PR, 月給) would come ahead of the general
 * ones (事故 for じこ). One alone is often a link on a company's page.
 */
const JOB_WORDS = [
  "応募",
  "月給",
  "時給",
  "年収",
  "募集要項",
  "求人",
  "採用情報",
  "給与",
  "福利厚生",
  "自己PR",
  "転職",
  "選考",
];

function isBusiness(text: string): boolean {
  const held = (words: string[]) => words.filter((w) => text.includes(w)).length;
  return held(BUSINESS_WORDS) >= 3 && held(JOB_WORDS) < 2;
}

export const source: Source = {
  name: "fineweb",
  ids: FILES.map((f) => f.id),
  async fetch(ctx) {
    for (const { id, url } of FILES) await ctx.get(id, url, { large: true });
  },
  async documents(record, ctx) {
    const { budget, business } = FILES.find((f) => f.id === record.source_id)!;
    return await fineweb2(record.source_id, ctx.path(record), budget, business);
  },
};

/**
 * One document per row of a FineWeb-2 parquet file, of the Japanese lines of
 * its text, in file order until their characters reach `budget`; past them,
 * the workplace documents (`fineweb2-business:`), in file order until their
 * characters reach `business`. The file is read a row group at a time, and no
 * further than the budgets need.
 */
export async function fineweb2(
  sourceId: string,
  path: string,
  budget = Infinity,
  business = 0,
): Promise<Document[]> {
  using handle = await Deno.open(path, { read: true });
  const file = fileBuffer(handle, (await handle.stat()).size);
  const metadata = await parquetMetadataAsync(file);
  const docs: Document[] = [];
  let chars = 0;
  let businessChars = 0;
  let rowStart = 0;
  for (const group of metadata.row_groups) {
    const rowEnd = rowStart + Number(group.num_rows);
    const rows = await parquetReadObjects({
      file,
      metadata,
      compressors,
      columns: ["id", "text"],
      rowStart,
      rowEnd,
    });
    for (const [i, { id, text }] of rows.entries()) {
      if (typeof id !== "string" || typeof text !== "string") {
        throw new Error(`row ${rowStart + i} has no string id and text`);
      }
      const general = chars < budget;
      const lines = finishLines(text.split("\n"), true);
      if (!general && !isBusiness(lines.join("\n"))) continue;
      const kind = general ? "fineweb2" : "fineweb2-business";
      const doc = documentOf(`${kind}:${uuidOf(id)}`, sourceId, lines);
      if (!doc) continue;
      docs.push(doc);
      const length = [...doc.text].length;
      if (general) chars += length;
      else businessChars += length;
      if (chars >= budget && businessChars >= business) return docs;
    }
    rowStart = rowEnd;
  }
  return docs;
}

/**
 * An open file as hyparquet reads it. hyparquet asks for several slices at
 * once, and the reads share the file's one position, so they take turns.
 */
function fileBuffer(handle: Deno.FsFile, byteLength: number): AsyncBuffer {
  let turn: Promise<unknown> = Promise.resolve();
  const readAt = async (start: number, length: number) => {
    const buf = new Uint8Array(length);
    await handle.seek(start, Deno.SeekMode.Start);
    for (let read = 0; read < length;) {
      const n = await handle.read(buf.subarray(read));
      if (n === null) throw new Error(`the file ends at ${start + read} of ${byteLength} bytes`);
      read += n;
    }
    return buf.buffer;
  };
  return {
    byteLength,
    slice(start, end) {
      const next = turn.then(() => readAt(start, (end ?? byteLength) - start));
      turn = next.catch(() => {});
      return next;
    },
  };
}

/** The UUID of a row ID written `<urn:uuid:…>`, or the ID itself when it is not. */
function uuidOf(id: string): string {
  return id.match(/^<urn:uuid:(.+)>$/)?.[1] ?? id;
}
