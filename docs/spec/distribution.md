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

## 素材の表示

`NOTICE` には、その辞書を作るのに使った素材ごとに、次のものを並べる。

- 素材の名前と URL
- 素材のライセンスや公開元が求める表示
- 素材をどう使ったか（語を数えた、読みを取った、など）と、素材の文章は辞書に含まないこと

## リリース

GitHub のリリースを公開すると、そのタグの辞書ごとの配布物を、`<名前>/` のフォルダごと `kanaemi-<名前>.zip` にしてリリースに付ける。リリースは手で作る。最新のリリースのものは `https://github.com/kanaemi-app/kanaemi-dict/releases/latest/download/kanaemi-<名前>.zip` で取れる。
