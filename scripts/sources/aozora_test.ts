import { assertEquals } from "@std/assert";
import { inHashOrder } from "../select.ts";
import { zipOf } from "../zip_test_helper.ts";
import { recordingFetch } from "../sources_test_helper.ts";
import { aozora, aozoraCandidates, source } from "./aozora.ts";

// Shift_JIS bytes for the test texts; TextEncoder only writes UTF-8.
function sjis(text: string): Uint8Array {
  const table = new Map<string, number[]>();
  const decoder = new TextDecoder("shift_jis");
  for (let hi = 0x81; hi <= 0xfc; hi++) {
    if (hi >= 0xa0 && hi <= 0xdf) continue;
    for (let lo = 0x40; lo <= 0xfc; lo++) {
      const ch = decoder.decode(new Uint8Array([hi, lo]));
      if (ch.length === 1 && ch !== "�" && !table.has(ch)) table.set(ch, [hi, lo]);
    }
  }
  const out: number[] = [];
  for (const ch of text) {
    const code = ch.charCodeAt(0);
    out.push(...(code < 0x80 ? [code] : table.get(ch) ?? [0x81, 0x48]));
  }
  return new Uint8Array(out);
}

const TOP = "aozorabunko_text-0984f7dc18a2270bdfd2ff99bf14fd1b3ebb4f65";

type Work = { id: string; kana?: string; text?: string };

const stem = (id: string) => `${Number(id)}_ruby_${Number(id) + 100}`;

/** The index zip and the bulk text archive holding `works`; a work without text is left out of the archive. */
async function sources(works: Work[]): Promise<[index: Uint8Array, archive: Uint8Array]> {
  const csv = [
    "作品ID,作品名,文字遣い種別,人物ID,テキストファイルURL",
    ...works.map(({ id, kana = "新字新仮名" }) =>
      `"${id}","題","${kana}","1","https://www.aozora.gr.jp/cards/000001/files/${stem(id)}.zip"`
    ),
  ].join("\r\n");
  const index = await zipOf([["list_person_all_extended_utf8.csv", `﻿${csv}`]]);
  const archive = await zipOf([
    [`${TOP}/README.md`, "# aozorabunko_text"],
    ...works.flatMap(({ id, text }): [string, Uint8Array][] =>
      text === undefined
        ? []
        : [[`${TOP}/cards/000001/files/${stem(id)}/${stem(id)}.txt`, sjis(text)]]
    ),
  ]);
  return [index, archive];
}

async function textOf(text: string): Promise<string | undefined> {
  return (await aozora("aozora-text", ...await sources([{ id: "000001", text }])))[0]?.text;
}

const LEGEND = "題名\r\n著者\r\n\r\n" +
  "-------------------------------------------------------\r\n" +
  "【テキスト中に現れる記号について】\r\n《》：ルビ\r\n" +
  "-------------------------------------------------------\r\n";

Deno.test("the body runs from after the legend to before the colophon", async () => {
  const text = `${LEGEND}　本文の一行目。\r\n二行目\r\n\r\n底本：「全集」\r\n入力：誰か\r\n`;

  assertEquals(await aozora("aozora-text", ...await sources([{ id: "000001", text }])), [{
    doc_id: "aozora:000001",
    source_id: "aozora-text",
    text: "本文の一行目。\n二行目",
  }]);
});

Deno.test("a colophon with a half-width colon also ends the body", async () => {
  assertEquals(await textOf(`${LEGEND}本文\r\n底本:「全集」\r\n`), "本文");
});

Deno.test("other colophon forms and an inputter's note also end the body", async () => {
  assertEquals(await textOf(`${LEGEND}本文\r\n底本「全集」第一巻\r\n入力：誰か\r\n`), "本文");
  assertEquals(await textOf(`${LEGEND}本文\r\n入力者注　原文の誤記を正した\r\n`), "本文");
});

Deno.test("without a legend the body starts on the third line", async () => {
  assertEquals(await textOf("題名\r\n著者\r\n本文\r\n"), "本文");
});

Deno.test("ruby leaves only the base text", async () => {
  assertEquals(
    await textOf(
      `${LEGEND}｜東京駅《とうきょうえき》と日本語《にほんご》と処々《ところどころ》\r\n`,
    ),
    "東京駅と日本語と処々",
  );
});

