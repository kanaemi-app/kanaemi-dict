import { assertEquals, assertRejects } from "@std/assert";
import { decodeBase64 } from "@std/encoding/base64";
import {
  bzip2Text,
  categoriesOf,
  dumpPages,
  isDisambiguation,
  type PageHeader,
} from "./mediawiki.ts";
import { bzip2File, dump, page } from "./mediawiki_test_helper.ts";

async function collect<T>(items: AsyncIterable<T>): Promise<T[]> {
  const out = [];
  for await (const item of items) out.push(item);
  return out;
}

async function* chunks(s: string, size: number): AsyncGenerator<string> {
  for (let i = 0; i < s.length; i += size) yield await Promise.resolve(s.slice(i, i + size));
}

const XML = dump([
  page(1, 0, "", "東京で&lt;b&gt;雪&lt;/b&gt;が降った。", "東京 &amp; 雪"),
  page(2, 1, "", "ノート"),
  page(3, 0, '<redirect title="x" />', "#転送 [[東京]]"),
]);

Deno.test("every page comes with its header and its text, references decoded", async () => {
  assertEquals(await collect(dumpPages(chunks(XML, XML.length))), [
    { ns: "0", id: "1", title: "東京 & 雪", redirect: false, text: "東京で<b>雪</b>が降った。" },
    { ns: "1", id: "2", title: "t2", redirect: false, text: "ノート" },
    { ns: "0", id: "3", title: "t3", redirect: true, text: "#転送 [[東京]]" },
  ]);
});

Deno.test("a page split across chunks reads the same", async () => {
  assertEquals(
    await collect(dumpPages(chunks(XML, 7))),
    await collect(dumpPages(chunks(XML, XML.length))),
  );
});

Deno.test("only the pages whose header is wanted come out", async () => {
  const asked: PageHeader[] = [];

  const pages = await collect(dumpPages(chunks(XML, 7), (header) => {
    asked.push(header);
    return header.ns === "0" && !header.redirect;
  }));

  assertEquals(pages.map((p) => p.id), ["1"]);
  assertEquals(asked.map((h) => [h.id, h.ns, h.redirect]), [
    ["1", "0", false],
    ["2", "1", false],
    ["3", "0", true],
  ]);
});

Deno.test("categories come in order, sort keys and comments left out", () => {
  assertEquals(
    categoriesOf(
      "本文\n[[Category:日本の都市]]\n[[カテゴリ: 東京都 |とうきょう]]<!-- [[Category:隠し]] -->",
    ),
    ["日本の都市", "東京都"],
  );
});

Deno.test("a disambiguation page is told by its template or its category", () => {
  assertEquals(isDisambiguation("'''東京'''は…\n{{Aimai}}"), true);
  assertEquals(isDisambiguation("{{曖昧さ回避}}"), true);
  assertEquals(isDisambiguation("{{人名の曖昧さ回避|あ}}"), true);
  assertEquals(
    isDisambiguation("[[Category:同名の地名|とうきょう]]\n[[Category:地名の曖昧さ回避]]"),
    true,
  );
  assertEquals(isDisambiguation("'''東京都'''は日本の首都。{{Aimai2}}"), false);
});

Deno.test("a bzip2 file reads as its text", async () => {
  const path = await bzip2File(XML);

  assertEquals((await collect(bzip2Text(path))).join(""), XML);
});

Deno.test("every stream of a file compressed as concatenated bzip2 streams is read", async () => {
  // `<mediawiki><page>…id 1…</page>` and `<page>…id 2…</page></mediawiki>`,
  // each compressed on its own by bzip2 and concatenated.
  const path = await Deno.makeTempFile({ suffix: ".bz2" });
  await Deno.writeFile(
    path,
    decodeBase64(
      "QlpoOTFBWSZTWRZ12FoAAAaZ+QAA4AUmq93AIDoAQABBABAOQCAAVDVP1CA0GgGjTDUGpGygG1NpPUA0A9wIEVI2oDlsST7xAzNQTjQRkKhELdFWQMIGwNiwVEuCxyGMcMSJUhNUJJFJ0jgn4u5IpwoSAs67C0BCWmg5MUFZJlNZaEntBwAACBn5AADQBSar3cAgHgBAAEEAEQ5AIABUNU9GoA0aAAbU8oNTU2UAybSaAaANDDBkZDQmBa7HxNrkRFBsR4hEUiAW2CyeJBx6ChzwTiKIeNQZxFxBOpIqN6xoqyb8XckU4UJBoSe0HA==",
    ),
  );

  const pages = await collect(dumpPages(bzip2Text(path)));

  assertEquals(pages.map((p) => p.id), ["1", "2"]);
});

Deno.test("a file that is not bzip2 fails rather than reading as empty", async () => {
  const path = await Deno.makeTempFile({ suffix: ".bz2" });
  await Deno.writeTextFile(path, "not bzip2");

  await assertRejects(() => collect(bzip2Text(path)), Error, "lbzip2 failed");
});

Deno.test("leaving a file half read stops its decompression", async () => {
  const path = await bzip2File(XML);

  for await (const _ of dumpPages(bzip2Text(path))) break;
});

Deno.test("leaving a file half read after lbzip2 has exited is no error", async () => {
  const path = await bzip2File(XML);

  for await (const _ of bzip2Text(path)) {
    // lbzip2 writes a small file out in full and exits well within this.
    await new Promise((resolve) => setTimeout(resolve, 300));
    break;
  }
});
