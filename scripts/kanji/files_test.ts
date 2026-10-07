import { assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import type { FetchOptions } from "../raw/fetch.ts";
import { appendRecord, type Record } from "../raw/manifest.ts";
import { RawStore } from "../raw/store.ts";
import { zipOf } from "../zip_test_helper.ts";
import { extractKanjiFile, fetchKanji, KANJI_FILES, type KanjiFile } from "./files.ts";

const MOZC: KanjiFile = {
  sourceId: "mozc-single-kanji",
  url: "",
  name: "single_kanji.tsv",
  zipped: false,
};
const UNIHAN: KanjiFile = {
  sourceId: "unicode-unihan",
  url: "",
  name: "Unihan_Readings.txt",
  zipped: true,
};

async function addRecord(dir: string, sourceId: string, url: string, bytes: Uint8Array) {
  await appendRecord(join(dir, "manifest.jsonl"), {
    source_id: sourceId,
    url,
    sha256: await new RawStore(dir).put(bytes),
    bytes: bytes.length,
    retrieved_at: "2026-10-07T00:00:00+0900",
  });
}

Deno.test("every kanji file is fetched at its pinned URL", async () => {
  const fetched: [string, string][] = [];
  const get = (sourceId: string, url: string, _options?: FetchOptions) => {
    fetched.push([sourceId, url]);
    const record: Record = { source_id: sourceId, url, sha256: "", bytes: 0, retrieved_at: "" };
    return Promise.resolve(record);
  };

  await fetchKanji(get);

  assertEquals(fetched, KANJI_FILES.map((f) => [f.sourceId, f.url]));
});

Deno.test("a file fetched as it is is written out as stored", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = join(await Deno.makeTempDir(), "kanji");
  await addRecord(
    raw,
    MOZC.sourceId,
    "https://m/single_kanji.tsv",
    new TextEncoder().encode("すで\t既已\n"),
  );

  const out = await extractKanjiFile(raw, MOZC, outDir);

  assertEquals(out, join(outDir, "single_kanji.tsv"));
  assertEquals(await Deno.readTextFile(out), "すで\t既已\n");
});

Deno.test("a zipped file comes out of the zip of the latest record", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = await Deno.makeTempDir();
  await addRecord(
    raw,
    UNIHAN.sourceId,
    "https://u/17.zip",
    await zipOf([["Unihan_Readings.txt", "古い"]]),
  );
  await addRecord(
    raw,
    UNIHAN.sourceId,
    "https://u/18.zip",
    await zipOf([["Unihan_IRGSources.txt", "x"], [
      "Unihan_Readings.txt",
      "U+9AD9\tkJapanese\tコウ\n",
    ]]),
  );

  const out = await extractKanjiFile(raw, UNIHAN, outDir);

  assertEquals(await Deno.readTextFile(out), "U+9AD9\tkJapanese\tコウ\n");
  assertEquals([...Deno.readDirSync(outDir)].map((e) => e.name), ["Unihan_Readings.txt"]);
});

Deno.test("without a record of the file's source there is nothing to extract", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = await Deno.makeTempDir();

  await assertRejects(() => extractKanjiFile(raw, MOZC, outDir), Error, "no mozc-single-kanji");
});

Deno.test("a zip without the file is an error naming its URL", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = await Deno.makeTempDir();
  await addRecord(raw, UNIHAN.sourceId, "https://u/18.zip", await zipOf([["a", "b"]]));

  await assertRejects(
    () => extractKanjiFile(raw, UNIHAN, outDir),
    Error,
    "no Unihan_Readings.txt in https://u/18.zip",
  );
});
