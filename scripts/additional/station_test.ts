import { assert, assertEquals } from "@std/assert";
import { STATIONS_URL } from "./station.ts";

Deno.test("the stations are asked of QLever as a TSV export of Japan's stations with a reading", () => {
  const url = new URL(STATIONS_URL);
  assertEquals(url.origin + url.pathname, "https://qlever.dev/api/wikidata");
  assertEquals(url.searchParams.get("action"), "tsv_export");
  const query = url.searchParams.get("query") ?? "";
  assert(query.includes("wdt:P31/wdt:P279* wd:Q55488"));
  assert(query.includes("wdt:P17 wd:Q17"));
  assert(query.includes("wdt:P1814 ?kana"));
  assert(query.includes("SELECT ?item ?kana ?label ?links"));
});
