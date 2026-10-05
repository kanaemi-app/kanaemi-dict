import { assertEquals } from "@std/assert";
import { recordingFetch } from "../sources_test_helper.ts";
import { fineweb2, source } from "./fineweb.ts";
import { parquetFile } from "./fineweb_test_helper.ts";

Deno.test("each row is a document of its Japanese lines", async () => {
  const path = await parquetFile([{
    id: "<urn:uuid:0001>",
    text: "ｶﾀｶﾅの見出し\nEnglish only\n\n　本文の一行目。 \nＡＢＣと日本語",
  }]);

  assertEquals(await fineweb2("fineweb2-jpn", path), [{
    doc_id: "fineweb2:0001",
    source_id: "fineweb2-jpn",
    text: "カタカナの見出し\n本文の一行目。\nABCと日本語",
  }]);
});

Deno.test("rows come in file order, and a row without Japanese lines is no document", async () => {
  const path = await parquetFile([
    { id: "b", text: "二つ目" },
    { id: "x", text: "No Japanese here." },
    { id: "a", text: "一つ目" },
  ]);

  assertEquals((await fineweb2("fineweb2-jpn", path)).map((d) => d.doc_id), [
    "fineweb2:b",
    "fineweb2:a",
  ]);
});

Deno.test("every row group of a file is read", async () => {
  const rows = ["一", "二", "三", "四", "五"].map((text, i) => ({ id: `r${i}`, text }));
  const path = await parquetFile(rows, 2);

  assertEquals((await fineweb2("fineweb2-jpn", path)).map((d) => d.text), [
    "一",
    "二",
    "三",
    "四",
    "五",
  ]);
});

Deno.test("with a budget, rows are taken in file order until their characters reach it", async () => {
  const rows = ["一二", "三四五", "六", "七八"].map((text, i) => ({ id: `r${i}`, text }));
  const path = await parquetFile(rows, 1);

  assertEquals((await fineweb2("fineweb2-jpn-train", path, 4)).map((d) => d.doc_id), [
    "fineweb2:r0",
    "fineweb2:r1",
  ]);
});

Deno.test("fetching takes the pinned test and train files as large downloads", async () => {
  const { ctx, fetched } = recordingFetch();

  await source.fetch(ctx);

  const base =
    "/datasets/HuggingFaceFW/fineweb-2/resolve/af9c13333eb981300149d5ca60a8e9d659b276b9/data/jpn_Jpan";
  assertEquals(fetched.map(([id, url, options]) => [id, new URL(url).pathname, options.large]), [
    ["fineweb2-jpn", `${base}/test/000_00000.parquet`, true],
    ["fineweb2-jpn-train", `${base}/train/000_00000.parquet`, true],
  ]);
});