Deno.test("implicit ruby covers only the kanji run", async () => {
  assertEquals(await textOf(`${LEGEND}これは漢字《かんじ》です\r\n`), "これは漢字です");
});

Deno.test("annotations and stray ruby bars are removed", async () => {
  assertEquals(
    await textOf(
      `${LEGEND}※［＃「木＋吶のつくり」、第3水準1-85-54］字［＃「字」に傍点］｜残り\r\n`,
    ),
    "字残り",
  );
});

Deno.test("ruby on an annotated character goes with the annotation", async () => {
  assertEquals(
    await textOf(
      `${LEGEND}その｜※［＃「てへん＋劣」、第3水準1-84-77］《むし》る音と※［＃「骨＋去」、第4水準2-93-28］《こつ》と〇《れい》\r\n`,
    ),
    "そのる音とと〇",
  );
});

Deno.test("ruby goes whatever its base is", async () => {
  assertEquals(
    await textOf(`${LEGEND}抑ゝ《そもそも》の○○《なになに》会と Woman's《ウーマンス》 revenge\r\n`),
    "抑ゝの○○会と Woman's revenge",
  );
});

Deno.test("a work without body lines is no document", async () => {
  assertEquals(await textOf(`${LEGEND}\r\n底本：「全集」\r\n`), undefined);
});

Deno.test("only the works the index selects are read, and those missing from the archive are skipped", async () => {
  const docs = await aozora(
    "aozora-text",
    ...await sources([
      { id: "000001", text: `${LEGEND}一つ目\r\n` },
      { id: "000002" },
      { id: "000003", kana: "旧字旧仮名", text: `${LEGEND}三つ目\r\n` },
    ]),
  );

  assertEquals(docs.map((d) => d.doc_id), ["aozora:000001"]);
});

Deno.test("works are taken in doc ID hash order until their characters reach the budget", async () => {
  const ids = ["000001", "000002", "000003", "000004"];
  const [index, archive] = await sources([
    ...ids.map((id) => ({ id, text: `${LEGEND}あいう\r\n` })),
    { id: "000005", text: `${LEGEND}\r\n` },
  ]);
  const inOrder = await inHashOrder(ids.map((id) => `aozora:${id}`), (docId) => docId);

  const docs = await aozora("aozora-text", index, archive, 4);

  assertEquals(docs.map((d) => d.doc_id), inOrder.slice(0, 2));
});

const HEADER = "作品ID,作品名,文字遣い種別,人物ID,テキストファイルURL";

function row(id: string, kana: string, url: string, person = "1"): string {
  return `"${id}","題","${kana}","${person}","${url}"`;
}

Deno.test("aozora candidates are modern-kana works with an Aozora zip, once each, in doc ID hash order", async () => {
  const zip = (id: string) => `https://www.aozora.gr.jp/cards/000001/files/${id}_ruby_1.zip`;
  const csv = [
    "\uFEFF" + HEADER,
    row("000001", "新字新仮名", zip("1")),
    row("000001", "新字新仮名", zip("1"), "2"),
    row("000002", "旧字旧仮名", zip("2")),
    row("000003", "新字旧仮名", zip("3")),
    row("000004", "新字新仮名", "https://example.com/4.zip"),
    row("000005", "新字新仮名", "https://www.aozora.gr.jp/cards/000001/files/5.html"),
    row("000006", "新字新仮名", ""),
    row("000007", "新字新仮名", zip("7")),
    row("000008", "新字新仮名", zip("8")),
  ].join("\r\n");

  const candidates = await aozoraCandidates(csv);

  const expected = await inHashOrder(
    ["000001", "000007", "000008"].map((workId) => ({
      docId: `aozora:${workId}`,
      workId,
      url: zip(String(Number(workId))),
    })),
    (c) => c.docId,
  );
  assertEquals(candidates, expected);
});

Deno.test("fetching takes the index, then the text archive as a large download", async () => {
  const { ctx, fetched } = recordingFetch();

  await source.fetch(ctx);

  assertEquals(fetched.map(([id, , options]) => [id, options.large ?? false]), [
    ["aozora-index", false],
    ["aozora-text", true],
  ]);
});
