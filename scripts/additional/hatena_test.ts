import { assertEquals } from "@std/assert";
import { HttpError } from "../raw/fetch.ts";
import { daysOf, hatenaHotentry, notPublishedYet } from "./hatena.ts";

Deno.test("only the last day missing is a day not published yet", () => {
  const missing = new HttpError("https://b.hatena.ne.jp/hotentry/all/20261007", 404);
  assertEquals(notPublishedYet(missing, true), true);
  assertEquals(notPublishedYet(missing, false), false);
  assertEquals(notPublishedYet(new HttpError("https://b.hatena.ne.jp/", 500), true), false);
  assertEquals(notPublishedYet(new Error("offline"), true), false);
});

const encode = (s: string) => new TextEncoder().encode(s);

const entry = (title: string, description: string) =>
  `<li><h3 class="entrylist-contents-title"><a href="https://example.com/" title="${title}">` +
  `${title}</a></h3><p class="entrylist-contents-description">${description}</p></li>`;

Deno.test("a day's hot entries come out as one document of their titles and summaries", () => {
  const html = `<html><body><ul class="entrylist-item js-hotentries">${
    entry("中道改革連合が&quot;発足&quot;", "新党の結成について。") +
    entry("Deleted articles cannot be recovered.", "English only.") +
    entry("ナフサショックの影響", "")
  }</ul><p>関係ない本文</p><ul class="entrylist-bottom">${
    entry("最近のはてなブログの記事", "取得した日の記事。")
  }</ul></body></html>`;

  assertEquals(hatenaHotentry("hatena-hotentry:20260315", encode(html)), [{
    doc_id: "hatena:20260315",
    source_id: "hatena-hotentry:20260315",
    text: '中道改革連合が"発足"\n新党の結成について。\nナフサショックの影響',
  }]);
});

Deno.test("a day without entries is no document", () => {
  assertEquals(hatenaHotentry("hatena-hotentry:20260101", encode("<html></html>")), []);
});

Deno.test("the days of a year are those before today in Japan", () => {
  const now = new Date("2026-01-03T03:00:00Z");

  assertEquals(daysOf(2026, now), ["20260101", "20260102"]);
  assertEquals(daysOf(2026, new Date("2026-01-01T03:00:00Z")), []);
  assertEquals(daysOf(2025, now).length, 365, "a past year is whole");
  assertEquals(daysOf(2025, now).at(-1), "20251231");
  assertEquals(daysOf(2027, now), [], "a year to come has none");
});
