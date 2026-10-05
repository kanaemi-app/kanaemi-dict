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

# Fetch SudachiDict into build/sudachi/raw and write the analyzer's dictionary and the UniDic lexicon into build/sudachi/.
sudachi:
    deno run --allow-read --allow-write --allow-net scripts/sudachi.ts

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

# Run the tests that need SudachiDict full (`just sudachi` first).
test-sudachi:
    cargo test --workspace --release -- --ignored

# Everything CI runs.
ci: fmt-check lint test
