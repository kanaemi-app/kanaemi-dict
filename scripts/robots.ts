/** Reading a host's robots.txt, and whether it lets a crawler fetch a page. */
import { Stopped, USER_AGENT } from "./raw/fetch.ts";

/** How long a host's robots.txt is waited for. */
const ROBOTS_TIMEOUT_MS = 60_000;

type Rule = { allow: boolean; pattern: string };

/** What a robots.txt rule matches of `url`: its path and query. */
export function robotsPath(url: string): string {
  const { pathname, search } = new URL(url);
  return pathname + search;
}

/**
 * Whether `robots` lets `agent` fetch `path`: the rules of the groups naming
 * the agent, or else of the groups for every agent, the longest matching
 * pattern deciding and Allow winning a tie.
 */
export function robotsAllows(robots: string, agent: string, path: string): boolean {
  const groups: { agents: string[]; rules: Rule[] }[] = [];
  let current: { agents: string[]; rules: Rule[] } | undefined;
  let ruled = false;
  for (const raw of robots.split(/\r?\n/)) {
    const line = raw.replace(/#.*/, "").trim();
    const colon = line.indexOf(":");
    if (colon < 0) continue;
    const field = line.slice(0, colon).trim().toLowerCase();
    const value = line.slice(colon + 1).trim();
    if (field === "user-agent") {
      if (!current || ruled) {
        current = { agents: [], rules: [] };
        groups.push(current);
        ruled = false;
      }
      current.agents.push(value.toLowerCase());
    } else if ((field === "allow" || field === "disallow") && current) {
      ruled = true;
      if (value) current.rules.push({ allow: field === "allow", pattern: value });
    }
  }
  const name = agent.toLowerCase().split("/")[0];
  const named = groups.filter((g) => g.agents.some((a) => a !== "*" && name.startsWith(a)));
  const rules = (named.length > 0 ? named : groups.filter((g) => g.agents.includes("*")))
    .flatMap((g) => g.rules);
  let decided: Rule | undefined;
  for (const rule of rules) {
    if (!matches(rule.pattern, path)) continue;
    if (
      !decided || rule.pattern.length > decided.pattern.length ||
      (rule.pattern.length === decided.pattern.length && rule.allow)
    ) {
      decided = rule;
    }
  }
  return decided?.allow ?? true;
}

function matches(pattern: string, path: string): boolean {
  const anchored = pattern.endsWith("$");
  const body = anchored ? pattern.slice(0, -1) : pattern;
  const regex = body.split("*").map((part) => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join(
    ".*",
  );
  return new RegExp(`^${regex}${anchored ? "$" : ""}`).test(path);
}

/**
 * A host's robots.txt: empty when it has none (a client error), none when it
 * cannot be read (a server error or no answer), when the host is crawled not
 * at all.
 */
export async function robotsOf(host: string, stop?: AbortSignal): Promise<string | undefined> {
  const timeout = AbortSignal.timeout(ROBOTS_TIMEOUT_MS);
  try {
    const res = await fetch(`https://${host}/robots.txt`, {
      headers: { "user-agent": USER_AGENT },
      signal: stop ? AbortSignal.any([stop, timeout]) : timeout,
    });
    if (res.ok) return await res.text();
    await res.body?.cancel();
    return res.status < 500 ? "" : undefined;
  } catch {
    if (stop?.aborted) throw new Stopped(`https://${host}/robots.txt`);
    return undefined;
  }
}
