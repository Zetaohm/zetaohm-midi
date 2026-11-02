# Meson package of the C ABI (DEC-019). The root meson.build calls it. Cargo builds the static
# library (`ffi` profile) and the package gives it as the dependency `zmidi`, with the header, the
# `ZMIDI_<PART>` macros of the selected parts and the system libraries that the Rust library needs.
py = import('python').find_installation()
cargo = find_program('cargo')
rustc = find_program('rustc')
cc = meson.get_compiler('c')
helper = files('scripts/meson_cargo.py')

parts = get_option('features')
rust_target = get_option('cargo_target')
if meson.is_cross_build() and rust_target == ''
  error('zmidi: a cross build needs -Dcargo_target=<Rust target triple>, for example thumbv7em-none-eabihf')
endif

# std on a host; the library's own panic handler for bare metal and wasm32-unknown-unknown (no_std).
runtime = get_option('runtime')
if runtime == 'auto'
  no_std = rust_target.contains('-none') or rust_target.startswith('wasm32-unknown-unknown')
  runtime = no_std ? 'panic-handler' : 'std'
endif

lib_name = cc.get_argument_syntax() == 'msvc' ? 'zmidi.lib' : 'libzmidi.a'
zmidi_lib = custom_target(
  'zmidi-rust',
  output: lib_name,
  command: [
    py, helper, 'build', cargo, meson.current_source_dir() / 'Cargo.toml',
    meson.current_build_dir() / 'cargo', ','.join(parts + [runtime]), rust_target, '@OUTPUT@',
  ],
  # Cargo knows when the library is out of date.
  build_always_stale: true,
  console: true,
  install: not meson.is_subproject(),
  install_dir: get_option('libdir'),
)

# The system libraries of a Rust static library with std (rustc --print native-static-libs), without
# duplicates; the compiler on macOS already links libSystem.
native_libs = []
if runtime == 'std'
  probed = run_command(py, helper, 'native-libs', rustc, rust_target, check: true).stdout().strip().split()
  foreach lib : probed
    if lib not in native_libs and not (host_machine.system() == 'darwin' and lib == '-lSystem')
      native_libs += lib
    endif
  endforeach
endif

defines = []
foreach part : parts
  defines += '-DZMIDI_' + part.to_upper().replace('-', '_')
endforeach

zmidi_dep = declare_dependency(
  include_directories: include_directories('include'),
  link_with: zmidi_lib,
  link_args: native_libs,
  compile_args: defines,
)
meson.override_dependency('zmidi', zmidi_dep)

if not meson.is_subproject()
  install_headers('include/zmidi.h', 'include/zmidi_usb_gen.h')
  import('pkgconfig').generate(
    name: 'zmidi',
    description: 'C ABI of zetaohm-midi (MIDI 2.0)',
    version: meson.project_version(),
    libraries: ['-L${libdir}', '-lzmidi'],
    libraries_private: native_libs,
    extra_cflags: defines,
  )
endif

# The checks of scripts/ffi-check.sh: the C tests on a host (as the CMake package runs them), the
# bare-metal links of tests/cmake/fw*.c in a cross build.
if get_option('tests')
  if meson.is_cross_build()
    fw_c_args = ['-Os', '-ffunction-sections', '-fdata-sections']
    fw_link_args = ['-nostartfiles', '-Wl,-e,Reset_Handler', '-Wl,--gc-sections', '-Wl,-z,noexecstack']
    foreach fw : ['fw', 'fw_ci', 'fw_usb_fixed']
      executable(fw, 'tests/cmake' / fw + '.c',
        include_directories: include_directories('tests/c'),
        c_args: fw_c_args, link_args: fw_link_args, dependencies: zmidi_dep)
    endforeach
  else
    c_args = cc.get_supported_arguments('-Wall', '-Wextra', '-Werror', '-pedantic')
    foreach t : ['io', 'ump', 'ble', 'file', 'ci', 'pe', 'net', 'usb', 'usb_desc', 'usb_fixed']
      exe = executable('c_' + t, 'tests/c' / t + '.c',
        c_args: c_args, override_options: ['c_std=c99'], dependencies: zmidi_dep)
      test('c_' + t, exe)
    endforeach
  endif
endif
