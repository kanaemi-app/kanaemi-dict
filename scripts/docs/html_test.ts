import { assertEquals } from "@std/assert";
import { elementLines, type LineOptions, parseDocument } from "./html.ts";

function lines(body: string, options?: LineOptions): string[] {
  return elementLines(parseDocument(`<html><body>${body}</body></html>`).body, options);
}

function nonEmpty(body: string, options?: LineOptions): string[] {
  return lines(body, options).filter((line) => line.trim() !== "");
}

Deno.test("a block boundary starts a new line, an inline element does not", () => {
  assertEquals(lines("<p>一<b>二</b>三</p><div>四</div>"), ["", "一二三", "", "四", ""]);
});

Deno.test("a break or a rule starts a new line", () => {
  assertEquals(lines("一<br>二<hr>三"), ["一", "二", "三"]);
});

Deno.test("whitespace runs become one space, across inline elements too", () => {
  assertEquals(lines("<p>  a \n\t b <i> c</i></p>"), ["", " a b c", ""]);
});

Deno.test("scripts, styles and other non-prose elements are left out", () => {
  assertEquals(
    nonEmpty(
      "<script>run()</script><style>p{}</style><noscript>無効</noscript><template>型</template>" +
        "<svg><text>図</text></svg><iframe>枠</iframe><p>本文</p>",
    ),
    ["本文"],
  );
});

Deno.test("tables are left out unless kept, and a kept table gives one line per cell", () => {
  const body = "<p>前</p><table><tr><th>見出し</th><td>セル</td></tr></table><p>後</p>";

  assertEquals(nonEmpty(body), ["前", "後"]);
  assertEquals(nonEmpty(body, { keepTables: true }), ["前", "見出し", "セル", "後"]);
});

Deno.test("only the text under the given element is read", () => {
  const document = parseDocument(
    "<html><body><nav>ナビ</nav><main><h1>題</h1><p>本文</p></main></body></html>",
  );

  assertEquals(elementLines(document.querySelector("main")!).filter((l) => l), ["題", "本文"]);
});
