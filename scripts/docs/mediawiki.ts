/**
 * Reading MediaWiki XML exports (the Wikipedia and Wikinews dumps): their
 * pages as they stream, and what a page's wikitext says about the page.
 */
import { SaxesParser } from "saxes";

/** What the export says about a page before its text. */
export type PageHeader = { ns: string; id: string; title: string; redirect: boolean };

export type Page = PageHeader & { text: string };

/**
 * The pages of an export as its XML streams in, in export order. A page
 * `wanted` turns down by its header does not come out, and its text is never
 * held.
 */
export async function* dumpPages(
  xml: AsyncIterable<string>,
  wanted: (header: PageHeader) => boolean = () => true,
): AsyncGenerator<Page> {
  const parser = new SaxesParser();
  const path: string[] = [];
  const ready: Page[] = [];
  let page: Page = { ns: "", id: "", title: "", redirect: false, text: "" };
  let keep = false;
  parser.on("opentag", (tag) => {
    if (tag.name === "page") page = { ns: "", id: "", title: "", redirect: false, text: "" };
    if (tag.name === "redirect" && path.at(-1) === "page") page.redirect = true;
    // The export puts the title, namespace, ID and redirect before the revision.
    if (tag.name === "revision" && path.at(-1) === "page") {
      const { ns, id, title, redirect } = page;
      keep = wanted({ ns, id, title, redirect });
    }
    if (!tag.isSelfClosing) path.push(tag.name);
  });
  parser.on("closetag", (tag) => {
    if (tag.isSelfClosing) return;
    path.pop();
    if (tag.name === "page" && keep) ready.push(page);
    if (tag.name === "page") keep = false;
  });
  parser.on("text", (text) => {
    const tail = path.slice(path.lastIndexOf("page")).join("/");
    if (tail === "page/ns") page.ns += text;
    else if (tail === "page/id") page.id += text;
    else if (tail === "page/title") page.title += text;
    else if (tail === "page/revision/text" && keep) page.text += text;
  });
  for await (const chunk of xml) {
    parser.write(chunk);
    yield* ready.splice(0);
  }
  parser.close();
  yield* ready.splice(0);
}

/**
 * The text of a bzip2 file, decompressed by `lbzip2` as it is read; every
 * stream of a file of concatenated streams is read. Leaving it unfinished
 * stops the decompression.
 */
export async function* bzip2Text(path: string): AsyncGenerator<string> {
  const child = new Deno.Command("lbzip2", {
    args: ["-dc", path],
    // Deno refuses to pass the LD_* variables a Nix shell sets to a command
    // allowed by name, and lbzip2 needs no environment.
    clearEnv: true,
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  const stderr = new Response(child.stderr).text();
  let finished = false;
  try {
    yield* child.stdout.pipeThrough(new TextDecoderStream());
    finished = true;
  } finally {
    if (!finished) {
      try {
        child.kill();
      } catch {
        // lbzip2 may already have written everything out and exited, and
        // Deno refuses to kill an exited child.
      }
      await child.status;
      await stderr;
    }
  }
  // A corrupt file can decompress into text that reads well before lbzip2
  // notices; only its exit tells.
  const status = await child.status;
  if (!status.success) throw new Error(`lbzip2 failed on ${path}: ${(await stderr).trim()}`);
}

const COMMENT = /<!--[\s\S]*?(?:-->|$)/g;
const CATEGORY = /\[\[\s*(?:Category|カテゴリ)\s*:\s*([^\]|]+)(?:\|[^\]]*)?\]\]/gi;
const DISAMBIGUATION_TEMPLATE =
  /\{\{\s*(?:aimai|disambig|曖昧さ回避|人名の曖昧さ回避|地名の曖昧さ回避)\s*[|}]/i;

/** The categories a page's wikitext puts it in, in order, outside comments. */
export function categoriesOf(wikitext: string): string[] {
  return [...wikitext.replace(COMMENT, "").matchAll(CATEGORY)].map((m) => m[1].trim());
}

/**
 * Whether a page is a disambiguation page, by the template that marks one
 * or a disambiguation category.
 */
export function isDisambiguation(wikitext: string): boolean {
  return DISAMBIGUATION_TEMPLATE.test(wikitext.replace(COMMENT, "")) ||
    categoriesOf(wikitext).some((c) => c.includes("曖昧さ回避"));
}
