#!/usr/bin/env bash
# Assemble artifact/build/, a self-contained Docker build context: egg-stitch,
# babble and stitch shallow-cloned at their pinned commits, plus every crate and
# Python wheel they depend on, so the builds need no network.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
build=$here/build

pin() { sed -n "s/^$1 = \"\(.*\)\"$/\1/p" "$root/expts/run_models/$2.py"; }

clone() {
    git init -q "$build/$1"
    git -C "$build/$1" remote add origin "$2"
    git -C "$build/$1" fetch -q --depth 1 origin "$3"
    git -C "$build/$1" checkout -q FETCH_HEAD
}

# The egg-stitch clone comes from GitHub, while the files copied below come from
# this checkout; refuse to mix them.
if [[ -n $(git -C "$root" status --porcelain) ]]; then
    echo "commit or stash local changes first" >&2
    exit 1
fi
git -C "$root" fetch -q origin
if [[ -z $(git -C "$root" branch -r --contains HEAD) ]]; then
    echo "push HEAD to origin first" >&2
    exit 1
fi

rm -rf "$build"
mkdir -p "$build"
estitch_commit=$(git -C "$root" rev-parse HEAD)
babble_commit=$(pin BABBLE_COMMIT babble)
stitch_commit=$(pin STITCH_COMMIT stitch)
clone egg-stitch https://github.com/mlb2251/egg-stitch.git "$estitch_commit"
# Instructions for coding assistants, not part of the artifact.
rm "$build/egg-stitch/CLAUDE.md"
clone babble https://github.com/kavigupta/babble.git "$babble_commit"
# babble's DreamCoder benchmark inputs are a submodule with an SSH URL.
git -C "$build/babble" -c url.https://github.com/.insteadOf=git@github.com: \
    submodule update -q --init --depth 1
clone stitch https://github.com/mlb2251/stitch.git "$stitch_commit"
# stitch doesn't track its Cargo.lock, so ship the one the reported numbers were built with.
cp "$here/stitch.Cargo.lock" "$build/stitch/Cargo.lock"

cd "$build"
cargo vendor --locked --manifest-path egg-stitch/Cargo.toml \
    -s babble/Cargo.toml -s stitch/Cargo.toml vendor > cargo-config.toml
# Wheels for the image's Python (CPython 3.12, x86-64 Linux), whatever Python runs this.
grep -v -e '^#' -e '^s-exp-parser==' egg-stitch/requirements-lock.txt > binary-reqs.txt
python3 -m pip download -q --no-deps --only-binary=:all: --implementation cp \
    --python-version 3.12 --abi cp312 --platform manylinux_2_28_x86_64 \
    --platform manylinux_2_17_x86_64 --platform manylinux2014_x86_64 \
    -r binary-reqs.txt -d wheels
rm binary-reqs.txt
# s-exp-parser only publishes an sdist; it's pure Python, so a locally built wheel works.
python3 -m pip wheel -q --no-deps "$(grep '^s-exp-parser==' egg-stitch/requirements-lock.txt)" -w wheels
cp "$here/Dockerfile" "$here/BUILDING.md" .
sed -e "s/@ESTITCH_COMMIT@/$estitch_commit/" -e "s/@BABBLE_COMMIT@/$babble_commit/" \
    -e "s/@STITCH_COMMIT@/$stitch_commit/" "$here/README.md" > README.md
echo "build context ready: $build"
