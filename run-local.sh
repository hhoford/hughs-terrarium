#!/bin/sh
set -eu
cd "$(dirname "$0")"
mkdir -p .runtime/world .runtime/geography
npm --prefix frontend ci
npm --prefix frontend run build
cargo build --locked --release --manifest-path backend/Cargo.toml
HUG_GAMER_STATIC="$PWD/frontend/dist" \
HUG_GAMER_BIND="${HUG_GAMER_BIND:-127.0.0.1:8787}" \
HUG_WORLD_ROOT="$PWD/.runtime/world" \
HUG_GEOGRAPHY_ROOT="$PWD/.runtime/geography" \
HUG_GEOGRAPHY_MODE="${HUG_GEOGRAPHY_MODE:-build}" \
  backend/target/release/hug-gamer-backend
