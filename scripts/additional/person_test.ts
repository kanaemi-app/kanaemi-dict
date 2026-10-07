import { assert, assertEquals } from "@std/assert";
import { PEOPLE_URL } from "./person.ts";

Deno.test("the people are asked of QLever as a TSV export of humans with a reading", () => {
  const url = new URL(PEOPLE_URL);
  assertEquals(url.origin + url.pathname, "https://qlever.dev/api/wikidata");
  assertEquals(url.searchParams.get("action"), "tsv_export");
  const query = url.searchParams.get("query") ?? "";
  assert(query.includes("wdt:P31 wd:Q5"));
  assert(query.includes("wdt:P1814 ?kana"));
  assert(query.includes("SELECT ?item ?kana ?label ?links ?ja ?birth"));
});
