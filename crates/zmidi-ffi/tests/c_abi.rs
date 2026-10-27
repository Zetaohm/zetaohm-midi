//! Runs the C tests of `tests/c/` against the staticlib of this crate (DEC-019, REQ-FFI-IO-006,
//! REQ-FFI-ABI-003). `cargo test` builds only the rlib, so this test builds the staticlib itself
//! with `cargo rustc --crate-type staticlib`, in its own target directory.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Builds the staticlib with `features` in the target directory `dir` and gives the path of a copy.
/// cargo-nextest runs each test in its own process, and cargo replaces `debug/libzmidi.a` on each
/// run (also when the build is fresh), so a file lock keeps the builds apart, and the copy is
/// written under another name and then renamed: a C compiler of another process always reads a
/// whole file.
fn build_staticlib(dir: &str, features: &str) -> PathBuf {
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let target = tmp.join(dir);
    std::fs::create_dir_all(tmp).expect("create the target directory");
    let lock =
        std::fs::File::create(tmp.join(format!("{dir}.lock"))).expect("create the lock file");
    lock.lock().expect("lock the staticlib build");
    let st = Command::new(env!("CARGO"))
        .args([
            "rustc",
            "--lib",
            "--crate-type",
            "staticlib",
            "--features",
            features,
        ])
        .arg("--manifest-path")
        .arg(root().join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("run cargo");
    assert!(st.success(), "build of the staticlib ({features})");
    let lib = target.join("libzmidi-c.a");
    let part = target.join(format!("libzmidi-c.a.{}", std::process::id()));
    std::fs::copy(target.join("debug").join("libzmidi.a"), &part).expect("copy the staticlib");
    std::fs::rename(&part, &lib).expect("rename the staticlib copy");
    drop(lock);
    lib
}

/// Builds the staticlib with all parts once for all tests and gives its path.
fn staticlib() -> &'static Path {
    static LIB: OnceLock<PathBuf> = OnceLock::new();
    LIB.get_or_init(|| build_staticlib("c-abi", "std,io,ump,ble,file,ci,net,net-auth,usb"))
}

/// Builds the responder-only ci staticlib (REQ-FFI-CI-009) once and gives its path.
fn responder_staticlib() -> &'static Path {
    static LIB: OnceLock<PathBuf> = OnceLock::new();
    LIB.get_or_init(|| build_staticlib("c-abi-ci-responder", "std,ci-device,ci-property-exchange"))
}

fn cc() -> String {
    std::env::var("CC").unwrap_or_else(|_| "cc".into())
}

/// Compiles `tests/c/<name>.c` as C99 with sanitizers, links the staticlib and runs it.
fn run_c(name: &str) {
    run_c_with(name, staticlib());
}

/// Compiles `tests/c/<name>.c` as C99 with sanitizers, links `lib` and runs it.
fn run_c_with(name: &str, lib: &Path) {
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("c-{name}"));
    let st = Command::new(cc())
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .args([
            "-fsanitize=address,undefined",
            "-fno-sanitize-recover=all",
            "-g",
        ])
        .arg("-I")
        .arg(root().join("include"))
        .arg(root().join("tests").join("c").join(format!("{name}.c")))
        .arg(lib)
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("run the C compiler");
    assert!(st.success(), "compile {name}.c");
    let out = Command::new(&exe).output().expect("run the C test");
    print!("{}", String::from_utf8_lossy(&out.stdout));
    assert!(
        out.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// REQ-FFI-IO-006: the io part through the header, with M2-104 Appendix D and M1 vectors.
#[test]
fn c_io() {
    run_c("io");
}

/// REQ-FFI-UMP-006: the ump part through the header, with M2-104 and M2-115 vectors.
#[test]
fn c_ump() {
    run_c("ump");
}

/// REQ-FFI-BLE-005: the ble part through the header, with BLE-MIDI v1.0 packets.
#[test]
fn c_ble() {
    run_c("ble");
}

/// REQ-FFI-FILE-004: the file part through the header, with M2-116 clips.
#[test]
fn c_file() {
    run_c("file");
}

/// REQ-FFI-CI-007: two MIDI-CI devices through the header (M2-101, M2-103).
#[test]
fn c_ci() {
    run_c("ci");
}

/// REQ-FFI-PE-001 to REQ-FFI-PE-010: the Property Exchange engine between two devices, from C.
#[test]
fn c_pe() {
    run_c("pe");
}

/// REQ-FFI-CI-015: a responder-only ci build (ci-device, ci-property-exchange) through the header
/// (M2-101 5.11, Tables 6, 8, 17; DEC-CI-001).
#[test]
fn c_ci_responder() {
    run_c_with("ci_responder", responder_staticlib());
}

/// REQ-FFI-NET-005: a Network MIDI 2.0 Host and Client in memory through the header (M2-124).
#[test]
fn c_net() {
    run_c("net");
}

/// REQ-FFI-USB-005: the usb part through the header (USB-MIDI 1.0 and USB MIDI 2.0).
#[test]
fn c_usb() {
    run_c("usb");
}

/// REQ-FFI-USB-006 to REQ-FFI-USB-010: the USB descriptor writer and the Group Terminal Block
/// requests through the header (USB-MIDI v2.0 Appendix B and section 6).
#[test]
fn c_usb_desc() {
    run_c("usb_desc");
}

/// REQ-FFI-USB-011, REQ-FFI-USB-012: the fixed USB MIDI function (OPT-038): the host generator of
/// `zmidi_usb_gen.h` and `zmidi_usb_fixed_gtb_setup`.
#[test]
fn c_usb_fixed() {
    run_c("usb_fixed");
}

/// REQ-FFI-ABI-003: the header compiles as C++17 with all parts on.
#[test]
fn header_compiles_as_cpp17() {
    let cxx = std::env::var("CXX").unwrap_or_else(|_| "c++".into());
    let st = Command::new(cxx)
        .args([
            "-std=c++17",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-pedantic",
            "-fsyntax-only",
        ])
        .args([
            "-DZMIDI_IO",
            "-DZMIDI_UMP",
            "-DZMIDI_BLE",
            "-DZMIDI_FILE",
            "-DZMIDI_CI",
            "-DZMIDI_NET",
            "-DZMIDI_NET_AUTH",
            "-DZMIDI_USB",
            "-x",
            "c++",
        ])
        .arg(root().join("include").join("zmidi.h"))
        .status()
        .expect("run the C++ compiler");
    assert!(st.success());
}
