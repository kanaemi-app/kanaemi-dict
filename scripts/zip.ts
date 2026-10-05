import { type Entry, Uint8ArrayReader, Uint8ArrayWriter, ZipReader } from "@zip-js/zip-js";

export type ZipEntry = { name: string; bytes: () => Promise<Uint8Array> };

/** Every file entry of a zip, in archive order, with a reader for its bytes. */
export async function* zipEntries(zip: Uint8Array): AsyncGenerator<ZipEntry> {
  const reader = new ZipReader(new Uint8ArrayReader(zip));
  try {
    for (const entry of await reader.getEntries()) {
      if (entry.directory) continue;
      yield { name: entry.filename, bytes: () => bytesOf(entry) };
    }
  } finally {
    await reader.close();
  }
}

function bytesOf(entry: Entry): Promise<Uint8Array> {
  if (entry.directory) return Promise.resolve(new Uint8Array());
  return entry.getData(new Uint8ArrayWriter());
}
