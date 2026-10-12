import { assert, assertEquals } from "@std/assert";
import { PEOPLE_URL, UNREAD_URL, unreadPeople, wikipediaPeople } from "./person.ts";

Deno.test("the people are asked of QLever as a TSV export of humans with a reading", () => {
  const url = new URL(PEOPLE_URL);
  assertEquals(url.origin + url.pathname, "https://qlever.dev/api/wikidata");
  assertEquals(url.searchParams.get("action"), "tsv_export");
  const query = url.searchParams.get("query") ?? "";
  assert(query.includes("wdt:P31 wd:Q5"));
  assert(query.includes("wdt:P1814 ?kana"));
  assert(query.includes("SELECT ?item ?kana ?label ?links ?ja ?birth"));
});

Deno.test("the unread people are asked of QLever as humans with an article in Japanese and no reading", () => {
  const url = new URL(UNREAD_URL);
  assertEquals(url.origin + url.pathname, "https://qlever.dev/api/wikidata");
  assertEquals(url.searchParams.get("action"), "tsv_export");
  const query = url.searchParams.get("query") ?? "";
  assert(query.includes("wdt:P31 wd:Q5"));
  assert(query.includes("schema:isPartOf <https://ja.wikipedia.org/>"));
  assert(query.includes("FILTER NOT EXISTS { ?item wdt:P1814 ?kana }"));
  assert(query.includes("SELECT ?item ?label ?links ?ja ?birth"));
});

const ENTITY = "http://www.wikidata.org/entity";
const WIKI = "https://ja.wikipedia.org/wiki";

Deno.test("the unread people go by the titles of their articles in Japanese", () => {
  const tsv = "?item\t?label\t?links\t?ja\t?birth\n" +
    `<${ENTITY}/Q541005>\t"呉清源"@ja\t20\t<${WIKI}/%E5%91%89%E6%B8%85%E6%BA%90>\t1914-06-12T00:00:00Z\n` +
    `<${ENTITY}/Q541005>\t"呉清源"@ja\t20\t<${WIKI}/%E5%91%89%E6%B8%85%E6%BA%90>\t1914-05-19T00:00:00Z\n` +
    `<${ENTITY}/Q2>\t\t\t<${WIKI}/%E5%B1%B1%E7%94%B0%E5%A4%AA%E9%83%8E_(%E6%94%BF%E6%B2%BB%E5%AE%B6)>\t\n` +
    `<${ENTITY}/Q3>\t"無記事"@ja\t1\t\t\n`;

  const people = unreadPeople(tsv);

  assertEquals([...people.keys()], ["呉清源", "山田太郎 (政治家)"]);
  assertEquals(people.get("呉清源")!.length, 2);
  assertEquals(people.get("山田太郎 (政治家)"), [
    {
      item: `<${ENTITY}/Q2>`,
      links: "",
      ja: `<${WIKI}/%E5%B1%B1%E7%94%B0%E5%A4%AA%E9%83%8E_(%E6%94%BF%E6%B2%BB%E5%AE%B6)>`,
      birth: "",
    },
  ]);
});

Deno.test("the people read by their leads are written as the people's export, the title their label", () => {
  const go = `<${WIKI}/%E5%91%89%E6%B8%85%E6%BA%90>`;
  const yamada = `<${WIKI}/%E5%B1%B1%E7%94%B0%E5%A4%AA%E9%83%8E_(%E6%94%BF%E6%B2%BB%E5%AE%B6)>`;
  const people = unreadPeople(
    "?item\t?label\t?links\t?ja\t?birth\n" +
      `<${ENTITY}/Q541005>\t"呉清源"@ja\t20\t${go}\t1914-06-12T00:00:00Z\n` +
      `<${ENTITY}/Q2>\t"Taro"@ja\t3\t${yamada}\t\n` +
      `<${ENTITY}/Q3>\t"不読"@ja\t1\t<${WIKI}/%E4%B8%8D%E8%AA%AD>\t\n`,
  );
  const readings = new Map([
    ["山田太郎 (政治家)", { surface: "山田太郎", reading: "やまだ たろう" }],
    ["呉清源", { surface: "呉清源", reading: "ご せいげん" }],
  ]);

  const written = wikipediaPeople(people, readings);

  assertEquals(
    written,
    "?item\t?kana\t?label\t?links\t?ja\t?birth\n" +
      `<${ENTITY}/Q2>\t"やまだ たろう"\t"山田太郎"@ja\t3\t${yamada}\t\n` +
      `<${ENTITY}/Q541005>\t"ご せいげん"\t"呉清源"@ja\t20\t${go}\t1914-06-12T00:00:00Z\n`,
    "a person whose lead gives no reading is left out",
  );
});
