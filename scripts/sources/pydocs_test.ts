import { assertEquals } from "@std/assert";
import { pydocs } from "./pydocs.ts";
import { zipOf } from "../zip_test_helper.ts";

function page(main: string): string {
  return `<!DOCTYPE html><html lang="ja"><head><meta charset="utf-8"><title>t</title></head><body>
<div class="related" role="navigation"><ul><li>ナビゲーション</li></ul></div>
<div class="document"><div class="body" role="main">${main}</div></div>
<div class="sphinxsidebar" role="navigation"><h3>目次</h3></div>
<div class="footer">著作権表示</div></body></html>`;
}

const MAIN = `<section id="more-on-lists">
<h2>リスト型についてもう少し<a class="headerlink" href="#more-on-lists">¶</a></h2>
<p>すべてのメソッドを以下に示します:</p>
<dl class="py method">
<dt class="sig sig-object py"><span class="sig-name descname"><span class="pre">append</span></span>(<em>value</em>)</dt>
<dd><p>リストの末尾に要素を一つ追加します。<code class="docutils literal">a[len(a):] = [x]</code> と同様です。</p></dd>
</dl>
<p>例えば:</p>
<div class="highlight-python3 notranslate"><div class="highlight"><pre><span></span>&gt;&gt;&gt; a = [1]  # 足し算の例
</pre></div></div>
<div class="admonition tip"><p class="admonition-title">Tip</p><p>Tip の説明文です。</p></div>
<ul><li><p>箇条書きの項目です。</p></li></ul>
<p>English only paragraph.</p>
</section>`;

Deno.test("the main content of each page, without code, signatures or heading marks", async () => {
  const zip = await zipOf([["python-3.14-docs-html/tutorial/datastructures.html", page(MAIN)]]);

  assertEquals(await pydocs("pydocs-ja", zip), [{
    doc_id: "pydocs:tutorial/datastructures.html",
    source_id: "pydocs-ja",
    text: [
      "リスト型についてもう少し",
      "すべてのメソッドを以下に示します:",
      "リストの末尾に要素を一つ追加します。a[len(a):] = [x] と同様です。",
      "例えば:",
      "Tip の説明文です。",
      "箇条書きの項目です。",
    ].join("\n"),
  }]);
});

Deno.test("tables keep the prose in their cells, one line per cell", async () => {
  const main =
    "<table><thead><tr><th>演算</th><th>結果</th></tr></thead><tbody><tr><td>x or y</td><td>x が真なら x, そうでなければ y</td></tr></tbody></table>";
  const zip = await zipOf([["d/a.html", page(main)]]);

  assertEquals(
    (await pydocs("pydocs-ja", zip))[0].text,
    "演算\n結果\nx が真なら x, そうでなければ y",
  );
});

Deno.test("index and search pages and non-HTML files are skipped, the rest in name order", async () => {
  const zip = await zipOf([
    ["d/b.html", page("<p>二</p>")],
    ["d/genindex-A.html", page("<p>索引</p>")],
    ["d/py-modindex.html", page("<p>モジュール索引</p>")],
    ["d/search.html", page("<p>検索</p>")],
    ["d/_static/x.js", "x"],
    ["d/a.html", page("<p>一</p>")],
  ]);

  assertEquals((await pydocs("pydocs-ja", zip)).map((d) => d.doc_id), [
    "pydocs:a.html",
    "pydocs:b.html",
  ]);
});
