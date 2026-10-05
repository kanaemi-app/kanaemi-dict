import { assertEquals } from "@std/assert";
import { documentOf } from "./document.ts";

Deno.test("a document's text is its lines joined by new lines", () => {
  assertEquals(documentOf("a:1", "a", ["一行目", "二行目"]), {
    doc_id: "a:1",
    source_id: "a",
    text: "一行目\n二行目",
  });
});

Deno.test("no lines make no document", () => {
  assertEquals(documentOf("a:1", "a", []), undefined);
});
