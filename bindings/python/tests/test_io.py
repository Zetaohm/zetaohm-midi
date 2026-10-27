"""MIDI 1.0 byte stream to UMP and back (REQ-PY-IO-*): M2-104 Appendix D, M1."""

import pytest

import zetaohm_midi as zm


def test_bytes_to_ump_midi1_and_running_status() -> None:
    r = zm.BytesToUmp(group=3)
    assert r.feed(bytes([0x90, 0x3C, 0x64])) == [[0x23903C64]]
    assert r.feed(bytes([0x3E, 0x40])) == [[0x23903E40]]
    # Timing Clock between data bytes comes out at once.
    assert r.feed(bytes([0x90, 0x3C, 0xF8, 0x64])) == [[0x13F80000], [0x23903C64]]


def test_bytes_to_ump_midi2_and_sysex() -> None:
    assert zm.BytesToUmp(0, 2).feed(bytes([0x90, 0x3C, 0x64])) == [[0x40903C00, 0xC9240000]]
    sysex = bytes([0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7])
    assert zm.BytesToUmp().feed(sysex) == [[0x30047E7F, 0x06010000]]


def test_ump_to_bytes_running_status_and_refresh() -> None:
    w = zm.UmpToBytes(running_status=True)
    assert w.write([0x20903C64]) == bytes([0x90, 0x3C, 0x64])
    assert w.write([0x20903E40]) == bytes([0x3E, 0x40])
    w.refresh_status()
    assert w.write([0x20903E40]) == bytes([0x90, 0x3E, 0x40])


def test_bad_values() -> None:
    with pytest.raises(zm.InvalidValueError):
        zm.BytesToUmp(16)
    with pytest.raises(zm.InvalidValueError):
        zm.BytesToUmp(0, 3)
