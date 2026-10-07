# 配布物

利用者に配るもの。配る辞書と並べ替えのモデルは、リポジトリの `dictionaries/` に置く（[辞書は手元で作ってコミットし、CI は配布物にまとめるだけにする](../adr/20261004-commit-built-dictionaries-and-package-in-ci.md)）。

## ライセンス

辞書と並べ替えのモデルは、CC BY 4.0（`dictionaries/LICENSE`）で配る。辞書を作る道具のコードは MIT。

## 辞書ごとの配布物

辞書ごとに 1 つのフォルダにまとめる。

| ファイル | 中身 |
| --- | --- |
| `kanaemi-<名前>.tsv` | 辞書。Kanaemi の [テキストの辞書](https://github.com/kanaemi-app/kanaemi/blob/main/docs/spec/text-dictionary.md) |
| `NOTICE` | 辞書を作るのに使った素材の表示（下） |
| `LICENSE` | CC BY 4.0 |

基本辞書のフォルダには、組にした [並べ替えのモデル](ranking.md) の `ranking.model` も入れる。Kanaemi の設定のフォルダに置いて使う。基本辞書の `NOTICE` には、モデルを学習するのに使った素材の表示も入れる。

次のものは配らない。

- Kanaemi が読めない行のある辞書と、Kanaemi が読めないモデル
- 基本辞書と組でないモデル
- 基本辞書と同じ行を持つ追加辞書

## 目録

配る辞書の一覧を、配布物のフォルダと並べて `index.json` に書く。Kanaemi の設定アプリは、これを見て辞書を入れ、入れたものが最新かを確かめる。

```json
{
  "format": 1,
  "dictionaries": [
    {
      "name": "base",
      "base": true,
      "label": "Kanaemi 公式辞書・基本（kanaemi-dict）",
      "archive": "kanaemi-base.zip",
      "dictionary": { "file": "kanaemi-base.tsv", "size": 15897567, "sha256": "…" },
      "model": { "format": 3, "sha256": "…" }
    }
  ]
}
```

- `format`：目録の形の版。この形は `1`。形を変えたら上げる。
- `dictionaries`：配る辞書のすべて。基本辞書を先に、ほかは名前の順。
- `name`：辞書の名前。配布物のフォルダの名前と同じ。
- `base`：基本辞書なら `true`。
- `label`：辞書の 1 行目の説明。
- `archive`：その辞書の配布物をまとめたファイルの名前（下の [リリース](#リリース)）。
- `dictionary`：配布物の中の辞書のファイルの名前と、そのバイト数と SHA-256。
- `model`：基本辞書だけにある。配布物の中の `ranking.model` の形式の版と SHA-256。
- SHA-256 は、小文字の 16 進で書く。まとめたファイルの SHA-256 は書かない。まとめるたびに中の時刻が変わりうるので、中身が同じでも同じにならない。

## 素材の表示

`NOTICE` には、その辞書を作るのに使った素材ごとに、次のものを並べる。

- 素材の名前と URL
- 素材のライセンスや公開元が求める表示
- 素材をどう使ったか（語を数えた、読みを取った、など）と、素材の文章は辞書に含まないこと
- 一覧から取った項目（漢字一字の読み、記号、人名 など）は、その一覧の名前と、固定した版（commit や版番号のあるもの）と、その一覧の条件の全文

冒頭には、素材の文章は辞書に含まないことと、一覧から取った項目はそれぞれの一覧の条件に従って含むことを書く。

## リリース

GitHub のリリースを公開すると、そのタグの辞書ごとの配布物を、`<名前>/` のフォルダごと `kanaemi-<名前>.zip` にしてリリースに付ける。目録の `index.json` も付ける。リリースは手で作る。最新のリリースのものは `https://github.com/kanaemi-app/kanaemi-dict/releases/latest/download/kanaemi-<名前>.zip` と `https://github.com/kanaemi-app/kanaemi-dict/releases/latest/download/index.json` で取れる。
