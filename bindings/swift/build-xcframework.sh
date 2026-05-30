#!/usr/bin/env bash
# build-xcframework.sh — Build XCFramework for iOS from the ldgr-ffi Rust crate.
#
# Prerequisites:
#   - Xcode (with iOS SDK)
#   - Rust targets: rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
#   - uniffi-bindgen workspace crate (crates/uniffi-bindgen)
#
# Usage:
#   cd bindings/swift
#   ./build-xcframework.sh [--release]
#
# Output:
#   Frameworks/ldgr_ffiFFI.xcframework   — fat XCFramework
#   Sources/LdgrFFI/ldgr_ffi.swift       — generated Swift bindings

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FFI_CRATE="$REPO_ROOT/crates/ldgr-ffi"
OUT_DIR="$SCRIPT_DIR/Frameworks"
SWIFT_OUT="$SCRIPT_DIR/Sources/LdgrFFI"
UDL_PATH="$FFI_CRATE/src/ldgr.udl"

PROFILE="debug"
CARGO_FLAGS=""
if [[ "${1:-}" == "--release" ]]; then
    PROFILE="release"
    CARGO_FLAGS="--release"
fi

echo "╔══════════════════════════════════════════════════╗"
echo "║  ldgr — Building XCFramework ($PROFILE)         ║"
echo "╚══════════════════════════════════════════════════╝"
echo ""

# ── Step 1: Build for iOS targets ───────────────────────────────────────────────

echo "▸ Building for aarch64-apple-ios (device)…"
cargo build -p ldgr-ffi --target aarch64-apple-ios $CARGO_FLAGS

echo "▸ Building for aarch64-apple-ios-sim (ARM simulator)…"
cargo build -p ldgr-ffi --target aarch64-apple-ios-sim $CARGO_FLAGS

echo "▸ Building for x86_64-apple-ios (Intel simulator)…"
cargo build -p ldgr-ffi --target x86_64-apple-ios $CARGO_FLAGS

DEVICE_LIB="$REPO_ROOT/target/aarch64-apple-ios/$PROFILE/libldgr_ffi.a"
SIM_ARM_LIB="$REPO_ROOT/target/aarch64-apple-ios-sim/$PROFILE/libldgr_ffi.a"
SIM_X86_LIB="$REPO_ROOT/target/x86_64-apple-ios/$PROFILE/libldgr_ffi.a"

for lib in "$DEVICE_LIB" "$SIM_ARM_LIB" "$SIM_X86_LIB"; do
    if [[ ! -f "$lib" ]]; then
        echo "ERROR: Expected library not found: $lib"
        exit 1
    fi
done

# Create universal simulator library
echo "▸ Creating universal simulator library (arm64 + x86_64)…"
SIM_UNIVERSAL="$REPO_ROOT/target/universal-sim/$PROFILE"
mkdir -p "$SIM_UNIVERSAL"
lipo -create "$SIM_ARM_LIB" "$SIM_X86_LIB" \
     -output "$SIM_UNIVERSAL/libldgr_ffi.a"

SIM_LIB="$SIM_UNIVERSAL/libldgr_ffi.a"

# ── Step 2: Generate Swift bindings ─────────────────────────────────────────────

echo "▸ Generating Swift bindings…"
mkdir -p "$SWIFT_OUT"

cargo run -p uniffi-bindgen -- generate "$UDL_PATH" \
    --language swift \
    --out-dir "$SWIFT_OUT" \
    --lib-file "$DEVICE_LIB"

# Locate generated header and modulemap
HEADER="$SWIFT_OUT/ldgr_ffiFFI.h"
MODULEMAP="$SWIFT_OUT/ldgr_ffiFFI.modulemap"

if [[ ! -f "$HEADER" ]]; then
    HEADER=$(find "$SWIFT_OUT" -name "*.h" | head -1)
    MODULEMAP=$(find "$SWIFT_OUT" -name "*.modulemap" | head -1)
fi

# ── Step 3: Create XCFramework ──────────────────────────────────────────────────

echo "▸ Packaging XCFramework…"
rm -rf "$OUT_DIR/ldgr_ffiFFI.xcframework"

# Prepare header directories (must be named module.modulemap for xcframework)
DEVICE_HEADERS="$OUT_DIR/headers-device"
SIM_HEADERS="$OUT_DIR/headers-sim"
rm -rf "$DEVICE_HEADERS" "$SIM_HEADERS"
mkdir -p "$DEVICE_HEADERS" "$SIM_HEADERS"

if [[ -n "${HEADER:-}" && -f "$HEADER" ]]; then
    cp "$HEADER" "$DEVICE_HEADERS/"
    cp "$HEADER" "$SIM_HEADERS/"
    if [[ -n "${MODULEMAP:-}" && -f "$MODULEMAP" ]]; then
        cp "$MODULEMAP" "$DEVICE_HEADERS/module.modulemap"
        cp "$MODULEMAP" "$SIM_HEADERS/module.modulemap"
    fi
fi

xcodebuild -create-xcframework \
    -library "$DEVICE_LIB" \
    -headers "$DEVICE_HEADERS" \
    -library "$SIM_LIB" \
    -headers "$SIM_HEADERS" \
    -output "$OUT_DIR/ldgr_ffiFFI.xcframework"

# Clean up temp dirs
rm -rf "$DEVICE_HEADERS" "$SIM_HEADERS"

echo ""
echo "✓ XCFramework built: $OUT_DIR/ldgr_ffiFFI.xcframework"
echo "✓ Swift bindings:    $SWIFT_OUT/"
echo ""
echo "Add to your Xcode project:"
echo "  1. Open Package.swift in Xcode"
echo "  2. Or: .package(path: \"bindings/swift\")"
