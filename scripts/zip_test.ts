import { assertEquals } from "@std/assert";
import { zipEntries } from "./zip.ts";
import { zipOf } from "./zip_test_helper.ts";

Deno.test("zip entries come in archive order with their bytes", async () => {
  const zip = await zipOf([["b.txt", "二"], ["a/a.txt", "一"]]);

  const entries = [];
  for await (const entry of zipEntries(zip)) {
    entries.push([entry.name, new TextDecoder().decode(await entry.bytes())]);
  }

  assertEquals(entries, [["b.txt", "二"], ["a/a.txt", "一"]]);
});

Deno.test("directory entries are skipped", async () => {
  const zip = await zipOf([["d/", ""], ["d/a.txt", "一"]]);

  const names = [];
  for await (const entry of zipEntries(zip)) names.push(entry.name);

  assertEquals(names, ["d/a.txt"]);
});
