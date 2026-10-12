/**
 * The railway stations of Wikidata the station name dictionary is built
 * from: every station in Japan with a reading in kana (P1814), with its
 * Japanese label and its site links, asked of QLever. QLever's index moves,
 * so a fetch is kept until fetched again on purpose.
 */

export const STATION = "station";
export const STATIONS_ID = "wikidata-stations";

const QUERY = `PREFIX wd: <http://www.wikidata.org/entity/>
PREFIX wdt: <http://www.wikidata.org/prop/direct/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
PREFIX wikibase: <http://wikiba.se/ontology#>
SELECT ?item ?kana ?label ?links WHERE {
  ?item wdt:P31/wdt:P279* wd:Q55488 .
  ?item wdt:P17 wd:Q17 .
  ?item wdt:P1814 ?kana .
  ?item rdfs:label ?label . FILTER(LANG(?label)="ja")
  OPTIONAL { ?item wikibase:sitelinks ?links }
}`;

export const STATIONS_URL = `https://qlever.dev/api/wikidata?${new URLSearchParams({
  query: QUERY,
  action: "tsv_export",
})}`;

/** The name the stations are written out under in build/additional/station/. */
export const STATIONS_FILE = "stations.tsv";
