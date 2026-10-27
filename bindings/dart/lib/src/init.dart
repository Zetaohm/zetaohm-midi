/// The set up of the package. The native form has nothing to load: the first call loads the native
/// library.
library;

/// Loads the WASM build of the package on the web: the TypeScript package of zetaohm-midi at
/// [moduleUrl] (default: `packages/zetaohm_midi/js/dist/index.js` beside the page). Call it once,
/// and wait for it, before the other calls on the web. On the native platforms it does nothing,
/// so a program can call it on every platform.
Future<void> initWeb({String? moduleUrl}) async {}
