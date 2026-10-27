/// MIDI 2.0 for Dart: UMP, MIDI-CI with Property Exchange, Network MIDI 2.0, BLE-MIDI, USB-MIDI
/// and MIDI Clip Files, through the zetaohm-midi C ABI (DEC-019): dart:ffi on the native
/// platforms, the WASM build on the web (call [initWeb] first there).
library;

export 'src/ble.dart';
export 'src/ci.dart'
    hide CiConfig, ciHandle, newCiEvent, newCiProcessEvent, newCiProfileEvent;
export 'src/errors.dart';
export 'src/file.dart';
export 'src/init.dart' if (dart.library.js_interop) 'src/web/init.dart';
export 'src/io.dart';
export 'src/net.dart' hide netAuthSharedSecret, netAuthUser, newNetEvent;
export 'src/pe.dart' hide PeHeader, newPeEvent, newPeSubscription;
export 'src/ump.dart';
export 'src/usb.dart';
