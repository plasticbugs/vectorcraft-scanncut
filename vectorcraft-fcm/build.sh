#!/bin/sh
# Builds the plug-in: dist/scanncut-fcm.wasm, which VectorCraft installs with File › Open.
# Needs the wasm32-unknown-unknown target: `rustup target add wasm32-unknown-unknown`.
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
mkdir -p dist
cp target/wasm32-unknown-unknown/release/vectorcraft_fcm.wasm dist/scanncut-fcm.wasm
echo "built dist/scanncut-fcm.wasm ($(wc -c < dist/scanncut-fcm.wasm | tr -d ' ') bytes)"
