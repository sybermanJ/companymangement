#!/usr/bin/env bash
# Builds one self-contained Linux executable: API server + web app + PostgreSQL 17 (D25).
# Output: dist/hengtu-linux-x86_64 (static musl binary, runs on any x86-64 Linux).
set -euo pipefail
cd "$(dirname "$0")/.."

PG_VERSION=17.6.0
TXZ=vendor/postgres-linux-x86_64.txz
if [ ! -f "$TXZ" ]; then
  mkdir -p vendor
  curl -sSfLo vendor/pg.jar \
    "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-amd64/${PG_VERSION}/embedded-postgres-binaries-linux-amd64-${PG_VERSION}.jar"
  (cd vendor && unzip -o -q pg.jar 'postgres-linux-x86_64.txz' && rm pg.jar)
fi

(cd web && pnpm install --frozen-lockfile && pnpm build)

TARGET=x86_64-unknown-linux-musl
cargo build --release --locked -p cm-server --features bundle --target "$TARGET"
mkdir -p dist
cp "target/$TARGET/release/cm-server" dist/hengtu-linux-x86_64
strip dist/hengtu-linux-x86_64 2>/dev/null || true
ls -lh dist/hengtu-linux-x86_64
