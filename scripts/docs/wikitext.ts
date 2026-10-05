/** Wikitext as a reader sees it: markup, templates, tables and media removed. */
import { parseHTML } from "linkedom";

const COMMENT = /<!--[\s\S]*?-->/g;
const SELF_CLOSING_REF = /<ref(?:\s[^>]*)?\/>/g;
const REF = /<ref(?:\s[^>]*[^/])?>[\s\S]*?<\/ref\s*>/g;
const LINK = /\[\[(?:[^[\]|]*\|)?([^[\]|]*)\]\]/g;
const EXTERNAL_LINK = /\[(?:https?:|ftp:|\/\/)[^\s\]]*(?:[ \t]+([^\]]*))?\]/g;
const GALLERY = /<gallery(?:\s[^>]*)?>[\s\S]*?<\/gallery\s*>/gi;
const HEADING = /^=+.*=+[ \t]*$/gm;
const TAG = /<[^>]+>/g;
const LIST_MARK = /^[*#:;]+[ \t]*/gm;
const MEDIA_PREFIXES = new Set([
  "file",
  "ファイル",
  "image",
  "画像",
  "media",
  "メディア",
  "category",
  "カテゴリ",
]);

/** The characters a reader sees, with wikitext markup removed. */
export function stripWikitext(wikitext: string): string {
  let s = wikitext.replace(COMMENT, "").replace(SELF_CLOSING_REF, "").replace(REF, "")
    .replace(GALLERY, "");
  s = removeNested(s, "{{", "}}");
  s = removeNested(s, "{|", "|}");
  s = removeMediaLinks(s);
  s = s.replace(LINK, "$1")
    .replace(EXTERNAL_LINK, (_, label) => label ?? "")
    .replace(HEADING, "")
    .replace(LIST_MARK, "")
    .replaceAll("'''", "")
    .replaceAll("''", "")
    .replace(TAG, "");
  return tidyPunctuation(decodeCharacterReferences(s));
}

/**
 * Removes every `open … close` span, nested ones included. An `open` never
 * closed stays as text, as MediaWiki shows it, rather than taking the rest
 * of the page with it.
 */
function removeNested(s: string, open: string, close: string): string {
  let out = "";
  let depth = 0;
  let outer = 0;
  for (let i = 0; i < s.length;) {
    if (s.startsWith(open, i)) {
      if (depth === 0) outer = i;
      depth++;
      i += open.length;
    } else if (depth > 0 && s.startsWith(close, i)) {
      depth--;
      i += close.length;
    } else {
      if (depth === 0) out += s[i];
      i++;
    }
    if (i >= s.length && depth > 0) {
      out += open;
      i = outer + open.length;
      depth = 0;
    }
  }
  return out;
}

/** Removes `[[File:…]]`-style links with everything inside them. */
function removeMediaLinks(s: string): string {
  let out = "";
  let rest = s;
  for (let start = rest.indexOf("[["); start >= 0; start = rest.indexOf("[[")) {
    out += rest.slice(0, start);
    const length = linkLength(rest.slice(start));
    if (length === undefined) {
      out += "[[";
      rest = rest.slice(start + 2);
      continue;
    }
    const link = rest.slice(start, start + length);
    const inner = link.slice(2, -2);
    const prefix = inner.split(":")[0].trim().toLowerCase();
    if (!(inner.includes(":") && MEDIA_PREFIXES.has(prefix))) out += link;
    rest = rest.slice(start + link.length);
  }
  return out + rest;
}

/** The length of the `[[…]]` at the start of `s`, nested links included; undefined when it never closes. */
function linkLength(s: string): number | undefined {
  let depth = 0;
  for (let i = 0; i < s.length;) {
    if (s.startsWith("[[", i)) {
      depth++;
      i += 2;
    } else if (s.startsWith("]]", i)) {
      depth--;
      i += 2;
      if (depth === 0) return i;
    } else {
      i++;
    }
  }
  return undefined;
}

function decodeCharacterReferences(s: string): string {
  if (!s.includes("&")) return s;
  const { document } = parseHTML("<html><body><pre></pre></body></html>");
  const pre = document.querySelector("pre")!;
  pre.innerHTML = s.replaceAll("<", "&lt;");
  return pre.textContent ?? s;
}

const SEPARATORS = "[\\s、,，;；]";
const AFTER_OPEN = new RegExp(`([（(])${SEPARATORS}+`, "g");
const BEFORE_CLOSE = new RegExp(`${SEPARATORS}+([）)])`, "g");
/** `()` right after code such as `f()` is kept. */
const EMPTY_BRACKETS = /（）|(?<=[^\p{ASCII}])\s*\(\)|\s+\(\)/gu;
const BEFORE_FULL_STOP = /[、,，]\s*(?=。)/g;

/**
 * Removes the separators a removed template leaves at the edge of brackets,
 * the brackets it leaves empty, and a comma it leaves before a full stop:
 * `読み（よみ、{{lang|en|…}}）` reads `読み（よみ）`.
 */
function tidyPunctuation(s: string): string {
  s = s.replace(AFTER_OPEN, "$1").replace(BEFORE_CLOSE, "$1");
  for (let prev = ""; prev !== s;) {
    prev = s;
    s = s.replace(EMPTY_BRACKETS, "");
  }
  return s.replace(BEFORE_FULL_STOP, "");
}
