/** A page of a MediaWiki XML export, with `extra` elements after its ID; for tests. */
export function page(
  id: number,
  ns: number,
  extra: string,
  text: string,
  title = `t${id}`,
): string {
  return `<page><title>${title}</title><ns>${ns}</ns><id>${id}</id>${extra}<revision><id>9${id}</id><text bytes="1" xml:space="preserve">${text}</text></revision></page>`;
}

/** A MediaWiki XML export of `pages`; for tests. */
export function dump(pages: string[]): string {
  return `<mediawiki xmlns="http://www.mediawiki.org/xml/export-0.11/"><siteinfo><sitename>テスト</sitename></siteinfo>${
    pages.join("")
  }</mediawiki>`;
}

/** A temporary file of `text` compressed by bzip2; for tests. */
export async function bzip2File(text: string): Promise<string> {
  const path = await Deno.makeTempFile({ suffix: ".bz2" });
  const child = new Deno.Command("lbzip2", {
    args: ["-c"],
    clearEnv: true,
    stdin: "piped",
    stdout: "piped",
  })
    .spawn();
  const writer = child.stdin.getWriter();
  await writer.write(new TextEncoder().encode(text));
  await writer.close();
  const { stdout } = await child.output();
  await Deno.writeFile(path, stdout);
  return path;
}
