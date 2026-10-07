/**
 * The tables of Mozc that the symbol, emoji and emoticon dictionaries are
 * taken from, at the commit the base dictionary's single kanji table is
 * pinned to.
 */

const MOZC =
  "https://raw.githubusercontent.com/google/mozc/23366530244c9c85a7c0c0e0e766e04fde8e0d4d";

export type MozcTable = {
  sourceId: string;
  url: string;
  /** The additional dictionary the table builds. */
  dictionary: string;
  /** The name it is written out under in build/additional/DICTIONARY/. */
  name: string;
};

export const MOZC_TABLES: MozcTable[] = [
  {
    sourceId: "mozc-symbol",
    url: `${MOZC}/src/data/symbol/symbol.tsv`,
    dictionary: "symbol",
    name: "symbol.tsv",
  },
  {
    sourceId: "mozc-emoji:emoji_data",
    url: `${MOZC}/src/data/emoji/emoji_data.tsv`,
    dictionary: "emoji",
    name: "emoji_data.tsv",
  },
  {
    sourceId: "mozc-emoji:manual_emoji_data",
    url: `${MOZC}/src/data/emoji/manual_emoji_data.tsv`,
    dictionary: "emoji",
    name: "manual_emoji_data.tsv",
  },
  {
    sourceId: "mozc-emoticon",
    url: `${MOZC}/src/data/emoticon/emoticon.tsv`,
    dictionary: "emoticon",
    name: "emoticon.tsv",
  },
];
