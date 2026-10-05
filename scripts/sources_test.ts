import { assert, assertEquals } from "@std/assert";
import { matchesId, ownerOf, type Source, SOURCES } from "./sources.ts";

function stub(name: string, ids: string[], companions?: string[]): Source {
  return {
    name,
    ids,
    companions,
    fetch: () => Promise.resolve(),
    documents: () => Promise.resolve([]),
  };
}

Deno.test("every source has a name of its own", () => {
  const names = SOURCES.map((s) => s.name);

  assertEquals(new Set(names).size, names.length);
});

Deno.test("every source declares the source IDs of its documents", () => {
  for (const source of SOURCES) assert(source.ids.length > 0, source.name);
});

Deno.test("no two source ID patterns of the list overlap", () => {
  const patterns = SOURCES.flatMap((s) => [...s.ids, ...s.companions ?? []]);

  for (const [i, a] of patterns.entries()) {
    for (const b of patterns.slice(i + 1)) {
      assert(!matchesId(a, b) && !matchesId(b, a), `${a} and ${b} overlap`);
    }
  }
});

Deno.test("a pattern matches its exact ID, or every ID under a prefix ending in a colon", () => {
  assert(matchesId("rurema", "rurema"));
  assert(!matchesId("rurema", "rurema:x"));
  assert(matchesId("day:", "day:20260101"));
  assert(!matchesId("day:", "days:20260101"));
});

Deno.test("a record is owned by the source that declares its ID, as documents or as a companion", () => {
  const one = stub("one", ["one"]);
  const days = stub("days", ["day:"], ["day-index"]);
  const sources = [one, days];

  assertEquals(ownerOf("one", sources), { source: one, companion: false });
  assertEquals(ownerOf("day:20260101", sources), { source: days, companion: false });
  assertEquals(ownerOf("day-index", sources), { source: days, companion: true });
  assertEquals(ownerOf("other", sources), undefined);
});
