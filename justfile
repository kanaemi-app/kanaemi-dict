# Run `nix develop` (or use direnv) first so every tool is on PATH.

set shell := ["bash", "-euo", "pipefail", "-c"]

# List the recipes.
default:
    @just --list

# Format every source.
fmt:
    cargo fmt --all
    deno fmt
    nix fmt flake.nix

# Check formatting without changing files.
fmt-check:
    cargo fmt --all --check
    deno fmt --check

# Lint every crate and script.
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    deno lint
    deno check scripts/

# Run the Rust and script tests.
test:
    cargo test --workspace
    deno test --allow-read --allow-write --allow-net=127.0.0.1 --allow-run=lbzip2

# Fetch every source (or those named: aozora, law, wikinews, wikipedia, pydocs, rurema, fineweb) into build/raw;
# `--minutes N` stops starting downloads after N minutes and exits with 75 if unfinished.
fetch *args:
    deno run --allow-read --allow-write --allow-net scripts/fetch.ts {{args}}

# Take the documents out of build/raw into build/docs.jsonl.
docs:
    deno run --allow-read --allow-write --allow-run=lbzip2 scripts/docs.ts

# Fetch SudachiDict into build/sudachi/raw and write the analyzer's dictionary, the dictionary that checks its readings
# and the UniDic lexicon into build/sudachi/.
sudachi:
    deno run --allow-read --allow-write --allow-net scripts/sudachi.ts

# Fetch Mozc's single kanji table and the Unihan readings into build/kanji/raw and write them into build/kanji/.
kanji:
    deno run --allow-read --allow-write --allow-net scripts/kanji.ts

# Write the titles of the Wikipedia articles into build/base-titles.tsv and the names of every article and redirect
# into build/base-names.txt, taking the Wikipedia dump out of build/raw.
base-titles:
    deno run --allow-read --allow-write --allow-run=lbzip2 scripts/base-titles.ts

# Cut every document of build/docs.jsonl into build/units.jsonl.
units:
    cargo run --release -p kanaemi-dict -- units

# Build build/dictionaries/base.tsv from build/units.jsonl and the UniDic lexicon;
# `--train-only` builds build/dictionaries/base-train.tsv from the train documents alone.
dictionary *flags:
    cargo run --release -p kanaemi-dict -- dictionary {{flags}}

# Convert the eval documents' units with build/dictionaries/base-train.tsv into build/evaluation.tsv.
evaluate:
    cargo run --release -p kanaemi-dict -- evaluate

# Set up this year's dictionary of new words (or YEAR's: `--year YEAR`) if it is not there, and fetch the postal code
# data and the hot entries of every year with a dictionary into build/additional/raw.
additional-fetch *args:
    deno run --allow-read --allow-write --allow-net scripts/additional.ts fetch {{args}}

# Write what each dictionary under additional/ is built from into build/additional/NAME/,
# taking the Wikipedia dump and the laws out of build/raw.
additional-docs:
    deno run --allow-read --allow-write --allow-run=lbzip2 scripts/additional.ts docs

# Build the additional dictionaries NAME (or every one) into build/dictionaries/NAME.tsv,
# without what build/dictionaries/base.tsv already gives.
additional *names:
    cargo run --release -p kanaemi-dict -- additional {{names}}

# Train the ranking model into build/dictionaries/base.model on the candidates of build/dictionaries/base-train.tsv,
# paired with build/dictionaries/base.tsv (build it first), and measure the eval documents without and with it into build/ranking-evaluation.tsv.
ranking:
    cargo run --release -p kanaemi-dict -- ranking

# Put the dictionaries and the model of build/dictionaries/ into dictionaries/, once they check as they would ship.
take:
    cargo run --release -p kanaemi-dict -- take

# Check dictionaries/ and gather each dictionary with its notice and license into build/dist/NAME/, with the catalog build/dist/index.json.
dist:
    cargo run --release -p kanaemi-dict -- dist

# Convert the word set evaluation/words.tsv with the base dictionary and model of dictionaries/, without and with the model,
# into build/check-words.tsv and the words that did not come first into build/check-words-misses.tsv;
# `--build` converts build/dictionaries/base.tsv instead, without a model.
check-words *flags:
    cargo run --release -p kanaemi-dict -- check-words {{flags}}

# Sample the items of dictionaries/ by stratum into build/check-sample.tsv for people to judge.
check-sample:
    cargo run --release -p kanaemi-dict -- check-sample

# Read the surfaces with kanji of dictionaries/ with MeCab and IPAdic, and list the items it never reads as the
# dictionary does in build/check-readings.tsv.
check-readings:
    cargo run --release -p kanaemi-dict -- check-readings

# Fetch the readings in kana of Wikidata's items from QLever into build/wikidata/ (again with `--refresh`).
wikidata *args:
    deno run --allow-read --allow-write --allow-net scripts/wikidata.ts {{args}}

# List the words of dictionaries/base.tsv that read none of the ways the Wikidata items of their surface read
# in build/check-wikidata.tsv.
check-wikidata:
    cargo run --release -p kanaemi-dict -- check-wikidata

# Run the tests that need SudachiDict (`just sudachi` first).
test-sudachi:
    cargo test --workspace --release -- --ignored

# Everything CI runs.
ci: fmt-check lint test
