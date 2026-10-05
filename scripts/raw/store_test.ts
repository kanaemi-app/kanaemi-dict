import { assert, assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import { isSha256, RawStore, sha256Hex } from "./store.ts";

// SHA-256 of the ASCII bytes "abc" (FIPS 180-2 test vector).
const ABC = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const abc = new TextEncoder().encode("abc");

Deno.test("sha256Hex is lowercase hex", async () => {
  assertEquals(await sha256Hex(abc), ABC);
});

Deno.test("isSha256 accepts only 64 lowercase hex digits", () => {
  assert(isSha256(ABC));
  assert(!isSha256(ABC.toUpperCase()));
  assert(!isSha256(ABC.slice(1)));
  assert(!isSha256(`${ABC.slice(1)}z`));
});

Deno.test("put stores content under its hash, creating the directory", async () => {
  const tmp = await Deno.makeTempDir();
  const store = new RawStore(join(tmp, "build/raw"));

  const sha = await store.put(abc);

  assertEquals(sha, ABC);
  assert(await store.contains(sha));
  assertEquals(await store.read(sha), abc);
});

Deno.test("put never rewrites an existing file", async () => {
  const tmp = await Deno.makeTempDir();
  const store = new RawStore(tmp);
  const sha = await store.put(abc);
  await Deno.writeTextFile(store.path(sha), "tampered");

  assertEquals(await store.put(abc), sha);

  assertEquals(await Deno.readTextFile(store.path(sha)), "tampered");
});

Deno.test("put leaves no temporary file", async () => {
  const tmp = await Deno.makeTempDir();
  const store = new RawStore(tmp);

  await store.put(abc);

  const names = [];
  for await (const e of Deno.readDir(tmp)) names.push(e.name);
  assertEquals(names, [ABC]);
});

Deno.test("concurrent puts of the same content all succeed with one whole file", async () => {
  const tmp = await Deno.makeTempDir();
  const store = new RawStore(tmp);
  const data = new Uint8Array(1 << 20).fill(7);

  const shas = await Promise.all(Array.from({ length: 8 }, () => store.put(data)));

  assert(shas.every((sha) => sha === shas[0]));
  assertEquals(await store.read(shas[0]), data);
  const names = [];
  for await (const e of Deno.readDir(tmp)) names.push(e.name);
  assertEquals(names.length, 1);
});

Deno.test("put fails when something other than a file holds the name", async () => {
  const tmp = await Deno.makeTempDir();
  const store = new RawStore(tmp);
  await Deno.mkdir(store.path(ABC));

  await assertRejects(() => store.put(abc), Deno.errors.AlreadyExists);
  assert((await Deno.stat(store.path(ABC))).isDirectory);
});

Deno.test("contains is false for missing content", async () => {
  const tmp = await Deno.makeTempDir();

  assert(!(await new RawStore(tmp).contains(ABC)));
});

Deno.test("a stream is stored like the same bytes put whole", async () => {
  const store = new RawStore(await Deno.makeTempDir());
  const data = new TextEncoder().encode("流れてくる中身".repeat(1000));

  const streamed = await store.putStream(ReadableStream.from([data.slice(0, 10), data.slice(10)]));

  assertEquals(streamed, { sha256: await sha256Hex(data), bytes: data.length });
  assertEquals(await store.read(streamed.sha256), data);
  assertEquals((await Array.fromAsync(Deno.readDir(store.dir))).length, 1);
});
