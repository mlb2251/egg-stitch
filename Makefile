PORT ?= 8066

dev:
	python3 -c 'import run; run.dev()'

.PHONY: server
server:
	python3 viz/server.py $(PORT)

# ── WASM ───────────────────────────────────────────────────────────────────────
# Prerequisites (one-time):
#   cargo install wasm-pack
#   rustup target add wasm32-unknown-unknown

.PHONY: wasm
wasm:
	wasm-pack build --target web --features wasm

.PHONY: wasm-prof
wasm-prof:
	wasm-pack build --no-opt --target web --features wasm

.PHONY: wasm-debug
wasm-debug:
	wasm-pack build --dev --target web --features wasm

.PHONY: wasm-dev
wasm-dev: wasm server

# stitch:
