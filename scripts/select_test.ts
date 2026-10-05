import { assertEquals } from "@std/assert";
import { HashBudget, inHashOrder, keyHash, takeUntilBudget } from "./select.ts";
import { sha256Hex } from "./raw/store.ts";

Deno.test("items are ordered by the SHA-256 of their keys", async () => {
  const keys = ["a", "b", "c", "d"];
  const hashes = new Map(
    await Promise.all(
      keys.map(async (k) => [k, await sha256Hex(new TextEncoder().encode(k))] as const),
    ),
  );

  const ordered = await inHashOrder(keys, (k) => k);

  assertEquals(ordered, [...keys].sort((a, b) => hashes.get(a)! < hashes.get(b)! ? -1 : 1));
});

Deno.test("documents are taken in order until their characters reach the budget", async () => {
  const sizes: { [id: string]: number } = { a: 3, b: 4, c: 5, d: 1 };
  const taken: string[] = [];

  const chars = await takeUntilBudget(["a", "b", "c", "d"], 7, async (id) => {
    taken.push(id);
    return await Promise.resolve(sizes[id]);
  });

  assertEquals(taken, ["a", "b"]);
  assertEquals(chars, 7);
});

Deno.test("a document that cannot be read counts for nothing", async () => {
  const taken: string[] = [];

  await takeUntilBudget(["a", "b", "c"], 2, async (id) => {
    taken.push(id);
    return await Promise.resolve(id === "a" ? undefined : 1);
  });

  assertEquals(taken, ["a", "b", "c"]);
});

Deno.test("offered in any order, the kept items are the hash-order prefix that reaches the budget", () => {
  const sizes: { [id: string]: number } = { a: 3, b: 4, c: 5, d: 1, e: 2 };
  const ordered = Object.keys(sizes).sort((x, y) => keyHash(x) < keyHash(y) ? -1 : 1);
  const expected: string[] = [];
  let sum = 0;
  for (const id of ordered) {
    if (sum >= 6) break;
    expected.push(id);
    sum += sizes[id];
  }
  const kept = new HashBudget<string>(6);

  for (const id of ["e", "c", "a", "d", "b"]) kept.offer(keyHash(id), sizes[id], id);

  assertEquals(kept.items(), expected);
});

Deno.test("an item past the kept prefix is not wanted once the budget is reached", () => {
  const [first, second] = ["a", "b", "c"].sort((x, y) => keyHash(x) < keyHash(y) ? -1 : 1);
  const kept = new HashBudget<string>(2);

  assertEquals(kept.wants(keyHash(second)), true);
  kept.offer(keyHash(first), 2, first);

  assertEquals(kept.wants(keyHash(second)), false);
});

Deno.test("with no budget, nothing is wanted or kept", () => {
  const kept = new HashBudget<string>(0);

  assertEquals(kept.wants(keyHash("a")), false);
  kept.offer(keyHash("a"), 1, "a");

  assertEquals(kept.items(), []);
});

Deno.test("with less than the budget offered, every item is kept", () => {
  const kept = new HashBudget<string>(100);

  kept.offer(keyHash("a"), 1, "a");
  kept.offer(keyHash("b"), 1, "b");

  assertEquals(kept.items().sort(), ["a", "b"]);
});

Deno.test("the key hash is the hex SHA-256 of the key", async () => {
  assertEquals(keyHash("wikipedia:1"), await sha256Hex(new TextEncoder().encode("wikipedia:1")));
});
