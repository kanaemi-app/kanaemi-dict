import { assertEquals } from "@std/assert";
import { dump, page } from "../docs/mediawiki_test_helper.ts";
import { keyHash } from "../select.ts";
import { articlesOf, fieldsOf, titleReading } from "./wikipedia.ts";

async function* once(s: string): AsyncGenerator<string> {
  yield await Promise.resolve(s);
}

Deno.test("an article goes to every dictionary one of whose patterns its categories match", () => {
  const dictionaries = [
    { name: "medical", patterns: [/感染症|疾患/] },
    { name: "railway", patterns: [/鉄道/, /駅$/] },
  ];

  assertEquals(fieldsOf(["日本の感染症", "東京都の駅"], 1, dictionaries), [
    "medical",
    "railway",
  ]);
  assertEquals(fieldsOf(["ドラマ"], 1, dictionaries), []);
});

Deno.test("a dictionary with a first page ID takes only the articles from it on", () => {
  const dictionaries = [{ name: "2026", patterns: [/./], since: 100 }];

  assertEquals(fieldsOf(["日本の楽曲"], 100, dictionaries), ["2026"]);
  assertEquals(fieldsOf(["日本の楽曲"], 99, dictionaries), []);
});

Deno.test("an article's title comes with the reading its lead gives in hiragana", () => {
  assertEquals(
    titleReading("冪等", "'''冪等'''（べきとう）とは、ある操作を…"),
    { surface: "冪等", reading: "べきとう" },
  );
  assertEquals(
    titleReading("ライドシェア (日本)", "'''ライドシェア'''（らいどしぇあ、英: Ridesharing）は…"),
    { surface: "ライドシェア", reading: "らいどしぇあ" },
  );
  assertEquals(
    titleReading("令和の米騒動", "{{Infobox}}\n'''令和の米騒動'''（れいわのこめそうどう）は…"),
    { surface: "令和の米騒動", reading: "れいわのこめそうどう" },
  );
  assertEquals(titleReading("東京", "'''東京'''（とうきょう、Tokyo）"), {
    surface: "東京",
    reading: "とうきょう",
  });
  assertEquals(titleReading("ミャクミャク", "'''ミャクミャク'''は…"), {
    surface: "ミャクミャク",
    reading: "みゃくみゃく",
  }, "katakana reads as itself");
  assertEquals(titleReading("ABC", "'''ABC'''（エービーシー）は…"), undefined, "not hiragana");
  assertEquals(titleReading("無読み", "'''無読み'''は…"), undefined, "no reading");
  assertEquals(
    titleReading("（笑）", "''''''（わら）は…"),
    undefined,
    "nothing left of the title",
  );
});

Deno.test("a title comes normalized as the text it is counted in", () => {
  assertEquals(
    titleReading("ｶﾞﾝﾀﾞﾑ作品", "'''ｶﾞﾝﾀﾞﾑ作品'''（がんだむさくひん）は…"),
    { surface: "ガンダム作品", reading: "がんだむさくひん" },
  );
});

Deno.test("a reading inside a comment is not the title's", () => {
  assertEquals(
    titleReading("漢字", "<!-- '''漢字'''（あやまり） -->\n'''漢字'''（かんじ）は文字。"),
    { surface: "漢字", reading: "かんじ" },
  );
});

Deno.test("a field takes its articles in SHA-256 order up to its budget, with their titles", async () => {
  const ids = [11, 12, 13, 14];
  const xml = dump([
    ...ids.map((id) =>
      page(id, 0, "", `'''語${id}'''（ご）は鉄道の話。\n[[Category:日本の鉄道]]`, `語${id}`)
    ),
    page(20, 0, "", "'''医'''（い）\n[[Category:医学]]", "医"),
    page(21, 0, "", "'''鉄'''（てつ）\n{{Aimai}}\n[[Category:鉄道]]", "鉄"),
    page(22, 0, '<redirect title="x" />', "[[Category:鉄道]]", "転送"),
  ]);
  const ordered = ids.map((id) => `wikipedia:${id}`).sort((a, b) =>
    keyHash(a) < keyHash(b) ? -1 : 1
  );

  const found = await articlesOf(
    once(xml),
    [{ name: "railway", patterns: [/鉄道/] }],
    "wikipedia-ja",
    20,
  );

  const railway = found.get("railway")!;
  assertEquals(railway.docs.map((d) => d.doc_id).sort(), ordered.slice(0, 2).sort());
  assertEquals(
    railway.titles.map((t) => t.doc_id).sort(),
    ordered.slice(0, 2).sort(),
  );
});

Deno.test("a year's dictionary takes the titles of every article from its first page on, and no text", async () => {
  const xml = dump([
    page(5, 0, "", "'''古語'''（こご）\n[[Category:言葉]]", "古語"),
    page(9, 0, "", "'''新語'''（しんご）\n[[Category:言葉]]", "新語"),
  ]);

  const found = await articlesOf(
    once(xml),
    [{ name: "2026", patterns: [/./], since: 9 }],
    "wikipedia-ja",
    100,
  );

  assertEquals(found.get("2026"), {
    docs: [],
    titles: [{ doc_id: "wikipedia:9", reading: "しんご", surface: "新語" }],
  });
});
