import { parquetWriteBuffer } from "hyparquet-writer";
import { zstdCompressSync } from "node:zlib";

/**
 * A parquet file of FineWeb-2's columns, zstd-compressed as the dataset is,
 * with `rowGroupSize` rows to a row group; for tests.
 */
export function parquetOf(rows: { id: string; text: string }[], rowGroupSize = 1000): Uint8Array {
  const column = (name: string, data: string[]) => ({ name, data, type: "STRING" as const });
  const buffer = parquetWriteBuffer({
    columnData: [
      column("text", rows.map((r) => r.text)),
      column("id", rows.map((r) => r.id)),
      column("dump", rows.map(() => "CC-MAIN-2024-10")),
      column("url", rows.map((_, i) => `https://example.jp/${i}`)),
      column("date", rows.map(() => "2024-02-20T12:00:00Z")),
    ],
    codec: "ZSTD",
    compressors: { ZSTD: (input) => zstdCompressSync(input) },
    rowGroupSize,
  });
  return new Uint8Array(buffer);
}

/** [`parquetOf`] written to a temporary file; for tests. */
export async function parquetFile(
  rows: { id: string; text: string }[],
  rowGroupSize?: number,
): Promise<string> {
  const path = await Deno.makeTempFile({ suffix: ".parquet" });
  await Deno.writeFile(path, parquetOf(rows, rowGroupSize));
  return path;
}
