/** The additional dictionaries set up under `additional/`, one directory each. */
import { join } from "@std/path";

/** An additional dictionary, with what its setup says it is built from. */
export type Dictionary = {
  name: string;
  /** Patterns of the Wikipedia categories its articles are in. */
  patterns?: RegExp[];
  /** The first page ID of the articles of its year. */
  since?: number;
};

/** Every dictionary set up under `dir`, by name. */
export async function readDictionaries(dir: string): Promise<Dictionary[]> {
  const found: Dictionary[] = [];
  for await (const entry of Deno.readDir(dir)) {
    if (!entry.isDirectory || entry.name.startsWith(".")) continue;
    const path = join(dir, entry.name);
    const dictionary: Dictionary = { name: entry.name };
    const patterns = await readPatterns(join(path, "wikipedia.txt"));
    if (patterns !== undefined) dictionary.patterns = patterns;
    const since = await readOptional(join(path, "wikipedia-since.txt"));
    if (since !== undefined) {
      const id = Number(since.trim());
      if (!Number.isSafeInteger(id)) throw new Error(`${path}/wikipedia-since.txt: not a page ID`);
      dictionary.since = id;
    }
    found.push(dictionary);
  }
  return found.sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
}

/** The category patterns of a `wikipedia.txt`, one per line, if it is there. */
export async function readPatterns(path: string): Promise<RegExp[] | undefined> {
  const text = await readOptional(path);
  return text?.split("\n").map((l) => l.trim()).filter(Boolean).map((p) => new RegExp(p));
}

async function readOptional(path: string): Promise<string | undefined> {
  try {
    return await Deno.readTextFile(path);
  } catch (e) {
    if (e instanceof Deno.errors.NotFound) return undefined;
    throw e;
  }
}
