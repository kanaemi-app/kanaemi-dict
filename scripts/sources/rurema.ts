import { UntarStream } from "@std/tar";
import { type Document, documentOf } from "../docs/document.ts";
import { finishLines } from "../docs/text.ts";
import type { Source } from "../sources.ts";

const ID = "rurema";
/** The Ruby reference manual's doctree, pinned to a commit. */
const SOURCE_URL =
  "https://codeload.github.com/rurema/doctree/tar.gz/cc68abf27520dd4e6e60a6f94fb88be9550373b7";

export const source: Source = {
  name: "rurema",
  ids: [ID],
  async fetch(ctx) {
    await ctx.get(ID, SOURCE_URL);
  },
  async documents(record, ctx) {
    return await rurema(record.source_id, await ctx.read(record));
  },
};

const MARKUP_PREFIXES = ["#@", "--- ", "@", "="];
/** Directives closed by their own `#@end`. */
const BLOCK_DIRECTIVE = /^#@(samplecode|since|until|if)\b/;
/** A reference such as `[[c:String]]` or `[[m:$!]]`, read as the name it points at. */
const REFERENCE = /\[\[[A-Za-z]+:([^\]]+)\]\]/g;

/**
 * One document per file under `refm/api/src/` in the Ruby reference manual's
 * doctree archive.
 */
export async function rurema(sourceId: string, tarGz: Uint8Array): Promise<Document[]> {
  const entries = ReadableStream.from([new Uint8Array(tarGz)])
    .pipeThrough(new DecompressionStream("gzip"))
    .pipeThrough(new UntarStream());
  const files: [string, string][] = [];
  for await (const entry of entries) {
    const isFile = entry.header.typeflag === "0" || entry.header.typeflag === "\0";
    if (!entry.readable) continue;
    if (!isFile || !entry.path.includes("/refm/api/src/")) {
      await entry.readable.cancel();
      continue;
    }
    files.push([entry.path, await new Response(entry.readable).text()]);
  }
  files.sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
  return files.flatMap(([name, text]) => {
    const path = name.slice(name.indexOf("/") + 1);
    return documentOf(`rurema:${path}`, sourceId, finishLines(prose(text), true)) ?? [];
  });
}

function prose(text: string): string[] {
  const out = [];
  let inCode = false;
  let sampleDepth = 0;
  for (const line of text.split("\n")) {
    if (sampleDepth > 0 || line.startsWith("#@samplecode")) {
      if (BLOCK_DIRECTIVE.test(line)) sampleDepth++;
      else if (line.startsWith("#@end")) sampleDepth--;
      continue;
    }
    if (line.startsWith("//emlist") || line.startsWith("//}")) {
      inCode = line.startsWith("//emlist");
      continue;
    }
    if (!inCode && !MARKUP_PREFIXES.some((p) => line.startsWith(p))) {
      out.push(line.replace(REFERENCE, "$1"));
    }
  }
  return out;
}
