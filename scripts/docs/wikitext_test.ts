import { assertEquals } from "@std/assert";
import { stripWikitext } from "./wikitext.ts";

Deno.test("plain text comes back unchanged", () => {
  assertEquals(stripWikitext("東京で雪が降った。\n二行目"), "東京で雪が降った。\n二行目");
});

Deno.test("list and definition marks at the start of a line go", () => {
  assertEquals(
    stripWikitext("*[[東京都|東京]]で発表\n** 入れ子の項目\n# 番号付き\n; 用語\n: 説明"),
    "東京で発表\n入れ子の項目\n番号付き\n用語\n説明",
  );
});

Deno.test("a self-closing ref goes alone, not with the text after it", () => {
  assertEquals(
    stripWikitext('前<ref name="a"/>重要な本文<ref name="b">注釈</ref>後<ref name="c" />。'),
    "前重要な本文後。",
  );
});

Deno.test("templates, tables, refs and comments go with their content", () => {
  assertEquals(
    stripWikitext('前{{a|{{b}}c}}中{|\n|x\n|}後<ref name="r">注</ref>終<!-- 隠し -->了'),
    "前中後終了",
  );
});

Deno.test("file and category links go with their content", () => {
  assertEquals(
    stripWikitext(
      "[[File:a.jpg|thumb|[[東京]]の写真]]本文[[ファイル:b.png]][[Category:c]][[カテゴリ:d|e]]",
    ),
    "本文",
  );
});

Deno.test("image and media links under their other names go too", () => {
  assertEquals(
    stripWikitext(
      "[[画像:a.jpg|thumb|説明]]本文[[Image:b.jpg|right]][[image:c.jpg]][[Media:d.ogg]][[メディア:e.ogg]]",
    ),
    "本文",
  );
});

Deno.test("an unclosed template or link stays as text instead of swallowing the rest", () => {
  assertEquals(
    stripWikitext("{{a}}前{{b\n本文。\n[[ファイル:c.jpg|説明\n後の段落。"),
    "前{{b\n本文。\n[[ファイル:c.jpg|説明\n後の段落。",
  );
});

Deno.test("links to other namespaces stay as their visible text", () => {
  assertEquals(stripWikitext("[[Wikipedia:方針|方針]]を読む"), "方針を読む");
});

Deno.test("links become their visible text", () => {
  assertEquals(
    stripWikitext(
      "[[東京都|東京]]と[[大阪]]、[https://example.com 例のサイト]と[https://example.com]",
    ),
    "東京と大阪、例のサイトと",
  );
});

Deno.test("emphasis, tags and references are unwrapped", () => {
  assertEquals(
    stripWikitext(
      "'''太字'''と''斜体''と<span class=\"x\">囲み</span>\n&amp;&#26085;&#x672C;&nbsp;",
    ),
    "太字と斜体と囲み\n&日本\u00a0",
  );
});

Deno.test("a lone less-than sign survives decoding references", () => {
  assertEquals(stripWikitext("1 < 2 &amp; 3"), "1 < 2 & 3");
});

Deno.test("a gallery goes with its files and captions", () => {
  assertEquals(
    stripWikitext(
      '前\n<gallery mode="packed">\nFile:a.jpg|拝殿\nファイル:b.JPG|本殿（2023年7月）\n</gallery>\n後',
    ),
    "前\n\n後",
  );
});

Deno.test("a section heading goes, since it is no sentence", () => {
  assertEquals(
    stripWikitext("本文。\n== 脚注 ==\n=== 外部リンク===\n続き。"),
    "本文。\n\n\n続き。",
  );
});

Deno.test("brackets a removed template left empty go", () => {
  assertEquals(
    stripWikitext(
      "インセンティブ（{{lang-en|incentive}}）は誘因（{{lang|en|x}}, {{lang|fr|y}}）。" +
        "「ゾーン ({{lang|en|x}})」とPOEL ({{lang|el|y}}) です。",
    ),
    "インセンティブは誘因。「ゾーン」とPOEL です。",
  );
});

Deno.test("brackets left empty by removing the empty brackets inside them go", () => {
  assertEquals(stripWikitext("名（{{lang|en|a}}（{{lang|fr|b}}））です。"), "名です。");
});

Deno.test("separators a removed template left at the edge of brackets go", () => {
  assertEquals(
    stripWikitext(
      "中央銀行（ちゅうおうぎんこう、{{lang-en|central bank}}）と潜流（せんりゅう、{{en|a}}、{{fr|b}}）と" +
        "選手（{{lang-de|x}}, 1946年 - ）と（{{en|a}}、略称: SF）",
    ),
    "中央銀行（ちゅうおうぎんこう）と潜流（せんりゅう）と選手（1946年 -）と（略称: SF）",
  );
});

Deno.test("a comma a removed template left before a full stop goes", () => {
  assertEquals(stripWikitext("登録名は、{{lang|zh|索普}}。"), "登録名は。");
});

Deno.test("brackets with content and code calls keep their marks", () => {
  assertEquals(
    stripWikitext("関数 f() と printf() を（例、二つ）呼ぶ。"),
    "関数 f() と printf() を（例、二つ）呼ぶ。",
  );
});
