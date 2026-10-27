#!/bin/sh
# RAM of the MIDI-CI types on thumbv7em, with no category feature and with all features
# (OPT-049, step 0). usage: sh tools/size-probe/types.sh
cd "$(dirname "$0")/types" || exit 2
NM=$(ls ~/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-nm 2>/dev/null | head -1)
[ -n "$NM" ] || NM=llvm-nm
for f in none full; do
    if [ $f = none ]; then cargo build -q --release --target thumbv7em-none-eabihf || exit 1
    else cargo build -q --release --target thumbv7em-none-eabihf --features full || exit 1; fi
    "$NM" -S --radix=d target/thumbv7em-none-eabihf/release/libsize_types.rlib 2>/dev/null |
        awk '/ SZ_/ {sub("SZ_", "", $4); print $4, $2 + 0}' | sort > "target/$f.txt"
done
printf '%-26s %10s %10s\n' type "RAM none" "RAM full"
join target/none.txt target/full.txt | awk '{printf "%-26s %10d %10d\n", $1, $2, $3}'
