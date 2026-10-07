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
