# Meson package of the C ABI of zetaohm-midi (crates/zmidi-ffi, DEC-019). Cargo needs the whole
# workspace, so the repository root is the Meson project: use the repository as a subproject and
# `dependency('zmidi')`. Usage: crates/zmidi-ffi/README.md, section "Build with Meson".
project(
  'zmidi',
  'c',
  # The version of zmidi-ffi (crates/zmidi-ffi/Cargo.toml); scripts/ffi-check.sh compares them.
  version: '0.1.0-alpha.1',
  license: 'MIT',
  meson_version: '>= 1.1.0',
)

subdir('crates/zmidi-ffi')
