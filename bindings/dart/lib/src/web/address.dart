/// The IP address type of the web forms, which have no dart:io: the part of dart:io
/// InternetAddress that NetAddress uses (IPv4 dotted text and IPv6 text, RFC 4291 2.2).
library;

import 'dart:typed_data';

/// The kind of an IP address.
final class InternetAddressType {
  const InternetAddressType._(this.name);

  /// The name of the kind.
  final String name;

  /// An IPv4 address.
  // The names of dart:io.
  // ignore: constant_identifier_names
  static const IPv4 = InternetAddressType._('IPv4');

  /// An IPv6 address.
  // ignore: constant_identifier_names
  static const IPv6 = InternetAddressType._('IPv6');

  @override
  String toString() => name;
}

/// An IPv4 or IPv6 address, from its text or its bytes.
final class InternetAddress {
  InternetAddress._(this.address, this.rawAddress);

  /// The address of [text] (IPv4 dotted or IPv6), or null when it is not an IP address.
  static InternetAddress? tryParse(String text) {
    final raw = _ipv4(text) ?? _ipv6(text);
    return raw == null
        ? null
        : InternetAddress._(text, Uint8List.fromList(raw));
  }

  /// The address of 4 (IPv4) or 16 (IPv6) bytes.
  factory InternetAddress.fromRawAddress(Uint8List raw) {
    if (raw.length == 4) {
      return InternetAddress._(raw.join('.'), Uint8List.fromList(raw));
    }
    if (raw.length == 16) {
      return InternetAddress._(_format(raw), Uint8List.fromList(raw));
    }
    throw ArgumentError('an IP address has 4 or 16 bytes, not ${raw.length}');
  }

  /// The text of the address.
  final String address;

  /// The bytes of the address: 4 (IPv4) or 16 (IPv6).
  final Uint8List rawAddress;

  /// The kind of the address.
  InternetAddressType get type => rawAddress.length == 4
      ? InternetAddressType.IPv4
      : InternetAddressType.IPv6;

  @override
  bool operator ==(Object other) =>
      other is InternetAddress &&
      other.rawAddress.length == rawAddress.length &&
      [
        for (var i = 0; i < rawAddress.length; i++)
          other.rawAddress[i] == rawAddress[i],
      ].every((x) => x);

  @override
  int get hashCode => Object.hashAll(rawAddress);

  @override
  String toString() => 'InternetAddress($address, ${type.name})';
}

final _decimal = RegExp(r'^\d{1,3}$');
final _hex = RegExp(r'^[0-9a-fA-F]{1,4}$');

List<int>? _ipv4(String s) {
  final parts = s.split('.');
  if (parts.length != 4 ||
      !parts.every((p) => _decimal.hasMatch(p) && int.parse(p) <= 255)) {
    return null;
  }
  return [for (final p in parts) int.parse(p)];
}

List<int>? _groups(String h) {
  if (h.isEmpty) {
    return [];
  }
  final out = <int>[];
  final parts = h.split(':');
  for (var i = 0; i < parts.length; i++) {
    final p = parts[i];
    if (i == parts.length - 1 && p.contains('.')) {
      final v4 = _ipv4(p);
      if (v4 == null) {
        return null;
      }
      out
        ..add((v4[0] << 8) | v4[1])
        ..add((v4[2] << 8) | v4[3]);
    } else if (_hex.hasMatch(p)) {
      out.add(int.parse(p, radix: 16));
    } else {
      return null;
    }
  }
  return out;
}

List<int>? _ipv6(String s) {
  final halves = s.split('::');
  if (halves.length > 2) {
    return null;
  }
  final head = _groups(halves[0]);
  final tail = halves.length == 2 ? _groups(halves[1]) : <int>[];
  if (head == null || tail == null) {
    return null;
  }
  final fill = 8 - head.length - tail.length;
  if ((halves.length == 2 && fill < 1) || (halves.length == 1 && fill != 0)) {
    return null;
  }
  return [
    for (final g in [...head, ...List.filled(fill, 0), ...tail]) ...[
      g >> 8,
      g & 0xff,
    ],
  ];
}

/// The RFC 5952 text of 16 bytes: the longest run of two or more zero groups is "::".
String _format(Uint8List b) {
  final g = [for (var i = 0; i < 16; i += 2) (b[i] << 8) | b[i + 1]];
  var best = -1;
  var bestLen = 1;
  for (var i = 0; i < 8;) {
    if (g[i] != 0) {
      i++;
      continue;
    }
    var j = i;
    while (j < 8 && g[j] == 0) {
      j++;
    }
    if (j - i > bestLen) {
      best = i;
      bestLen = j - i;
    }
    i = j;
  }
  String hex(List<int> xs) => xs.map((x) => x.toRadixString(16)).join(':');
  return best < 0
      ? hex(g)
      : '${hex(g.sublist(0, best))}::${hex(g.sublist(best + bestLen))}';
}
