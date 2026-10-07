import { assert, assertEquals } from "@std/assert";
import { MOZC_TABLES } from "./mozc.ts";

Deno.test("every table is pinned to one Mozc commit and has its own source ID", () => {
  const commits = new Set(MOZC_TABLES.map((t) => t.url.split("/")[5]));
  assertEquals(commits.size, 1);
  assert([...commits][0].match(/^[0-9a-f]{40}$/));
  assertEquals(new Set(MOZC_TABLES.map((t) => t.sourceId)).size, MOZC_TABLES.length);
});

Deno.test("the emoji dictionary takes both emoji tables", () => {
  assertEquals(
    MOZC_TABLES.filter((t) => t.dictionary === "emoji").map((t) => t.name),
    ["emoji_data.tsv", "manual_emoji_data.tsv"],
  );
});
