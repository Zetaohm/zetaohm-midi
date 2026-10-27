/// The web form of [initWeb]: it loads the TypeScript package and its WASM build.
library;

import 'runtime.dart';

/// Loads the WASM build of the package (web only); see `initWeb`.
Future<void> initWeb({String? moduleUrl}) => load(moduleUrl);
