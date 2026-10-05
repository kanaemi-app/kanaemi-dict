/** The text taken out of one work, law, article, file or page. */
export type Document = {
  doc_id: string;
  source_id: string;
  text: string;
};

/** A document from finished lines, or none when no line is left. */
export function documentOf(
  doc_id: string,
  source_id: string,
  lines: string[],
): Document | undefined {
  return lines.length > 0 ? { doc_id, source_id, text: lines.join("\n") } : undefined;
}
