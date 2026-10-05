import { assertEquals, assertThrows } from "@std/assert";
import { firstPageOfYear, firstPageQuery, yearFiles } from "./year.ts";

Deno.test("the first page of a year is asked of the creation log from its first moment", () => {
  const url = new URL(firstPageQuery(2027));

  assertEquals(url.searchParams.get("list"), "logevents");
  assertEquals(url.searchParams.get("letype"), "create");
  assertEquals(url.searchParams.get("lenamespace"), "0");
  assertEquals(url.searchParams.get("lestart"), "2027-01-01T00:00:00Z");
  assertEquals(url.searchParams.get("ledir"), "newer");
});

Deno.test("the first page of a year is the page ID the first creation names", () => {
  const answer = {
    query: { logevents: [{ pageid: 5185032, timestamp: "2026-01-01T00:00:22Z" }] },
  };

  assertEquals(firstPageOfYear(2026, answer), 5185032);
});

Deno.test("a year the creation log does not cover from its first day has no first page", () => {
  const answer = {
    query: { logevents: [{ logpage: 3807530, timestamp: "2018-06-27T23:16:06Z" }] },
  };

  assertThrows(() => firstPageOfYear(2018, answer), Error, "2018-01-01");
});

Deno.test("the first page is the ID the page had when created, even if it is gone now", () => {
  const answer = {
    query: { logevents: [{ pageid: 0, logpage: 5185032, timestamp: "2026-01-01T00:00:22Z" }] },
  };

  assertEquals(firstPageOfYear(2026, answer), 5185032);
});

Deno.test("a year with no article created yet has no first page", () => {
  assertThrows(() => firstPageOfYear(2027, { query: { logevents: [] } }));
  assertThrows(
    () =>
      firstPageOfYear(2026, {
        query: { logevents: [{ pageid: 1, timestamp: "2027-01-01T00:00:00Z" }] },
      }),
    Error,
    "2027",
  );
});

Deno.test("a year's dictionary is set up with its label, every category and its first page", () => {
  assertEquals(yearFiles(2027, 5400000), {
    "label.txt": "2027年の新語\n",
    "wikipedia.txt": ".\n",
    "wikipedia-since.txt": "5400000\n",
  });
});
