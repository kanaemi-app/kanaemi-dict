import { assertEquals } from "@std/assert";
import { robotsAllows, robotsPath } from "./robots.ts";

const ROBOTS = `
User-agent: other-bot
Disallow: /

User-agent: *
Disallow: /private/
Allow: /private/open/
Disallow: /*.pdf$
`;

Deno.test("robots.txt rules for every agent apply, the longest match winning", () => {
  const allows = (path: string) => robotsAllows(ROBOTS, "kanaemi-dict-fetch", path);

  assertEquals(allows("/"), true);
  assertEquals(allows("/private/x.html"), false);
  assertEquals(allows("/private/open/x.html"), true);
  assertEquals(allows("/a/b.pdf"), false);
  assertEquals(allows("/a/b.pdf.html"), true);
});

Deno.test("a group naming the agent replaces the rules for every agent", () => {
  assertEquals(robotsAllows(ROBOTS, "other-bot/1.0", "/"), false);
});

Deno.test("an empty Disallow ends its group like any other rule", () => {
  const robots = "User-agent: *\nDisallow:\n\nUser-agent: other-bot\nDisallow: /\n";

  assertEquals(robotsAllows(robots, "kanaemi-dict-fetch", "/"), true);
  assertEquals(robotsAllows(robots, "other-bot", "/"), false);
});

Deno.test("a rule with a query string applies to the URL's path and query", () => {
  const robots = "User-agent: *\nDisallow: /search?";

  assertEquals(robotsAllows(robots, "x", robotsPath("https://h.example/search?q=1")), false);
  assertEquals(robotsAllows(robots, "x", robotsPath("https://h.example/search/")), true);
});
