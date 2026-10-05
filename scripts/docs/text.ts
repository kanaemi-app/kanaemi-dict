/**
 * Line handling shared by every source: normalization, the Japanese-line
 * test, and finishing the lines a source reader produced.
 */

const HALF_WIDTH_KANA = /[｡-ﾟ]+/g;
const FULL_WIDTH_LETTER = /[Ａ-Ｚａ-ｚ]/g;
const JAPANESE = /[ぁ-ゖァ-ヺ㐀-鿿]/;

/**
 * NFC, then half-width katakana and punctuation to full width, then
 * full-width ASCII letters to half width. Full-width digits stay, since
 * their width is how the text writes a number.
 */
export function normalize(s: string): string {
  return s
    .normalize("NFC")
    .replace(HALF_WIDTH_KANA, (run) => run.normalize("NFKC"))
    .replace(FULL_WIDTH_LETTER, (ch) => String.fromCharCode(ch.charCodeAt(0) - 0xFEE0));
}

export function isJapanese(line: string): boolean {
  return JAPANESE.test(line);
}

export function finishLines(lines: Iterable<string>, japaneseOnly: boolean): string[] {
  const out = [];
  for (const line of lines) {
    const finished = normalize(line).trim();
    if (finished && (!japaneseOnly || isJapanese(finished))) out.push(finished);
  }
  return out;
}
