#!/usr/bin/env bash
# Builds one self-contained executable: API server + web app + PostgreSQL 17 (D25).
#   Linux x86-64:  dist/hengtu-linux-x86_64 (static musl binary)
#   macOS:         dist/hengtu-macos-arm64 / dist/hengtu-macos-x86_64
#   Windows:       dist/hengtu-windows-x86_64.exe (built on Windows, or cross-built on Linux with mingw-w64)
# Usage: scripts/build-bundle.sh [linux-x86_64|macos-arm64|macos-x86_64|windows-x86_64]   (default: this machine)
set -euo pipefail
cd "$(dirname "$0")/.."

PLATFORM=${1:-}
if [ -z "$PLATFORM" ]; then
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) PLATFORM=linux-x86_64 ;;
    Darwin-arm64) PLATFORM=macos-arm64 ;;
    Darwin-x86_64) PLATFORM=macos-x86_64 ;;
    MINGW*-x86_64|MSYS*-x86_64|CYGWIN*-x86_64) PLATFORM=windows-x86_64 ;;
    *) echo "unsupported platform $(uname -s)-$(uname -m)" >&2; exit 1 ;;
  esac
fi
case "$PLATFORM" in
  linux-x86_64) JAR=linux-amd64; TARGET=x86_64-unknown-linux-musl ;;
  macos-arm64) JAR=darwin-arm64v8; TARGET=aarch64-apple-darwin ;;
  macos-x86_64) JAR=darwin-amd64; TARGET=x86_64-apple-darwin ;;
  windows-x86_64)
    JAR=windows-amd64
    case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) TARGET=x86_64-pc-windows-msvc ;; *) TARGET=x86_64-pc-windows-gnu ;; esac ;;
  *) echo "unknown platform $PLATFORM" >&2; exit 2 ;;
esac

PG_VERSION=17.6.0
TXZ="$PWD/vendor/postgres-$PLATFORM.txz"
# CM_PG_TXZ is embedded with include_bytes!, which needs a native path on Windows.
if [ ! -f "$TXZ" ]; then
  # Older checkouts kept the Linux archive under its original name.
  if [ "$PLATFORM" = linux-x86_64 ] && [ -f vendor/postgres-linux-x86_64.txz ] && [ "$TXZ" != "$PWD/vendor/postgres-linux-x86_64.txz" ]; then
    cp vendor/postgres-linux-x86_64.txz "$TXZ"
  else
    tmp=$(mktemp -d)
    curl -sSfL --retry 5 --retry-delay 10 -o "$tmp/pg.jar" \
      "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-${JAR}/${PG_VERSION}/embedded-postgres-binaries-${JAR}-${PG_VERSION}.jar"
    if command -v unzip >/dev/null; then (cd "$tmp" && unzip -o -q pg.jar '*.txz')
    else (cd "$tmp" && python -c "import zipfile; z=zipfile.ZipFile('pg.jar'); [z.extract(n) for n in z.namelist() if n.endswith('.txz')]"); fi
    mkdir -p vendor
    mv "$tmp"/*.txz "$TXZ"
    rm -rf "$tmp"
  fi
fi

if [ ! -f web/dist/index.html ] || [ "${SKIP_WEB:-}" != 1 ]; then
  (cd web && pnpm install --frozen-lockfile && pnpm build)
fi

rustup target add "$TARGET" >/dev/null 2>&1 || true
PG_PATH="$TXZ"; command -v cygpath >/dev/null && PG_PATH=$(cygpath -m "$TXZ")
if [ "$PLATFORM" = windows-x86_64 ]; then
  if [ -z "${CM_VCRUNTIME:-}" ]; then
    [ -n "${SYSTEMROOT:-}" ] || { echo "set CM_VCRUNTIME to a vcruntime140.dll (x64)" >&2; exit 1; }
    CM_VCRUNTIME=$(cygpath -m "$SYSTEMROOT/System32/vcruntime140.dll")
  fi
  export CM_VCRUNTIME
fi
CM_PG_TXZ="$PG_PATH" cargo build --release --locked -p cm-server --features bundle --target "$TARGET"
mkdir -p dist
EXE=""; case "$PLATFORM" in windows-*) EXE=.exe ;; esac
OUT="dist/hengtu-$PLATFORM$EXE"
cp "target/$TARGET/release/cm-server$EXE" "$OUT"
strip "$OUT" 2>/dev/null || true
if [ "$(uname -s)" = Darwin ]; then codesign --force --sign - "$OUT"; fi
ls -lh "$OUT"
