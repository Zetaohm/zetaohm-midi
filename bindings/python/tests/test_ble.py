"""BLE-MIDI v1.0 packets (REQ-PY-BLE-*): sections 7 and 8."""

import pytest

import zetaohm_midi as zm


def hdr(t: int) -> int:
    return 0x80 | (t >> 7)


def ts(t: int) -> int:
    return 0x80 | (t & 0x7F)


def test_reader_section_7() -> None:
    r = zm.BleReader()
    umps, dropped = r.read(
        bytes([hdr(0x0A55), ts(0x0A55), 0x90, 0x3C, 0x64, ts(0x0A57), 0x80, 0x3C, 0x00])
    )
    assert umps == [(0x0A55, [0x20903C64]), (0x0A57, [0x20803C00])] and dropped == 0
    umps, _ = r.read(bytes([hdr(5), ts(5), 0x90, 0x3C, 0x64, 0x40, 0x64]))
    assert umps[1] == (5, [0x20904064])
    assert r.read(bytes([0x10, 0x90, 0x3C, 0x64])) == ([], 1)


def test_writer_section_7_and_running_status() -> None:
    w = zm.BleWriter(23)
    assert w.packet_len == 20
    packet, taken = w.write_packet([(0x0A55, [0x20903C64])])
    assert packet == bytes([hdr(0x0A55), ts(0x0A55), 0x90, 0x3C, 0x64]) and taken == 1
    w.reset()  # times never decrease (section 7); start a new stream
    packet, taken = w.write_packet([(5, [0x20903C64]), (5, [0x20904064])])
    assert packet == bytes([hdr(5), ts(5), 0x90, 0x3C, 0x64, 0x40, 0x64]) and taken == 2
    assert w.write_packet([]) == (b"", 0)


def test_sysex_round_trip_over_packets() -> None:
    umps = [
        (1, [0x30160001, 0x02030405]),
        (1, [0x30260607, 0x08090A0B]),
        (1, [0x30360C0D, 0x0E0F1011]),
    ]
    w, r = zm.BleWriter(23), zm.BleReader()
    got: list[list[int]] = []
    left = list(umps)
    for _ in range(16):
        packet, taken = w.write_packet(left)
        left = left[taken:]
        if not packet:
            break
        got += [words for _, words in r.read(packet)[0]]
    assert got == [words for _, words in umps]


def test_bad_values() -> None:
    with pytest.raises(zm.InvalidValueError):
        zm.BleWriter(22)
    with pytest.raises(zm.InvalidValueError):
        zm.BleReader(16)
