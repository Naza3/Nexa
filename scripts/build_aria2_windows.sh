#!/usr/bin/env bash
# Fixed-source Windows x64 Schannel build, executed on a Linux x64 host.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
work=${1:?usage: build_aria2_windows.sh ABSOLUTE_WORK_DIRECTORY}
mkdir -p "$work"
work=$(cd "$work" && pwd)
python3 "$repo/scripts/build_aria2_windows.py" prepare --work "$work"
toolchain="$work/llvm-mingw-20240619-ucrt-ubuntu-20.04-x86_64"
src="$work/aria2-1.37.0"
export PATH="$toolchain/bin:$PATH"
export SOURCE_DATE_EPOCH=1698768000
export TZ=UTC LC_ALL=C
# Avoid ambient pkg-config dependencies and host headers/libraries in cross build.
export PKG_CONFIG_LIBDIR="$work/empty-pkgconfig"
mkdir -p "$PKG_CONFIG_LIBDIR" "$work/build" "$work/artifacts"
cd "$work/build"
"$src/configure" --host=x86_64-w64-mingw32 --build="$("$src/config.guess")" \
  --disable-shared --enable-static --disable-nls --disable-bittorrent \
  --disable-metalink --disable-websocket --disable-libaria2 \
  --with-wintls --without-appletls --without-gnutls --without-openssl \
  --without-libnettle --without-libgmp --without-libgcrypt \
  --without-libxml2 --without-libexpat --without-sqlite3 --without-libcares \
  --without-libssh2 --without-libz --without-libuv --without-tcmalloc --without-jemalloc \
  ARIA2_STATIC=yes CC=x86_64-w64-mingw32-clang CXX=x86_64-w64-mingw32-clang++ \
  AR=x86_64-w64-mingw32-ar RANLIB=x86_64-w64-mingw32-ranlib \
  WINDRES=x86_64-w64-mingw32-windres \
  CFLAGS='-O2 -g0 -D_WIN32_WINNT=0x0A00' \
  CXXFLAGS='-O2 -g0 -D_WIN32_WINNT=0x0A00' \
  LDFLAGS='-static -Wl,--no-insert-timestamp' > "$work/artifacts/configure.log" 2>&1
python3 "$repo/scripts/build_aria2_windows.py" check-config --work "$work"
make -j"${NEXA_BUILD_JOBS:-2}" > "$work/artifacts/build.log" 2>&1
common=(-std=c++11 -O2 -g0 -D_WIN32_WINNT=0x0A00 -DHAVE_CONFIG_H -I. -I"$src/src" -I"$src/lib" -Ilib)
x86_64-w64-mingw32-clang++ "${common[@]}" "$repo/third_party/aria2/tests/policy_unit.cc" \
  -static -Wl,--no-insert-timestamp -lws2_32 -o "$work/artifacts/policy_unit.exe"
# Each forbidden feature must trip the source-level guard, even if a future
# configure invocation accidentally enables it.
for macro in ENABLE_BITTORRENT ENABLE_METALINK HAVE_LIBCARES ENABLE_ASYNC_DNS HAVE_LIBSSH2; do
  if x86_64-w64-mingw32-clang++ "${common[@]}" -D"$macro"=1 -c \
      "$repo/third_party/aria2/tests/policy_unit.cc" -o "$work/forbidden.o" \
      > "$work/artifacts/guard-$macro.log" 2>&1; then
    echo "Forbidden build feature did not fail: $macro" >&2; exit 1
  fi
  grep -q 'Nexa sidecar requires' "$work/artifacts/guard-$macro.log"
done
# Link the actual configured aria2 implementation, not a model or copied parser.
./libtool --mode=link x86_64-w64-mingw32-clang++ "${common[@]}" \
  "$repo/third_party/aria2/tests/engine_unit.cc" src/libaria2.la \
  -lws2_32 -lwsock32 -lgdi32 -lwinmm -liphlpapi -lpsapi -lcrypt32 -lsecur32 -ladvapi32 \
  -static -all-static -Wl,--no-insert-timestamp -o "$work/artifacts/engine_unit.exe"
cp src/aria2c.exe "$work/artifacts/nexa-aria2.exe"
x86_64-w64-mingw32-strip "$work/artifacts/nexa-aria2.exe"
llvm-readobj --file-headers --coff-imports "$work/artifacts/nexa-aria2.exe" > "$work/artifacts/pe.txt"
x86_64-w64-mingw32-clang++ --version > "$work/artifacts/compiler.txt"
python3 "$repo/scripts/build_aria2_windows.py" bundle --work "$work"
