import { assertEquals } from "@std/assert";
import { laws } from "./law.ts";
import { zipOf } from "../zip_test_helper.ts";

const LAW = `<?xml version="1.0" encoding="UTF-8"?>
<Law><LawNum>令和元年法律第一号</LawNum>
<LawBody><LawTitle>テスト法</LawTitle>
<MainProvision><Article><ArticleCaption>（目的）</ArticleCaption>
<Paragraph><ParagraphSentence>
<Sentence Num="1">この法律は、<Ruby>漢<Rt>かん</Rt></Ruby>字を定める。</Sentence>
<Sentence Num="2">
  </Sentence>
<Sentence>Ａ＆Ｂ&amp;C</Sentence>
</ParagraphSentence></Paragraph></Article></MainProvision>
</LawBody></Law>`;

function lawOf(body: string): string {
  return `<Law><LawBody><MainProvision>${body}</MainProvision></LawBody></Law>`;
}

async function idsAndTexts(files: [string, string][]): Promise<[string, string][]> {
  return (await laws("egov-all-xml", await zipOf(files))).map((d) => [d.doc_id, d.text]);
}

Deno.test("each sentence of the provisions is a line, ruby readings left out", async () => {
  const dir = "123AC0000000001_20200101_000000000000000";
  const docs = await laws("egov-all-xml", await zipOf([[`${dir}/${dir}.xml`, LAW]]));

  assertEquals(docs, [{
    doc_id: `law:${dir}`,
    source_id: "egov-all-xml",
    text: "この法律は、漢字を定める。\nA＆B&C",
  }]);
});

Deno.test("non-XML files and laws without sentences are skipped", async () => {
  const empty = "<Law><LawBody><LawTitle>空</LawTitle></LawBody></Law>";

  assertEquals(
    (await idsAndTexts([["a/a.xml", empty], ["b/readme.txt", "x"], ["c/c.xml", LAW]])).map((
      [id],
    ) => id),
    ["law:c"],
  );
});

Deno.test("a nested sentence stays inside the outer one, in order", async () => {
  const law = lawOf(
    "<Sentence>改正規定中「<QuoteStruct>\n    <Sentence>2，000円</Sentence>\n  </QuoteStruct>」を削る。</Sentence><Sentence>次の文。</Sentence>",
  );

  assertEquals(await idsAndTexts([["a/a.xml", law]]), [[
    "law:a",
    "改正規定中「2，000円」を削る。\n次の文。",
  ]]);
});

Deno.test("line breaks inside a sentence go with their surrounding spaces", async () => {
  const law = lawOf("<Sentence>毎月の\n      十二日を含む週</Sentence>");

  assertEquals(await idsAndTexts([["a/a.xml", law]]), [["law:a", "毎月の十二日を含む週"]]);
});

Deno.test("only the main and supplementary provisions are read", async () => {
  const law = "<Law><EnactStatement><Sentence>制定文</Sentence></EnactStatement><LawBody>" +
    "<MainProvision><Sentence>本則</Sentence></MainProvision>" +
    "<SupplProvision><Sentence>附則</Sentence></SupplProvision>" +
    "<AppdxTable><Sentence>別表</Sentence></AppdxTable>" +
    "</LawBody></Law>";

  assertEquals(await idsAndTexts([["a/a.xml", law]]), [["law:a", "本則\n附則"]]);
});

Deno.test("a law with a revision yet to take effect is read once, as in force now", async () => {
  const now = "123AC0000000001_20200101_000000000000000";
  const later = "123AC0000000001_20990101_999AC0000000001";
  const other = "456AC0000000002_20990101_999AC0000000001";
  const law = lawOf("<Sentence>あいう</Sentence>");

  const ids = (await idsAndTexts([
    [`${later}/${later}.xml`, law],
    [`${now}/${now}.xml`, law],
    [`${other}/${other}.xml`, law],
  ])).map(([id]) => id).sort();

  assertEquals(ids, [`law:${now}`, `law:${other}`]);
});

Deno.test("laws are taken in doc ID hash order until their characters reach the budget", async () => {
  const law = lawOf("<Sentence>あいう</Sentence>");
  const zip = await zipOf([["a/a.xml", law], ["b/b.xml", law], ["c/c.xml", law]]);

  const docs = await laws("egov-all-xml", zip, 4);

  assertEquals(docs.map((d) => d.doc_id).sort(), ["law:a", "law:b"]);
});
