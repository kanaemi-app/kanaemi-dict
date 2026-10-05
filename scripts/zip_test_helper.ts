import { Uint8ArrayReader, Uint8ArrayWriter, ZipWriter } from "@zip-js/zip-js";

/** A zip holding `files` in the given order, for tests. */
export async function zipOf(
  files: [name: string, body: Uint8Array | string][],
): Promise<Uint8Array> {
  const writer = new ZipWriter(new Uint8ArrayWriter());
  for (const [name, body] of files) {
    const bytes = typeof body === "string" ? new TextEncoder().encode(body) : body;
    await writer.add(name, new Uint8ArrayReader(bytes));
  }
  return await writer.close();
}
