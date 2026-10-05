import { type AsyncBuffer, parquetMetadataAsync, parquetReadObjects } from "hyparquet";
import { compressors } from "hyparquet-compressors";
import { type Document, documentOf } from "../docs/document.ts";
import { finishLines } from "../docs/text.ts";
import type { Source } from "../sources.ts";

const BASE_URL =
  "https://huggingface.co/datasets/HuggingFaceFW/fineweb-2/resolve/af9c13333eb981300149d5ca60a8e9d659b276b9/data/jpn_Jpan";
/** FineWeb-2's Japanese pages, each file one zstd-compressed parquet file, with the characters taken from it. */
const FILES = [
  { id: "fineweb2-jpn", url: `${BASE_URL}/test/000_00000.parquet`, budget: Infinity },
  { id: "fineweb2-jpn-train", url: `${BASE_URL}/train/000_00000.parquet`, budget: 40_000_000 },
];

export const source: Source = {
  name: "fineweb",
  ids: FILES.map((f) => f.id),
  async fetch(ctx) {
    for (const { id, url } of FILES) await ctx.get(id, url, { large: true });
  },
  async documents(record, ctx) {
    const { budget } = FILES.find((f) => f.id === record.source_id)!;
    return await fineweb2(record.source_id, ctx.path(record), budget);
  },
};

/**
 * One document per row of a FineWeb-2 parquet file, of the Japanese lines of
 * its text, in file order until their characters reach `budget`. The file is
 * read a row group at a time, and no further than the budget needs.
 */
export async function fineweb2(
  sourceId: string,
  path: string,
  budget = Infinity,
): Promise<Document[]> {
  using handle = await Deno.open(path, { read: true });
  const file = fileBuffer(handle, (await handle.stat()).size);
  const metadata = await parquetMetadataAsync(file);
  const docs: Document[] = [];
  let chars = 0;
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
      const doc = documentOf(
        `fineweb2:${uuidOf(id)}`,
        sourceId,
        finishLines(text.split("\n"), true),
      );
      if (!doc) continue;
      docs.push(doc);
      chars += [...doc.text].length;
      if (chars >= budget) return docs;
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
