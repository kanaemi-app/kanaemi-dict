import { assertEquals } from "@std/assert";
import { dump, page } from "../docs/mediawiki_test_helper.ts";
import { keyHash } from "../select.ts";
import { recordingFetch } from "../sources_test_helper.ts";
import { source, wikipedia } from "./wikipedia.ts";

async function* once(s: string): AsyncGenerator<string> {
  yield await Promise.resolve(s);
}

Deno.test("articles become documents of their Japanese lines", async () => {
  const xml = dump([
    page(1, 0, "", "'''東京'''は&lt;b&gt;日本&lt;/b&gt;の首都。\n== 概要 ==\nEnglish line"),
    page(2, 1, "", "ノートの文章"),
    page(3, 0, '<redirect title="東京" />', "#転送 [[東京]]"),
    page(4, 0, "", "'''とうきょう'''\n* [[東京]]\n{{Aimai}}"),
    page(5, 0, "", "English only"),
  ]);

  assertEquals(await wikipedia("wikipedia-ja", once(xml), Infinity), [
    { doc_id: "wikipedia:1", source_id: "wikipedia-ja", text: "東京は日本の首都。" },
  ]);
});

Deno.test("articles are taken in the SHA-256 order of their doc IDs until the budget", async () => {
  const ids = [11, 12, 13, 14, 15, 16];
  const xml = dump(ids.map((id) => page(id, 0, "", "四文字だ")));
  const ordered = ids.map((id) => `wikipedia:${id}`)
    .sort((a, b) => keyHash(a) < keyHash(b) ? -1 : 1);

  const docs = await wikipedia("wikipedia-ja", once(xml), 9);

  assertEquals(docs.map((d) => d.doc_id), ordered.slice(0, 3));
});

Deno.test("fetching takes the latest dump as a large download", async () => {
  const { ctx, fetched } = recordingFetch();

  await source.fetch(ctx);

  assertEquals(fetched.map(([id, url, options]) => [id, url, options.large]), [[
    "wikipedia-ja",
    "https://dumps.wikimedia.org/jawiki/latest/jawiki-latest-pages-articles.xml.bz2",
    true,
  ]]);
});
