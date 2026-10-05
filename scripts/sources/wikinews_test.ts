import { assertEquals } from "@std/assert";
import { bzip2File, dump, page } from "../docs/mediawiki_test_helper.ts";
import { wikinews } from "./wikinews.ts";

Deno.test("articles in the main namespace become documents of their Japanese lines", async () => {
  const path = await bzip2File(dump([
    page(1, 0, "", "{{日付|2010年1月2日}}\n東京で&lt;b&gt;雪&lt;/b&gt;が降った。\nEnglish line"),
    page(2, 0, "", "{{日付|2004年9月24日}}\n古い記事"),
    page(3, 0, "", "日付のない記事"),
    page(4, 1, "", "{{日付|2010年1月2日}}\nノート"),
    page(5, 0, '<redirect title="x" />', "{{日付|2010年1月2日}}\n転送"),
    page(6, 0, "", "{{日付|2010年1月2日}}\nEnglish only"),
  ]));

  assertEquals(await wikinews("wikinews-ja", path), [
    { doc_id: "wikinews:1", source_id: "wikinews-ja", text: "東京で雪が降った。" },
    { doc_id: "wikinews:2", source_id: "wikinews-ja", text: "古い記事" },
    { doc_id: "wikinews:3", source_id: "wikinews-ja", text: "日付のない記事" },
  ]);
});
