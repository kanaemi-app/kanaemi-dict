import { assert, assertEquals } from "@std/assert";
import { finishLines, isJapanese, normalize } from "./text.ts";

Deno.test("normalize composes to NFC", () => {
  assertEquals(normalize("が"), "が");
});

Deno.test("normalize widens half-width katakana and joins voicing marks", () => {
  assertEquals(normalize("ｶﾞｲﾄﾞ｡"), "ガイド。");
});

Deno.test("normalize narrows full-width letters only", () => {
  assertEquals(normalize("ＡＢＣａｂｃ！"), "ABCabc！");
});

Deno.test("normalize keeps full-width digits, whose width tells how a number is written", () => {
  assertEquals(normalize("０１２個"), "０１２個");
});

Deno.test("normalize keeps symbols NFKC would fold", () => {
  assertEquals(normalize("①㍻"), "①㍻");
});

Deno.test("Japanese lines have kana or kanji", () => {
  assert(isJapanese("abc ひ"));
  assert(isJapanese("カ"));
  assert(isJapanese("漢"));
  assert(isJapanese("㐀"));
  assert(!isJapanese("print(1)"));
  assert(!isJapanese("ー・。"));
  assert(!isJapanese("隆"));
  assert(!isJapanese("𠮷"));
});

Deno.test("finishLines normalizes, then trims, then drops empty lines", () => {
  assertEquals(finishLines(["　 ﾃｽﾄ\t\r", "   ", "　", "ｂ"], false), ["テスト", "b"]);
});

Deno.test("finishLines can keep only Japanese lines, judged after normalization", () => {
  assertEquals(finishLines(["English only", "日本語 and English", "ｱ"], true), [
    "日本語 and English",
    "ア",
  ]);
});
