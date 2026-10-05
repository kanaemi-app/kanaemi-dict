import { assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import { appendRecord, loadManifest } from "../raw/manifest.ts";
import { RawStore } from "../raw/store.ts";
import { zipOf } from "../zip_test_helper.ts";
import { parquetOf } from "../sources/fineweb_test_helper.ts";
import {
  DuplicateIdError,
  extractAll,
  SourceError,
  UnknownSourceError,
  writeJsonl,
} from "./extract.ts";

const encode = (s: string) => new TextEncoder().encode(s);

async function setup() {
  const dir = await Deno.makeTempDir();
  const store = new RawStore(dir);
  const add = async (source_id: string, url: string, data: string | Uint8Array) => {
    const bytes = typeof data === "string" ? encode(data) : data;
    await appendRecord(join(dir, "manifest.jsonl"), {
      source_id,
      url,
      sha256: await store.put(bytes),
      bytes: bytes.length,
      retrieved_at: "2026-10-02T11:07:41+0900",
    });
  };
  const records = () => loadManifest(join(dir, "manifest.jsonl"));
  return { dir, store, add, records };
}

const AOZORA_INDEX = zipOf([[
  "list_person_all_extended_utf8.csv",
  "作品ID,作品名,文字遣い種別,人物ID,テキストファイルURL\r\n" +
  '"000001","題","新字新仮名","1","https://www.aozora.gr.jp/cards/000001/files/1_ruby_1.zip"\r\n',
]]);

const AOZORA_TEXT = zipOf([[
  "aozorabunko_text-0984f7dc18a2270bdfd2ff99bf14fd1b3ebb4f65/cards/000001/files/1_ruby_1/1_ruby_1.txt",
  "Title\r\nAuthor\r\nBody\r\n",
]]);

Deno.test("documents come from every build source, sorted by ID", async () => {
  const { store, add, records } = await setup();
  await add(
    "fineweb2-jpn",
    "https://f/0.parquet",
    parquetOf([{ id: "b", text: "二つ目" }, { id: "a", text: "一つ目" }]),
  );
  await add("aozora-index", "https://a/index.zip", await AOZORA_INDEX);
  await add("aozora-text", "https://a/text.zip", await AOZORA_TEXT);

  const docs = await extractAll(store, await records());

  assertEquals(docs.map((d) => [d.doc_id, d.source_id]), [
    ["aozora:000001", "aozora-text"],
    ["fineweb2:a", "fineweb2-jpn"],
    ["fineweb2:b", "fineweb2-jpn"],
  ]);
});

Deno.test("the Aozora Bunko text archive cannot be read without its index", async () => {
  const { store, add, records } = await setup();
  await add("aozora-text", "https://a/text.zip", await AOZORA_TEXT);

  const err = await assertRejects(async () => extractAll(store, await records()), SourceError);

  assertEquals(err.sourceId, "aozora-text");
});

Deno.test("a record no source owns is an error naming its source ID", async () => {
  const { store, add, records } = await setup();
  await add("renamed-source", "https://r", "x");

  const err = await assertRejects(
    async () => extractAll(store, await records()),
    UnknownSourceError,
  );

  assertEquals(err.sourceId, "renamed-source");
});

Deno.test("a record read only along with another gives no documents on its own", async () => {
  const { store, add, records } = await setup();
  await add("aozora-index", "https://a/index.zip", await AOZORA_INDEX);

  assertEquals(await extractAll(store, await records()), []);
});

Deno.test("an unreadable source names itself", async () => {
  const { store, add, records } = await setup();
  await add("fineweb2-jpn", "https://f/0.parquet", "not parquet");

  const err = await assertRejects(async () => extractAll(store, await records()), SourceError);

  assertEquals(err.sourceId, "fineweb2-jpn");
});

Deno.test("two documents with one ID are an error", async () => {
  const { store, add, records } = await setup();
  await add(
    "fineweb2-jpn",
    "https://f/0.parquet",
    parquetOf([{ id: "a", text: "一" }, { id: "a", text: "二" }]),
  );

  await assertRejects(async () => extractAll(store, await records()), DuplicateIdError);
});

Deno.test("documents round-trip through JSON Lines", async () => {
  const dir = await Deno.makeTempDir();
  const path = join(dir, "build/docs.jsonl");
  const docs = [{ doc_id: "a", source_id: "s", text: "一行目\n二行目" }];

  await writeJsonl(path, docs);

  const lines = (await Deno.readTextFile(path)).trimEnd().split("\n");
  assertEquals(lines.map((l) => JSON.parse(l)), docs);
});
