"""UMP functions (REQ-PY-UMP-*): M2-104 Table 4, 7.4 figures, Appendix D.2, M2-115 Tables 5 and 8."""

import pytest

import zetaohm_midi as zm


def test_word_count_table_4() -> None:
    want = [1, 1, 1, 2, 2, 4, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4]
    assert [zm.word_count(mt << 28) for mt in range(16)] == want


def test_midi2_writers_match_figures() -> None:
    assert zm.midi2_note_on(0, 0, 60, 0xC000) == [0x40903C00, 0xC0000000]
    assert zm.midi2_control_change(1, 0, 7, 0xFFFFFFFF) == [0x41B00700, 0xFFFFFFFF]
    assert zm.midi2_program_change(0, 0, 5, (1, 2)) == [0x40C00001, 0x05000102]
    assert zm.midi2_pitch_bend(2, 15, 0x80000000) == [0x42EF0000, 0x80000000]


def test_writers_reject_out_of_range() -> None:
    with pytest.raises(zm.InvalidValueError):
        zm.midi2_note_on(16, 0, 60, 1)
    with pytest.raises(zm.InvalidValueError):
        zm.midi2_note_on(0, 0, 128, 1)
    assert issubclass(zm.InvalidValueError, zm.MidiError)


def test_parse_gives_fields() -> None:
    u = zm.parse([0x43923C00, 0xC0000000])
    assert (u.message_type, u.group, u.status, u.channel, u.decoded) == (4, 3, 9, 2, True)
    assert u.words == [0x43923C00, 0xC0000000] and len(u) == 2
    with pytest.raises(zm.MalformedError):
        zm.parse([0x43923C00])


def test_midi2_to_midi1_appendix_d2() -> None:
    assert zm.midi2_to_midi1(zm.midi2_note_on(0, 0, 60, 0xC000)) == [0x20903C60]
    assert zm.midi2_to_midi1(zm.midi2_note_on(0, 0, 60, 0x0100)) == [0x20903C01]
    assert zm.midi2_to_midi1([0x20903C64]) is None


def test_scaling_m2_115() -> None:
    assert [zm.upscale(v, 7, 16) for v in (0x00, 0x40, 0x46, 0x60, 0x7F)] == [
        0,
        0x8000,
        0x8C30,
        0xC104,
        0xFFFF,
    ]
    assert [zm.downscale(v, 16, 7) for v in (0x1400, 0x8000, 0xAEBA, 0xFFFF)] == [
        0x0A,
        0x40,
        0x57,
        0x7F,
    ]
    with pytest.raises(zm.InvalidValueError):
        zm.upscale(0x80, 7, 16)
