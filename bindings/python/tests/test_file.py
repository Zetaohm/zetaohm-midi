"""MIDI Clip Files (REQ-PY-FILE-*): M2-116."""

import struct

import pytest

import zetaohm_midi as zm


def words(data: bytes) -> list[int]:
    return list(struct.unpack(f">{(len(data) - 8) // 4}I", data[8:]))


def test_minimal_clip() -> None:
    w = zm.ClipWriter(96)
    w.start(0)
    data = w.finish(0)
    assert data[:8] == b"SMF2CLIP"
    assert words(data) == [
        0x00400000,
        0x00300060,
        0x00400000,
        0xF0200000,
        0,
        0,
        0,
        0x00400000,
        0xF0210000,
        0,
        0,
        0,
    ]


def test_write_and_read_back() -> None:
    note_on, note_off = [0x40903C00, 0xFFFF0000], [0x40803C00, 0]
    w = zm.ClipWriter(
        96,
        tempo_10ns=50_000_000,
        time_signature=(4, 2, 8),
        profiles=[(bytes([0x7E, 0, 1, 2, 3]), "group", 0, 5)],
    )
    w.start(0)
    w.event(0, note_on)
    w.event(96, note_off)
    with pytest.raises(zm.RejectedError):
        w.event(10, note_on)
    with pytest.raises(zm.InvalidValueError):
        w.event(96, note_on[:1])
    events = zm.read_clip(w.finish(192))
    notes = [
        (t, ws) for section, t, ws in events if section == 1 and ws[0] in (note_on[0], note_off[0])
    ]
    assert notes == [(0, note_on), (96, note_off)]


def test_bad_input() -> None:
    with pytest.raises(zm.MalformedError):
        zm.read_clip(b"SMF1CLIP")
    with pytest.raises(zm.InvalidValueError):
        zm.ClipWriter(0)
