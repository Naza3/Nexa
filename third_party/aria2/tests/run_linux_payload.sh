#!/usr/bin/env bash
# Separate Linux implementation/fixture evidence; never target-Windows evidence.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../../.." && pwd)
work=${1:?usage: run_linux_payload.sh PREPARED_ABSOLUTE_WORK_DIRECTORY}
work=$(cd "$work" && pwd)
src="$work/aria2-1.37.0"
mkdir -p "$work/native-build" "$work/artifacts"
cd "$work/native-build"
"$src/configure" --disable-bittorrent --disable-metalink --disable-websocket --disable-nls \
  --with-openssl --without-wintls --without-appletls --without-gnutls \
  --without-libxml2 --without-libexpat --without-sqlite3 --without-libcares --without-libssh2 \
  --without-libz --without-libnettle --without-libgmp --without-libgcrypt \
  CXXFLAGS='-O1 -g0' CFLAGS='-O1 -g0' > "$work/artifacts/linux-configure.log" 2>&1
make -j"${NEXA_BUILD_JOBS:-2}" > "$work/artifacts/linux-build.log" 2>&1
common=(-std=c++11 -O1 -g0 -DHAVE_CONFIG_H -I. -I"$src/src" -I"$src/lib" -Ilib)
g++ "${common[@]}" "$repo/third_party/aria2/tests/payload_unit.cc" \
  src/.libs/libaria2.a -lssl -lcrypto -o "$work/native-build/payload_unit"
"$work/native-build/payload_unit" "$work/native-build/unit-payload" > "$work/artifacts/linux-payload-unit.log"
# Instrument the affected IOFile translation unit and regression test. The rest
# of libaria2 is NOT an ASan/UBSan build; make that narrower claim explicit.
g++ "${common[@]}" -g -fsanitize=address,undefined \
  "$repo/third_party/aria2/tests/payload_unit.cc" "$src/src/IOFile.cc" \
  src/.libs/libaria2.a -lssl -lcrypto -o "$work/native-build/payload_unit_sanitize"
# LeakSanitizer is outside this focused bounds regression and unsupported in some sandboxes.
ASAN_OPTIONS=detect_leaks=0 "$work/native-build/payload_unit_sanitize" "$work/native-build/unit-payload" \
  > "$work/artifacts/linux-iofile-sanitizer.log" 2>&1
gcc -shared -fPIC "$repo/third_party/aria2/tests/linux_fixture_hook.c" -ldl -o "$work/native-build/fixture_hook.so"
fixture_status=0
python3 "$repo/third_party/aria2/tests/linux_payload_probe.py" --binary "$work/native-build/src/aria2c" \
  --hook "$work/native-build/fixture_hook.so" --work "$work/linux-payload-fixtures" || fixture_status=$?
cp "$work/linux-payload-fixtures/results.json" "$work/artifacts/linux-payload-results.json"
python3 "$repo/scripts/build_aria2_windows.py" bundle --work "$work"
exit "$fixture_status"
