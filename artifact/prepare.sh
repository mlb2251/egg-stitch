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

stitch_commit=$(pin STITCH_COMMIT stitch)
# stitch doesn't track its Cargo.lock, so ship the one the reported numbers were built with.
if [[ $(git -C "$root/../stitch" rev-parse HEAD) != "$stitch_commit" ]]; then
    echo "../stitch is not at the pinned $stitch_commit; its Cargo.lock may not match" >&2
    exit 1
fi

rm -rf "$build"
mkdir -p "$build"
clone egg-stitch https://github.com/mlb2251/egg-stitch.git "$(git -C "$root" rev-parse HEAD)"
clone babble https://github.com/kavigupta/babble.git "$(pin BABBLE_COMMIT babble)"
clone stitch https://github.com/mlb2251/stitch.git "$stitch_commit"
cp "$root/../stitch/Cargo.lock" "$build/stitch/"

cd "$build"
cargo vendor --locked --manifest-path egg-stitch/Cargo.toml \
    -s babble/Cargo.toml -s stitch/Cargo.toml vendor > cargo-config.toml
# The image's Python is 3.12 on x86-64 Linux, so these wheels must come from the same.
python3 -m pip wheel -q --no-deps -r egg-stitch/requirements-lock.txt -w wheels
cp "$here/Dockerfile" "$here/README.md" .
echo "build context ready: $build"
