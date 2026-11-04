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
  --features io,ump,ble,file,ci,net,net-auth,usb,midi1
cargo check --quiet -p zmidi-ffi --lib --no-default-features --target wasm32-unknown-unknown --features midi1

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

# REQ-FFI-ABI-005, M7 of DEC-022: fw_midi1 with the feature midi1 only: a flash budget, and no
# symbol of a MIDI 2.0 module (the rule of scripts/size-check.py). 11,244 B on 2026-11-04: the DIN
# reader, writer, mode, 16 controller and 16 parameter states, and the io, ble and usb parts.
echo "ffi-check: thumbv7em-none-eabihf, fw_midi1 with midi1" >&2
cmake -S "$ffi/tests/cmake" -B "$build/thumbv7em-midi1" -DCMAKE_TOOLCHAIN_FILE=thumbv7em.cmake \
  -DZMIDI_CHECK_MIDI1_ONLY=ON $fetch >/dev/null
cmake --build "$build/thumbv7em-midi1" >/dev/null
fw_midi1="$build/thumbv7em-midi1/fw_midi1"
arm-none-eabi-size "$fw_midi1" |
  awk 'NR == 2 { printf "ffi-check: fw_midi1 (midi1) on Cortex-M4F: flash %d B, RAM %d B\n", $1 + $2, $2 + $3 }'
arm-none-eabi-size "$fw_midi1" |
  awk -v budget=12288 'NR == 2 && $1 + $2 > budget { printf "ffi-check: fw_midi1 flash %d B is over its budget of %d B\n", $1 + $2, budget; exit 1 }'
midi2=$(arm-none-eabi-nm -C --defined-only "$fw_midi1" |
  grep -E 'zmidi_ump::(translate::|channel_voice2|flex_data|ump_stream|data128)|zmidi_usb::(ms2::|gtb::|ump_usb::)|zmidi_ci|zmidi_net|Midi1ToMidi2|midi2_to_midi1' |
  grep -v 'ms2::check_max_packet' || true)
if [ -n "$midi2" ]; then
  echo "ffi-check: fw_midi1 has MIDI 2.0 code:" >&2
  echo "$midi2" >&2
  exit 1
fi

# The Meson package (root meson.build, crates/zmidi-ffi/meson.build): its version is the crate
# version, the C tests pass on the host, and the bare-metal links build with the cross file.
crate_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ffi/Cargo.toml" | head -n 1)
meson_version=$(sed -n "s/^  version: '\(.*\)',$/\1/p" meson.build | head -n 1)
if [ "$crate_version" != "$meson_version" ]; then
  echo "ffi-check: meson.build version $meson_version is not the zmidi-ffi version $crate_version" >&2
  exit 1
fi
all_parts=io,ump,ble,file,ci,net,net-auth,usb,midi1
# The bare-metal links keep the MIDI 2.0 part list (rule R0 of DEC-022), as in the CMake package.
fw_parts=io,ump,ble,file,ci,net,net-auth,usb
echo "ffi-check: host (Meson)" >&2
# An existing build directory gets the part list again, so that a new part reaches it.
if [ -d "$build/meson-host" ]; then
  meson configure "$build/meson-host" -Dfeatures=$all_parts >/dev/null
else
  meson setup "$build/meson-host" -Dfeatures=$all_parts -Dtests=true >/dev/null
fi
meson test -C "$build/meson-host" --print-errorlogs
echo "ffi-check: thumbv7em-none-eabihf (Meson)" >&2
if [ -d "$build/meson-thumbv7em" ]; then
  meson configure "$build/meson-thumbv7em" -Dfeatures=$fw_parts >/dev/null
else
  meson setup "$build/meson-thumbv7em" --cross-file "$ffi/tests/meson/thumbv7em.ini" -Dfeatures=$fw_parts \
    -Dtests=true >/dev/null
fi
meson compile -C "$build/meson-thumbv7em" >/dev/null
for fw in fw fw_ci fw_usb_fixed; do
  arm-none-eabi-size "$build/meson-thumbv7em/crates/zmidi-ffi/$fw" |
    awk -v job="$fw" 'NR == 2 { printf "ffi-check: %s (Meson) on Cortex-M4F: flash %d B, RAM %d B\n", job, $1 + $2, $2 + $3 }'
done
