#!/usr/bin/env bash
# Build release argus-local (Darwin) and stage a portable macOS arm64 zip + SHA256.
# ARG-34 packaging helper. Does NOT upload a GitHub Release asset.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT_DIR="${OUT_DIR:-proof/arg-34}"
DIST_DIR="${DIST_DIR:-dist}"
SKIP_BUILD="${SKIP_BUILD:-0}"

arch="$(uname -m)"
os="$(uname -s)"
if [[ "$os" != "Darwin" ]]; then
  echo "error: package-macos-release.sh must run on macOS (got $os)" >&2
  exit 1
fi

# Prefer Apple Silicon; document if building on Intel
if [[ "$arch" == "arm64" ]]; then
  TARGET="aarch64-apple-darwin"
elif [[ "$arch" == "x86_64" ]]; then
  TARGET="x86_64-apple-darwin"
  echo "warn: building on Intel x86_64 — prefer Apple Silicon / macos-14 CI for arm64"
else
  echo "error: unsupported arch $arch" >&2
  exit 1
fi

if [[ "$SKIP_BUILD" != "1" ]]; then
  echo "==> rustup target add $TARGET (if needed)"
  rustup target add "$TARGET" >/dev/null
  echo "==> cargo build --release --target $TARGET"
  cargo build --release --target "$TARGET"
fi

BIN_SRC="target/${TARGET}/release/argus-local"
if [[ ! -f "$BIN_SRC" ]]; then
  if [[ -f target/release/argus-local ]]; then
    BIN_SRC="target/release/argus-local"
  else
    echo "error: missing binary at $BIN_SRC" >&2
    exit 1
  fi
fi

mkdir -p "$OUT_DIR" "$DIST_DIR"
BIN_NAME="argus-local"
ZIP_NAME="argus-local-macos-arm64.zip"
if [[ "$TARGET" == "x86_64-apple-darwin" ]]; then
  ZIP_NAME="argus-local-macos-x86_64.zip"
fi

BIN_DST="$OUT_DIR/$BIN_NAME"
cp -f "$BIN_SRC" "$BIN_DST"
chmod +x "$BIN_DST"

BIN_HASH="$(shasum -a 256 "$BIN_DST" | awk '{print $1}')"
printf '%s  %s\n' "$BIN_HASH" "$BIN_NAME" > "$OUT_DIR/${BIN_NAME}.sha256"

STAGE="$OUT_DIR/stage-macos"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -f "$BIN_DST" "$STAGE/$BIN_NAME"
cp -f "$OUT_DIR/${BIN_NAME}.sha256" "$STAGE/${BIN_NAME}.sha256"
cat > "$STAGE/README-macOS.txt" <<EOF
Argus Local (macOS)

Arch: $TARGET
Binary: ./argus-local

Gatekeeper: first launch may be blocked because this build is not notarized.
Right-click the binary → Open, or:
  xattr -d com.apple.quarantine ./argus-local
then:
  ./argus-local open
  ./argus-local tui --fixtures

No Rust toolchain required.
EOF

ZIP_DST="$OUT_DIR/$ZIP_NAME"
rm -f "$ZIP_DST"
(
  cd "$STAGE"
  zip -q -r "../$ZIP_NAME" .
)

ZIP_HASH="$(shasum -a 256 "$ZIP_DST" | awk '{print $1}')"
printf '%s  %s\n' "$ZIP_HASH" "$ZIP_NAME" > "$OUT_DIR/${ZIP_NAME}.sha256"

cp -f "$ZIP_DST" "$DIST_DIR/$ZIP_NAME"
cp -f "$OUT_DIR/${ZIP_NAME}.sha256" "$DIST_DIR/${ZIP_NAME}.sha256"
cp -f "$STAGE/README-macOS.txt" "$OUT_DIR/README-macOS.txt"

echo ""
echo "BIN:  $BIN_DST"
echo "      SHA256=$BIN_HASH"
echo "ZIP:  $ZIP_DST"
echo "      SHA256=$ZIP_HASH"
echo "Target: $TARGET"
echo "Also copied to $DIST_DIR/"
echo ""
echo "Do NOT attach to GitHub Release until Review+PM then Gadi clear (ARG-34)."