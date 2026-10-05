import { parseHTML } from "linkedom";

const BLOCKS = new Set([
  "P",
  "DIV",
  "SECTION",
  "ARTICLE",
  "MAIN",
  "H1",
  "H2",
  "H3",
  "H4",
  "H5",
  "H6",
  "LI",
  "DT",
  "DD",
  "BLOCKQUOTE",
  "PRE",
  "FIGURE",
  "FIGCAPTION",
  "ADDRESS",
  "UL",
  "OL",
  "DL",
  "TR",
  "TD",
  "TH",
]);
const DROPPED = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "SVG", "IFRAME"]);

export type LineOptions = { keepTables?: boolean };

export function parseDocument(html: string): Document {
  return parseHTML(html).document;
}

/**
 * The text under `root`, a new line at every block boundary and break.
 * Scripts and other non-prose elements are left out, and tables too unless
 * `keepTables` is set; a kept table gives one line per cell.
 */
export function elementLines(root: Node, { keepTables = false }: LineOptions = {}): string[] {
  let text = "";
  const walk = (node: Node) => {
    for (const child of node.childNodes) {
      if (child.nodeType === 3) {
        text += (child.textContent ?? "").replace(/[ \t\r\n\f]+/g, " ");
      } else if (child.nodeType === 1) {
        const name = (child as Element).tagName.toUpperCase();
        if (DROPPED.has(name) || (name === "TABLE" && !keepTables)) continue;
        if (name === "BR" || name === "HR") {
          text += "\n";
          continue;
        }
        const block = BLOCKS.has(name);
        if (block) text += "\n";
        walk(child);
        if (block) text += "\n";
      }
    }
  };
  walk(root);
  return text.split("\n").map((line) => line.replace(/ +/g, " "));
}
