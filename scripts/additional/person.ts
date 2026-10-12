/**
 * The people of Wikidata the person name dictionary is built from: every
 * human with a reading in kana (P1814), with their Japanese label, their
 * site links, their article in Japanese and their birth, asked of QLever.
 * QLever's index moves, so a fetch is kept until fetched again on purpose.
 */

export const PERSON = "person";
export const PEOPLE_ID = "wikidata-people";

const QUERY = `PREFIX wd: <http://www.wikidata.org/entity/>
PREFIX wdt: <http://www.wikidata.org/prop/direct/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
PREFIX wikibase: <http://wikiba.se/ontology#>
PREFIX schema: <http://schema.org/>
SELECT ?item ?kana ?label ?links ?ja ?birth WHERE {
  ?item wdt:P31 wd:Q5 .
  ?item wdt:P1814 ?kana .
  OPTIONAL { ?item rdfs:label ?label . FILTER(LANG(?label)="ja") }
  OPTIONAL { ?item wikibase:sitelinks ?links }
  OPTIONAL { ?ja schema:about ?item ; schema:isPartOf <https://ja.wikipedia.org/> }
  OPTIONAL { ?item wdt:P569 ?birth }
}`;

export const PEOPLE_URL = `https://qlever.dev/api/wikidata?${new URLSearchParams({
  query: QUERY,
  action: "tsv_export",
})}`;

/** The name the people are written out under in build/additional/person/. */
export const PEOPLE_FILE = "people.tsv";

/**
 * The people of Wikidata with an article in Japanese but no reading in kana,
 * whose readings the leads of their articles give: the export picks which
 * articles are people's, with their site links and their birth.
 */
export const UNREAD_ID = "wikidata-people-unread";

const UNREAD_QUERY = `PREFIX wd: <http://www.wikidata.org/entity/>
PREFIX wdt: <http://www.wikidata.org/prop/direct/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
PREFIX wikibase: <http://wikiba.se/ontology#>
PREFIX schema: <http://schema.org/>
SELECT ?item ?label ?links ?ja ?birth WHERE {
  ?item wdt:P31 wd:Q5 .
  ?ja schema:about ?item ; schema:isPartOf <https://ja.wikipedia.org/> .
  FILTER NOT EXISTS { ?item wdt:P1814 ?kana }
  OPTIONAL { ?item rdfs:label ?label . FILTER(LANG(?label)="ja") }
  OPTIONAL { ?item wikibase:sitelinks ?links }
  OPTIONAL { ?item wdt:P569 ?birth }
}`;

export const UNREAD_URL = `https://qlever.dev/api/wikidata?${new URLSearchParams({
  query: UNREAD_QUERY,
  action: "tsv_export",
})}`;

/**
 * The name the people read by their articles' leads are written out under in
 * build/additional/person/, in the form of the people's export.
 */
export const WIKIPEDIA_PEOPLE_FILE = "wikipedia-people.tsv";

/** A row of the unread people's export, its fields as the export writes them. */
export type UnreadRow = { item: string; links: string; ja: string; birth: string };

const ARTICLE = "<https://ja.wikipedia.org/wiki/";

/**
 * The rows of the unread people's export by the title of their article in
 * Japanese, as the dump writes it (`_` as a space). A row without an
 * article is left out.
 */
export function unreadPeople(tsv: string): Map<string, UnreadRow[]> {
  const people = new Map<string, UnreadRow[]>();
  for (const line of tsv.split("\n")) {
    const [item, , links = "", ja = "", birth = ""] = line.split("\t");
    if (!item?.startsWith("<") || !ja.startsWith(ARTICLE) || !ja.endsWith(">")) continue;
    let title: string;
    try {
      title = decodeURIComponent(ja.slice(ARTICLE.length, -1)).replaceAll("_", " ");
    } catch {
      continue;
    }
    const rows = people.get(title) ?? [];
    rows.push({ item, links, ja, birth });
    people.set(title, rows);
  }
  return people;
}

/**
 * The people of `people` whose leads `readings` read, by title, as sorted
 * lines of the people's export (`?item ?kana ?label ?links ?ja ?birth`): the
 * lead's reading as the kana, and the title without its qualifier as the
 * label, as it is the name written in Japanese.
 */
export function wikipediaPeople(
  people: Map<string, UnreadRow[]>,
  readings: Map<string, { surface: string; reading: string }>,
): string {
  const lines: string[] = [];
  for (const [title, { surface, reading }] of readings) {
    for (const { item, links, ja, birth } of people.get(title) ?? []) {
      lines.push(`${item}\t"${reading}"\t"${surface}"@ja\t${links}\t${ja}\t${birth}\n`);
    }
  }
  return `?item\t?kana\t?label\t?links\t?ja\t?birth\n${lines.sort().join("")}`;
}
