# CMake package of the zetaohm-midi C ABI (DEC-019). Use it with add_subdirectory():
#
#   set(ZMIDI_FEATURES io)                 # the parts of the C ABI (Cargo features)
#   add_subdirectory(path/to/zetaohm-midi/crates/zmidi-ffi zmidi)
#   target_link_libraries(app PRIVATE zmidi::zmidi)
#
# Bare metal: set Rust_CARGO_TARGET (for example thumbv7em-none-eabihf) in the toolchain file. The
# package then adds the feature panic-handler; on a host it adds the feature std.
cmake_minimum_required(VERSION 3.22)
project(zmidi LANGUAGES C)

set(ZMIDI_FEATURES "io" CACHE STRING "Parts of the C ABI (Cargo features of zmidi-ffi)")

if(NOT COMMAND corrosion_import_crate)
  include(FetchContent)
  FetchContent_Declare(Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG v0.6.1)
  FetchContent_MakeAvailable(Corrosion)
endif()

if(CMAKE_CROSSCOMPILING)
  set(_zmidi_build panic-handler)
  set(_zmidi_no_std NO_STD)
else()
  set(_zmidi_build std)
  set(_zmidi_no_std "")
endif()

corrosion_import_crate(
  MANIFEST_PATH ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml
  CRATES zmidi-ffi
  OVERRIDE_CRATE_TYPE zmidi=staticlib
  CRATE_TYPES staticlib
  FEATURES ${ZMIDI_FEATURES} ${_zmidi_build}
  PROFILE ffi
  ${_zmidi_no_std})

target_include_directories(zmidi INTERFACE ${CMAKE_CURRENT_SOURCE_DIR}/include)
foreach(_part IN LISTS ZMIDI_FEATURES)
  string(TOUPPER "${_part}" _part)
  string(REPLACE "-" "_" _part "${_part}")
  target_compile_definitions(zmidi INTERFACE ZMIDI_${_part})
endforeach()
add_library(zmidi::zmidi ALIAS zmidi)
