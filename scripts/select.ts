/**
 * Taking documents up to a budget: the candidates in the order of the SHA-256
 * of their doc IDs, taken until their characters reach the budget.
 */
import { createHash } from "node:crypto";
import { sha256Hex } from "./raw/store.ts";

/** `items` sorted by the hex SHA-256 of their keys. */
export async function inHashOrder<T>(items: T[], key: (item: T) => string): Promise<T[]> {
  const keyed = await Promise.all(
    items.map(async (item) => ({
      item,
      hash: await sha256Hex(new TextEncoder().encode(key(item))),
    })),
  );
  return keyed.sort((a, b) => a.hash < b.hash ? -1 : a.hash > b.hash ? 1 : 0).map((k) => k.item);
}

/**
 * Calls `take` on each item in order until the characters it reports reach
 * `budget`, and returns the characters taken. An item `take` cannot read
 * (undefined) counts for nothing.
 */
export async function takeUntilBudget<T>(
  items: Iterable<T>,
  budget: number,
  take: (item: T) => Promise<number | undefined>,
): Promise<number> {
  let taken = 0;
  for (const item of items) {
    if (taken >= budget) break;
    taken += (await take(item)) ?? 0;
  }
  return taken;
}

/** The hex SHA-256 of `key`, the order [`inHashOrder`] sorts by. */
export function keyHash(key: string): string {
  return createHash("sha256").update(key).digest("hex");
}

/**
 * The items with the smallest hashes whose characters reach a budget, as
 * [`takeUntilBudget`] would take them in hash order, kept while the items are
 * offered one by one in any order. An item whose hash is past the kept ones
 * once they reach the budget is never taken, so it need not be read.
 */
export class HashBudget<T> {
  #kept: { hash: string; chars: number; item: T }[] = [];
  #chars = 0;

  constructor(readonly budget: number) {}

  /** Whether an item of `hash` would be kept if offered now. */
  wants(hash: string): boolean {
    if (this.#chars < this.budget) return true;
    const last = this.#kept.at(-1);
    return last !== undefined && hash < last.hash;
  }

  offer(hash: string, chars: number, item: T): void {
    if (!this.wants(hash)) return;
    let at = this.#kept.length;
    while (at > 0 && this.#kept[at - 1].hash > hash) at--;
    this.#kept.splice(at, 0, { hash, chars, item });
    this.#chars += chars;
    // Drop the last item while the ones before it still reach the budget.
    while (this.#kept.length > 1 && this.#chars - this.#kept.at(-1)!.chars >= this.budget) {
      this.#chars -= this.#kept.pop()!.chars;
    }
  }

  /** The kept items, in hash order. */
  items(): T[] {
    return this.#kept.map((k) => k.item);
  }
}
