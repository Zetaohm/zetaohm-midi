#!/bin/sh
# Checks of the C ABI (DEC-019) that the specdev gates do not run: the header is current, the
# CMake and the Meson packages build and run the C tests on the host, and the staticlib links for a
# bare-metal target (flash and RAM printed). CI runs this script after the gates.
# Needs cbindgen, cmake, meson, ninja, a C compiler and arm-none-eabi-gcc. Set CORROSION_DIR to a
# Corrosion checkout to build with no network access.
set -eu
cd "$(dirname "$0")/.."
for tool in cbindgen cmake meson ninja cc arm-none-eabi-gcc; do
  command -v "$tool" >/dev/null || { echo "ffi-check: $tool is missing" >&2; exit 2; }
done
ffi=crates/zmidi-ffi
build=target/ffi-check
fetch=""
[ -n "${CORROSION_DIR:-}" ] && fetch="-DFETCHCONTENT_SOURCE_DIR_CORROSION=$CORROSION_DIR"

echo "ffi-check: header" >&2
(cd "$ffi" && cbindgen --quiet --config cbindgen.toml --verify -o include/zmidi.h) || {
  echo "ffi-check: include/zmidi.h is not current: run (cd $ffi && cbindgen --config cbindgen.toml -o include/zmidi.h)" >&2
  exit 1
}

echo "ffi-check: host (CMake, CTest)" >&2
cmake -S "$ffi/tests/cmake" -B "$build/host" $fetch >/dev/null
cmake --build "$build/host" >/dev/null
ctest --test-dir "$build/host" --output-on-failure

# DEC-019 rule 1: the storage slots also hold the Rust state on wasm32 (u128 aligned to 16 there).
echo "ffi-check: wasm32-unknown-unknown storage sizes" >&2
cargo check --quiet -p zmidi-ffi --lib --no-default-features --target wasm32-unknown-unknown \
  --features io,ump,ble,file,ci,net,net-auth,usb

echo "ffi-check: thumbv7em-none-eabihf (CMake, Corrosion)" >&2
cmake -S "$ffi/tests/cmake" -B "$build/thumbv7em" -DCMAKE_TOOLCHAIN_FILE=thumbv7em.cmake $fetch >/dev/null
cmake --build "$build/thumbv7em" >/dev/null
for fw in fw fw_ci fw_usb_fixed; do
  arm-none-eabi-size "$build/thumbv7em/$fw" |
    awk -v job="$fw" 'NR == 2 { printf "ffi-check: %s on Cortex-M4F: flash %d B, RAM %d B\n", job, $1 + $2, $2 + $3 }'
done
# OPT-038: the fixed USB MIDI function stays small (1,114 B for the probe of 2026-10-25, with 662 B of
# memcpy); the run-time writer is 8,434 B. A link over the budget means the writer came back.
arm-none-eabi-size "$build/thumbv7em/fw_usb_fixed" |
  awk 'NR == 2 && $1 + $2 > 1600 { printf "ffi-check: fw_usb_fixed flash %d B is over its budget of 1600 B\n", $1 + $2; exit 1 }'

# REQ-FFI-CI-009: fw_ci with the responder-only ci features (no Initiator, Profiles or Process Inquiry).
echo "ffi-check: thumbv7em-none-eabihf, fw_ci with ci-device and ci-property-exchange" >&2
cmake -S "$ffi/tests/cmake" -B "$build/thumbv7em-ci-responder" -DCMAKE_TOOLCHAIN_FILE=thumbv7em.cmake \
  -DZMIDI_CHECK_CI_ONLY=ON "-DZMIDI_CHECK_CI_FEATURES=ci-device;ci-property-exchange" $fetch >/dev/null
cmake --build "$build/thumbv7em-ci-responder" >/dev/null
arm-none-eabi-size "$build/thumbv7em-ci-responder/fw_ci" |
  awk 'NR == 2 { printf "ffi-check: fw_ci (ci-device, ci-property-exchange) on Cortex-M4F: flash %d B, RAM %d B\n", $1 + $2, $2 + $3 }'

# The Meson package (root meson.build, crates/zmidi-ffi/meson.build): its version is the crate
# version, the C tests pass on the host, and the bare-metal links build with the cross file.
crate_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ffi/Cargo.toml" | head -n 1)
meson_version=$(sed -n "s/^  version: '\(.*\)',$/\1/p" meson.build | head -n 1)
if [ "$crate_version" != "$meson_version" ]; then
  echo "ffi-check: meson.build version $meson_version is not the zmidi-ffi version $crate_version" >&2
  exit 1
fi
all_parts=io,ump,ble,file,ci,net,net-auth,usb
echo "ffi-check: host (Meson)" >&2
[ -d "$build/meson-host" ] || meson setup "$build/meson-host" -Dfeatures=$all_parts -Dtests=true >/dev/null
meson test -C "$build/meson-host" --print-errorlogs
echo "ffi-check: thumbv7em-none-eabihf (Meson)" >&2
[ -d "$build/meson-thumbv7em" ] || meson setup "$build/meson-thumbv7em" \
  --cross-file "$ffi/tests/meson/thumbv7em.ini" -Dfeatures=$all_parts -Dtests=true >/dev/null
meson compile -C "$build/meson-thumbv7em" >/dev/null
for fw in fw fw_ci fw_usb_fixed; do
  arm-none-eabi-size "$build/meson-thumbv7em/crates/zmidi-ffi/$fw" |
    awk -v job="$fw" 'NR == 2 { printf "ffi-check: %s (Meson) on Cortex-M4F: flash %d B, RAM %d B\n", job, $1 + $2, $2 + $3 }'
done
