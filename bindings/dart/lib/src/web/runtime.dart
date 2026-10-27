/// The web runtime of the package: it loads the TypeScript package of zetaohm-midi (the WASM build
/// of the C ABI, DEC-019) through dart:js_interop, maps its errors to the exceptions of the package
/// and converts the values. The entry library does not export this library.
library;

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import '../checks.dart';
import '../errors.dart';

/// The location of the TypeScript package inside this package: `lib/js/dist/index.js`, served at
/// `packages/zetaohm_midi/js/dist/index.js`.
const modulePath = 'packages/zetaohm_midi/js/dist/index.js';

JSObject? _module;

/// The module object of the TypeScript package. Throws before [load].
JSObject get ts =>
    _module ??
    (throw const RejectedException(
      'call initWeb() before you use the package on the web',
    ));

/// Loads the TypeScript package from [moduleUrl] (default: [modulePath] beside the page), sets up
/// its WASM build and checks its C ABI version. A second call does nothing.
Future<void> load(String? moduleUrl) async {
  if (_module != null) {
    return;
  }
  final url = moduleUrl ?? Uri.base.resolve(modulePath).toString();
  final m = await importModule(url.toJS).toDart;
  final version = await guardAsync(
    () => m.callMethod<JSPromise<JSNumber>>('init'.toJS).toDart,
  );
  checkAbiVersion(version.toDartInt);
  _module = m;
}

/// The class [name] of the TypeScript package.
JSFunction tsClass(String name) => ts.getProperty<JSFunction>(name.toJS);

/// Calls the function [name] of the TypeScript package with [args].
R tsCall<R extends JSAny?>(String name, List<JSAny?> args) =>
    guard(() => ts.callMethodVarArgs<R>(name.toJS, args));

/// A new object of the class [name] of the TypeScript package.
JSObject tsNew(String name, List<JSAny?> args) =>
    guard(() => tsClass(name).callAsConstructorVarArgs<JSObject>(args));

/// Calls the method [name] of [o] with [args].
R call<R extends JSAny?>(
  JSObject o,
  String name, [
  List<JSAny?> args = const [],
]) => guard(() => o.callMethodVarArgs<R>(name.toJS, args));

/// The property [name] of [o].
R prop<R extends JSAny?>(JSObject o, String name) =>
    guard(() => o.getProperty<R>(name.toJS));

/// Runs [f]; maps an error of the TypeScript package to the exception of the package.
T guard<T>(T Function() f) {
  try {
    return f();
  } catch (e) {
    final m = mapError(e);
    if (m == null) {
      rethrow;
    }
    throw m;
  }
}

/// The async form of [guard].
Future<T> guardAsync<T>(Future<T> Function() f) async {
  try {
    return await f();
  } catch (e) {
    final m = mapError(e);
    if (m == null) {
      rethrow;
    }
    throw m;
  }
}

/// The exception of a JavaScript error of the TypeScript package (MidiError, InvalidValueError,
/// MalformedError, RejectedError), or null for another error.
MidiException? mapError(Object e) {
  if (e is MidiException || e is Error || e is Exception) {
    return null;
  }
  if (!e.isA<JSObject>()) {
    return null;
  }
  final o = e as JSObject;
  final name = o.getProperty<JSAny?>('name'.toJS);
  final message = o.getProperty<JSAny?>('message'.toJS);
  if (!name.isA<JSString>() || !message.isA<JSString>()) {
    return null;
  }
  final text = (message as JSString).toDart;
  return switch ((name as JSString).toDart) {
    'InvalidValueError' => InvalidValueException(text),
    'MalformedError' => MalformedException(text),
    'RejectedError' => RejectedException(text),
    'MidiError' => MidiException(text),
    _ => null,
  };
}

/// A JavaScript array of the numbers [values].
JSArray<JSNumber> jsInts(List<int> values) =>
    [for (final v in values) v.toJS].toJS;

/// A JavaScript Uint8Array copy of [data].
JSUint8Array jsBytes(List<int> data) =>
    (data is Uint8List ? data : Uint8List.fromList(data)).toJS;

/// The Dart ints of a JavaScript array of numbers.
List<int> ints(JSAny? a) => [
  for (final v in (a as JSArray<JSNumber>).toDart) v.toDartInt,
];

/// The UMPs of a JavaScript array of word arrays.
List<List<int>> umps(JSAny? a) => [
  for (final u in (a as JSArray<JSAny?>).toDart) ints(u),
];

/// A copy of a JavaScript Uint8Array.
Uint8List bytes(JSAny? a) => Uint8List.fromList((a as JSUint8Array).toDart);

/// The Dart int of a JavaScript number, or null.
int? intOrNull(JSAny? a) =>
    a.isA<JSNumber>() ? (a as JSNumber).toDartInt : null;

/// The Dart int of a JavaScript number.
int intOf(JSAny? a) => (a as JSNumber).toDartInt;

/// The Dart bool of a JavaScript boolean.
bool boolOf(JSAny? a) => (a as JSBoolean).toDart;

/// The Dart string of a JavaScript string, or null.
String? stringOrNull(JSAny? a) =>
    a.isA<JSString>() ? (a as JSString).toDart : null;

/// A JavaScript object with the properties [fields]; a null value leaves the property out.
JSObject jsObject(Map<String, JSAny?> fields) {
  final o = JSObject();
  fields.forEach((k, v) {
    if (v != null) {
      o.setProperty(k.toJS, v);
    }
  });
  return o;
}
