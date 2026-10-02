#!/usr/bin/env bash
# Explicit native build; never implicitly download MNN/models/toolchains.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${CARGO_TARGET_DIR:?set an external reusable Cargo target directory}"
: "${NEXA_MNN_ARTIFACT_DIR:?set the audited Android native artifact}"
: "${ANDROID_NDK_HOME:?set NDK r30}"
[[ "$(rustc --version)" == 'rustc 1.98.1 '* ]]
[[ "$(grep '^Pkg.Revision' "$ANDROID_NDK_HOME/source.properties" | awk '{print $3}')" == 30.0.16248370 ]]
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android28-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
export NEXA_LINK_MAP="$CARGO_TARGET_DIR/aarch64-linux-android/release/nexa_device_verifier.map"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
export NEXA_SOURCE_COMMIT="$(git rev-parse HEAD)"
export NEXA_SOURCE_DIRTY="$(test -z "$(git status --porcelain)" && echo false || echo true)"
export NEXA_BUILD_MODE=release
cargo build --manifest-path rust/Cargo.toml --locked --offline --release --target aarch64-linux-android --lib -j2
python3 tool/register_prebuilt.py
