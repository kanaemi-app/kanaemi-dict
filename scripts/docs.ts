/**
 * Takes the documents out of the fetched sources and writes them as JSON
 * Lines, reporting documents and characters per kind of document.
 *
 *     deno run -A scripts/docs.ts [DIR [OUT]]
 *
 * DIR defaults to build/raw and OUT to build/docs.jsonl.
 */
import { join } from "@std/path";
import { extractAll, writeJsonl } from "./docs/extract.ts";
import { loadManifest } from "./raw/manifest.ts";
import { RawStore } from "./raw/store.ts";

if (import.meta.main) {
  const dir = Deno.args[0] ?? "build/raw";
  const out = Deno.args[1] ?? "build/docs.jsonl";
  const records = await loadManifest(join(dir, "manifest.jsonl"));
  const docs = await extractAll(new RawStore(dir), records);
  await writeJsonl(out, docs);
  const byKind = new Map<string, { docs: number; chars: number }>();
  for (const doc of docs) {
    const kind = doc.doc_id.split(":")[0];
    const stat = byKind.get(kind) ?? { docs: 0, chars: 0 };
    stat.docs++;
    stat.chars += [...doc.text].length;
    byKind.set(kind, stat);
  }
  for (const [kind, { docs, chars }] of [...byKind].sort()) {
    console.log(`${kind}\tdocs: ${docs}\tchars: ${chars}`);
  }
  console.log(`total\tdocs: ${docs.length}\tout: ${out}`);
}
