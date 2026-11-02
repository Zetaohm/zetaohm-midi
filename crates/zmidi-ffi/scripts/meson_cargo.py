#!/usr/bin/env python3
"""Cargo steps of the Meson package of the C ABI (DEC-019).

build: builds the static library of zmidi-ffi with the `ffi` profile and copies it to the path
that Meson expects:
    meson_cargo.py build CARGO MANIFEST TARGET_DIR FEATURES RUST_TARGET OUTPUT
native-libs: prints the system libraries that a Rust static library with std needs for the
target (rustc --print native-static-libs), one line of linker arguments:
    meson_cargo.py native-libs RUSTC RUST_TARGET
An empty RUST_TARGET means the host.
"""
import os
import shutil
import subprocess
import sys
import tempfile


def build(cargo, manifest, target_dir, features, target, output):
    cmd = [cargo, "rustc", "--manifest-path", manifest, "--lib", "--crate-type", "staticlib",
           "--profile", "ffi", "--features", features, "--target-dir", target_dir]
    if target:
        cmd += ["--target", target]
    subprocess.run(cmd, check=True)
    out_dir = os.path.join(target_dir, target, "ffi") if target else os.path.join(target_dir, "ffi")
    shutil.copyfile(os.path.join(out_dir, os.path.basename(output)), output)


def native_libs(rustc, target):
    with tempfile.TemporaryDirectory() as d:
        src = os.path.join(d, "probe.rs")
        open(src, "w").close()
        cmd = [rustc, "--crate-type", "staticlib", "--crate-name", "probe", "--print",
               "native-static-libs", "-o", os.path.join(d, "probe.lib"), src]
        if target:
            cmd += ["--target", target]
        r = subprocess.run(cmd, capture_output=True, text=True, check=True)
    for line in (r.stderr + "\n" + r.stdout).splitlines():
        if "native-static-libs:" in line:
            print(line.split("native-static-libs:", 1)[1].strip())
            return
    print("")


if __name__ == "__main__":
    mode, args = sys.argv[1], sys.argv[2:]
    if mode == "build":
        build(*args)
    elif mode == "native-libs":
        native_libs(*args)
    else:
        sys.exit(f"meson_cargo.py: unknown mode {mode}")
