# kanaemi-dict

日本語入力 [Kanaemi](https://github.com/kanaemi-app/kanaemi) の公式の辞書と、それを作る道具。

集めた文章を解析器で読み、実際に使われた語から辞書を作る。候補の並べ替えのモデルも同じ文章から学習し、基本辞書と組にして配る。他の IME や辞書の項目一覧は取り込まない。考え方は [docs/concept.md](docs/concept.md) にある。

## 辞書

- 基本辞書：広い分野の文章から作る辞書。並べ替えのモデルと組で配る。
- 追加辞書：基本辞書に足して使う辞書。基本辞書にない語だけを持つ。分野の辞書（鉄道、医療、IT など）、その年の新語の辞書、地名の辞書、法律の辞書、送り仮名の揺れ（小い、行なう）の辞書がある。どの追加辞書があるかは [additional/](additional/) を見る。

## 使い方

[リリース](https://github.com/kanaemi-app/kanaemi-dict/releases/latest) から、辞書ごとの zip を取る。最新のものは次の URL で取れる。

```
https://github.com/kanaemi-app/kanaemi-dict/releases/latest/download/kanaemi-<名前>.zip
```

zip には、辞書の `kanaemi-<名前>.tsv`、素材の表示の `NOTICE`、ライセンスの `LICENSE` が入っている。基本辞書の zip には、並べ替えのモデルの `ranking.model` も入っている。

1. 辞書の `.tsv` を、Kanaemi の [設定のフォルダ](https://github.com/kanaemi-app/kanaemi/blob/main/docs/spec/settings-folder.md) の `dictionaries/` に置く。
2. 基本辞書の `ranking.model` を、設定のフォルダに置く。モデルは、組になっている基本辞書と一緒に使う。基本辞書を新しくしたら、モデルも同じ zip のものに替える。
3. 追加辞書は、基本辞書の後ろに置く。`config.toml` の [辞書の一覧](https://github.com/kanaemi-app/kanaemi/blob/main/docs/spec/settings.md#辞書の一覧) に、基本辞書を先に書く。一覧を書かないと、辞書はファイル名の順に読まれ、追加辞書が基本辞書より先に来ることがある。

```toml
dictionaries = ["custom", "kanaemi-base.tsv", "kanaemi-railway.tsv", "kanaemi-it.tsv"]
```

## ライセンス

- 辞書と並べ替えのモデル：CC BY 4.0（[dictionaries/LICENSE](dictionaries/LICENSE)）
- 辞書を作る道具のコード：MIT（[LICENSE](LICENSE)）

辞書を作るのに使った素材の表示は、配布物の `NOTICE` にある。素材の文章は、辞書にもモデルにも含まない。素材をどう扱うかは [文章は解析にだけ使い、素材のライセンスは問わない](docs/adr/20261004-read-any-text-for-analysis-only.md) による。

## 辞書を作る

辞書とモデルは手元で素材の取得から作り、評価してから [dictionaries/](dictionaries/) にコミットする。CI は、コミットされたものを確かめて配布物にまとめるだけで、素材の取得はしない（[辞書は手元で作ってコミットし、CI は配布物にまとめるだけにする](docs/adr/20261004-commit-built-dictionaries-and-package-in-ci.md)）。

道具は [Nix](https://nixos.org/) の開発環境にそろっている。`nix develop` に入ってから、[just](https://github.com/casey/just) のレシピを順に流す。素材は数 GB あり、取得と学習には時間がかかる。

```sh
nix develop

# 基本辞書
just fetch                     # 素材を build/raw に取得する
just sudachi                   # 解析器の辞書と UniDic の語彙を用意する
just docs                      # 素材から文書を取り出す
just base-titles               # Wikipedia の記事とリダイレクトの見出しを取り出す
just units                     # 文書を変換の単位に切る
just dictionary                # 基本辞書を作る
just dictionary --train-only   # 評価と学習に使う、学習用の文書だけの基本辞書を作る
just evaluate                  # 基本辞書を評価する

# 追加辞書
just additional-fetch          # 郵便番号データとその年の人気エントリーを取得する
just additional-docs           # 追加辞書ごとの文書を取り出す
just additional                # 追加辞書を作る

# 並べ替えのモデル
just ranking                   # モデルを学習し、モデルなしとありで評価する

# 入れる
just take                      # 作ったものを確かめて dictionaries/ に入れる
just dist                      # 配布物にまとめられるかを確かめる
```

`just take` のあと、`dictionaries/` の変更をコミットする。リリースは、`v<年月日>.<その日の通し番号>`（`v20260801.01`）のタグで GitHub の画面から作って公開する。公開すると、CI が辞書ごとの zip をそのリリースに付ける。

`dictionaries/` の辞書は、素材の文書での評価とは別に、解析器の外の答えでも確かめられる（[辞書の確かめ](docs/spec/checks.md)）。

```sh
just check-words               # 人が書いた正解集で変換する
just check-sample              # 人が判定する項目を層ごとに抜き出す
just check-readings            # MeCab の読みと食い違う項目を並べる
```

`just` だけを流すと、レシピの一覧が出る。コードを変えたら `just ci` を通す。

## 文書

- [docs/concept.md](docs/concept.md)：目指すもの
- [docs/adr/](docs/adr/)：設計判断の記録
- [docs/spec/](docs/spec/)：辞書・素材・配布物などの約束
- [docs/references/](docs/references/)：素材の扱いの根拠と、評価で決めた値の記録
