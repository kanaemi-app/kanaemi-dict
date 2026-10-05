import { assertEquals } from "@std/assert";
import { TarStream, type TarStreamInput } from "@std/tar";
import { rurema } from "./rurema.ts";

async function tarGzOf(files: [string, string][]): Promise<Uint8Array> {
  const inputs: TarStreamInput[] = files.map(([path, body]) => {
    const bytes = new TextEncoder().encode(body);
    return { type: "file", path, size: bytes.length, readable: ReadableStream.from([bytes]) };
  });
  const stream = ReadableStream.from(inputs)
    .pipeThrough(new TarStream())
    .pipeThrough(new CompressionStream("gzip"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

Deno.test("markup, code and English lines are dropped", async () => {
  const text =
    '= class String\n\n文字列のクラスです。\n#@since 3.0\n--- length -> Integer\n@return 長さ\n  str.length\n//emlist[例][ruby]{\np "文字".length\n//}\n長さを返します。\nReturns the length.\n';
  const tar = await tarGzOf([["doctree-cc68abf/refm/api/src/_builtin/String", text]]);

  assertEquals(await rurema("rurema", tar), [{
    doc_id: "rurema:refm/api/src/_builtin/String",
    source_id: "rurema",
    text: "文字列のクラスです。\n長さを返します。",
  }]);
});

Deno.test("sample code blocks go with nested directives up to their own end", async () => {
  const text =
    "前の説明です。\n#@samplecode 例\n# すべて正の数か？\np [1].all?\n#@since 2.5.0\np [1].any?(Integer)\n#@end\n# 後の注釈\n#@end\n後の説明です。\n";
  const tar = await tarGzOf([["d/refm/api/src/a", text]]);

  assertEquals((await rurema("rurema", tar))[0].text, "前の説明です。\n後の説明です。");
});

Deno.test("references become the name they point at", async () => {
  const text = "[[c:String]] を返します。[[m:$!]] の別名です。[[lib:json]] を使います。\n";
  const tar = await tarGzOf([["d/refm/api/src/a", text]]);

  assertEquals(
    (await rurema("rurema", tar))[0].text,
    "String を返します。$! の別名です。json を使います。",
  );
});

Deno.test("indented descriptions stay", async () => {
  const text =
    ": *\n    空文字列を含む任意の文字列と一致します。\n: ?\n    任意の一文字と一致します。\n";
  const tar = await tarGzOf([["d/refm/api/src/a", text]]);

  assertEquals(
    (await rurema("rurema", tar))[0].text,
    "空文字列を含む任意の文字列と一致します。\n任意の一文字と一致します。",
  );
});

Deno.test("only API sources are read, in name order", async () => {
  const tar = await tarGzOf([
    ["d/refm/api/src/b", "二"],
    ["d/refm/doc/x", "文書"],
    ["d/refm/api/src/a", "一"],
    ["d/refm/api/src/c", "English only"],
  ]);

  assertEquals((await rurema("rurema", tar)).map((d) => d.doc_id), [
    "rurema:refm/api/src/a",
    "rurema:refm/api/src/b",
  ]);
});
