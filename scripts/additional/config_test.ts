import { assertEquals } from "@std/assert";
import { join } from "@std/path";
import { readDictionaries } from "./config.ts";

Deno.test("each directory is a dictionary, with its patterns and first page when it has them", async () => {
  const dir = await Deno.makeTempDir();
  await Deno.mkdir(join(dir, "railway"));
  await Deno.writeTextFile(join(dir, "railway", "label.txt"), "鉄道\n");
  await Deno.writeTextFile(join(dir, "railway", "wikipedia.txt"), "鉄道\n\n駅$\n");
  await Deno.mkdir(join(dir, "2026"));
  await Deno.writeTextFile(join(dir, "2026", "label.txt"), "2026年の新語\n");
  await Deno.writeTextFile(join(dir, "2026", "wikipedia.txt"), ".\n");
  await Deno.writeTextFile(join(dir, "2026", "wikipedia-since.txt"), "5185032\n");
  await Deno.mkdir(join(dir, "place"));
  await Deno.writeTextFile(join(dir, "place", "label.txt"), "地名\n");

  assertEquals(await readDictionaries(dir), [
    { name: "2026", patterns: [/./], since: 5185032 },
    { name: "place" },
    { name: "railway", patterns: [/鉄道/, /駅$/] },
  ]);
});
