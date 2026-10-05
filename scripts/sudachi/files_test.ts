import { assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import type { FetchOptions } from "../raw/fetch.ts";
import { appendRecord, type Record } from "../raw/manifest.ts";
import { RawStore } from "../raw/store.ts";
import { zipOf } from "../zip_test_helper.ts";
import { extractSudachiFile, fetchSudachi, SUDACHI_FILES, type SudachiFile } from "./files.ts";

const FULL: SudachiFile = { sourceId: "sudachidict-full", url: "", name: "system_full.dic" };
const SMALL_LEX: SudachiFile = {
  sourceId: "sudachidict-small-lex",
  url: "",
  name: "small_lex.csv",
};

async function addRecord(dir: string, sourceId: string, url: string, zip: Uint8Array) {
  await appendRecord(join(dir, "manifest.jsonl"), {
    source_id: sourceId,
    url,
    sha256: await new RawStore(dir).put(zip),
    bytes: zip.length,
    retrieved_at: "2026-10-04T00:00:00+0900",
  });
}

Deno.test("every SudachiDict file is fetched as a large download", async () => {
  const fetched: [string, string, FetchOptions | undefined][] = [];
  const get = (sourceId: string, url: string, options?: FetchOptions) => {
    fetched.push([sourceId, url, options]);
    const record: Record = { source_id: sourceId, url, sha256: "", bytes: 0, retrieved_at: "" };
    return Promise.resolve(record);
  };

  await fetchSudachi(get);

  assertEquals(fetched, SUDACHI_FILES.map((f) => [f.sourceId, f.url, { large: true }]));
});

Deno.test("the system dictionary of the latest record is written out", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = join(await Deno.makeTempDir(), "sudachi");
  await addRecord(
    raw,
    FULL.sourceId,
    "https://s/old.zip",
    await zipOf([["d/system_full.dic", "古い"]]),
  );
  await addRecord(
    raw,
    FULL.sourceId,
    "https://s/v1.zip",
    await zipOf([["d/LEGAL", "x"], ["d/system_full.dic", "新しい"]]),
  );

  const out = await extractSudachiFile(raw, FULL, outDir);

  assertEquals(out, join(outDir, "system_full.dic"));
  assertEquals(await Deno.readTextFile(out), "新しい");
  assertEquals([...Deno.readDirSync(outDir)].map((e) => e.name), ["system_full.dic"]);
});

Deno.test("the UniDic lexicon comes out of the SudachiDict small lexicon record", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = await Deno.makeTempDir();
  const zip = await zipOf([["small_lex.csv", "IndexForm,Headword\n私,私\n"]]);
  await addRecord(raw, SMALL_LEX.sourceId, "https://s/small_lex.zip", zip);

  const out = await extractSudachiFile(raw, SMALL_LEX, outDir);

  assertEquals(await Deno.readTextFile(out), "IndexForm,Headword\n私,私\n");
});

Deno.test("without a record of the file's source there is nothing to extract", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = await Deno.makeTempDir();
  await addRecord(raw, SMALL_LEX.sourceId, "https://s/small_lex.zip", await zipOf([["a", "b"]]));

  await assertRejects(() => extractSudachiFile(raw, FULL, outDir), Error, "no sudachidict-full");
});

Deno.test("a zip without the file is an error naming its URL", async () => {
  const raw = await Deno.makeTempDir();
  const outDir = await Deno.makeTempDir();
  await addRecord(raw, FULL.sourceId, "https://s/v1.zip", await zipOf([["d/LEGAL", "x"]]));

  await assertRejects(
    () => extractSudachiFile(raw, FULL, outDir),
    Error,
    "no system_full.dic in https://s/v1.zip",
  );
});
