// Builds the zmidi-ffi C ABI as a dynamic library with cargo and gives it to Dart as the code asset
// of lib/src/bindings.g.dart (DEC-019). It needs a Rust toolchain with the target of the build.
import 'dart:io';

import 'package:code_assets/code_assets.dart';
import 'package:hooks/hooks.dart';

/// The Rust target triple of each Dart target that the package builds for.
final _triples = {
  (OS.macOS, Architecture.arm64): 'aarch64-apple-darwin',
  (OS.macOS, Architecture.x64): 'x86_64-apple-darwin',
  (OS.linux, Architecture.arm64): 'aarch64-unknown-linux-gnu',
  (OS.linux, Architecture.x64): 'x86_64-unknown-linux-gnu',
  (OS.windows, Architecture.arm64): 'aarch64-pc-windows-msvc',
  (OS.windows, Architecture.x64): 'x86_64-pc-windows-msvc',
  (OS.android, Architecture.arm64): 'aarch64-linux-android',
  (OS.android, Architecture.arm): 'armv7-linux-androideabi',
  (OS.android, Architecture.x64): 'x86_64-linux-android',
};

void main(List<String> args) async {
  await build(args, (input, output) async {
    if (!input.config.buildCodeAssets) {
      return;
    }
    final code = input.config.code;
    final triple = _triples[(code.targetOS, code.targetArchitecture)];
    if (triple == null) {
      throw UnsupportedError(
        'zetaohm_midi has no build for ${code.targetOS} ${code.targetArchitecture}',
      );
    }
    final workspace = input.packageRoot.resolve('../../');
    final targetDir = input.outputDirectoryShared.resolve('cargo/');
    final result = await Process.run(
      'cargo',
      [
        'rustc',
        '--quiet',
        '--manifest-path',
        workspace.resolve('Cargo.toml').toFilePath(),
        '-p',
        'zmidi-ffi',
        '--profile',
        'ffi',
        '--target',
        triple,
        '--no-default-features',
        '--features',
        'std,io,ump,ble,file,ci,net,net-auth,usb',
        '--crate-type',
        'cdylib',
      ],
      environment: {'CARGO_TARGET_DIR': targetDir.toFilePath()},
    );
    if (result.exitCode != 0) {
      throw ProcessException(
        'cargo',
        ['rustc', '-p', 'zmidi-ffi'],
        '${result.stdout}${result.stderr}',
        result.exitCode,
      );
    }
    final file = targetDir.resolve(
      '$triple/ffi/${code.targetOS.dylibFileName('zmidi')}',
    );
    output.assets.code.add(
      CodeAsset(
        package: input.packageName,
        name: 'src/bindings.g.dart',
        linkMode: DynamicLoadingBundled(),
        file: file,
      ),
    );
    // hooks_runner hashes a directory only by the names of its children, so each Rust source file
    // and manifest is a dependency: a change in one of them runs the hook again.
    final sources = Directory.fromUri(workspace.resolve('crates/'))
        .listSync(recursive: true)
        .whereType<File>()
        .where((f) => f.path.endsWith('.rs') || f.path.endsWith('Cargo.toml'))
        .map((f) => f.uri);
    output.dependencies.addAll([
      workspace.resolve('Cargo.toml'),
      workspace.resolve('Cargo.lock'),
      workspace.resolve('crates/'),
      ...sources,
    ]);
  });
}
