/**
 * The content-addressed store under `build/raw/`: every fetched file is kept
 * as fetched, named by the SHA-256 of its content, and never rewritten.
 */
import { crypto as stdCrypto } from "@std/crypto";
import { join } from "@std/path";

export async function sha256Hex(data: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", data as BufferSource));
  return Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function isSha256(hex: string): boolean {
  return /^[0-9a-f]{64}$/.test(hex);
}

export class RawStore {
  constructor(readonly dir: string) {}

  path(sha: string): string {
    return join(this.dir, sha);
  }

  async contains(sha: string): Promise<boolean> {
    return (await statOrUndefined(this.path(sha)))?.isFile ?? false;
  }

  read(sha: string): Promise<Uint8Array> {
    return Deno.readFile(this.path(sha));
  }

  /**
   * Stores `data` unless a file with the same hash is already there.
   *
   * The content goes to a temporary file of its own and is hard-linked into
   * place, which never replaces anything, so neither an interrupted write nor
   * a concurrent put of the same content can leave a file whose name promises
   * content it does not have.
   */
  async put(data: Uint8Array): Promise<string> {
    const sha = await sha256Hex(data);
    const path = this.path(sha);
    if (await this.contains(sha)) return sha;
    await Deno.mkdir(this.dir, { recursive: true });
    const tmp = await Deno.makeTempFile({ dir: this.dir, suffix: ".tmp" });
    try {
      await Deno.writeFile(tmp, data);
      await Deno.link(tmp, path);
    } catch (e) {
      // Another put stored the same content first; theirs is as good.
      if (!(e instanceof Deno.errors.AlreadyExists && await this.contains(sha))) throw e;
    } finally {
      await Deno.remove(tmp);
    }
    return sha;
  }

  /**
   * Stores what `stream` yields, never holding it whole: it goes to a
   * temporary file, is hashed from there, and is linked into place as
   * [`put`] does. Returns its hash and size.
   */
  async putStream(stream: ReadableStream<Uint8Array>): Promise<{ sha256: string; bytes: number }> {
    await Deno.mkdir(this.dir, { recursive: true });
    const tmp = await Deno.makeTempFile({ dir: this.dir, suffix: ".tmp" });
    try {
      const out = await Deno.open(tmp, { write: true });
      await stream.pipeTo(out.writable);
      const file = await Deno.open(tmp, { read: true });
      const digest = new Uint8Array(await stdCrypto.subtle.digest("SHA-256", file.readable));
      const sha = Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
      const bytes = (await Deno.stat(tmp)).size;
      if (!await this.contains(sha)) {
        try {
          await Deno.link(tmp, this.path(sha));
        } catch (e) {
          if (!(e instanceof Deno.errors.AlreadyExists && await this.contains(sha))) throw e;
        }
      }
      return { sha256: sha, bytes };
    } finally {
      await Deno.remove(tmp);
    }
  }
}

export async function statOrUndefined(path: string): Promise<Deno.FileInfo | undefined> {
  try {
    return await Deno.stat(path);
  } catch (e) {
    if (e instanceof Deno.errors.NotFound) return undefined;
    throw e;
  }
}
