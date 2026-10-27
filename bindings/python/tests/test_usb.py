"""USB-MIDI 1.0 Event Packets and USB MIDI 2.0 UMP words (REQ-PY-USB-*)."""

import pytest

import zetaohm_midi as zm


def test_event_packets_at_setting_0() -> None:
    link = zm.UsbLink()
    assert link.setting == 0 and not link.is_ump
    assert link.read(bytes([0x09, 0x90, 0x3C, 0x64])) == ([[0x20903C64]], 0)
    sysex = bytes([0x04, 0xF0, 0x7E, 0x7F, 0x07, 0x06, 0x01, 0xF7])
    assert link.read(sysex) == ([[0x30047E7F, 0x06010000]], 0)
    assert link.write_packet([[0x20903C64]]) == (bytes([0x09, 0x90, 0x3C, 0x64]), 1)


def test_ump_words_at_setting_1_and_reset() -> None:
    link = zm.UsbLink()
    link.set_alternate_setting(1, 0x0001, 64)
    assert link.setting == 1 and link.is_ump
    data = bytes([0x00, 0x3C, 0x90, 0x40, 0x00, 0x00, 0x00, 0xC0])
    assert link.read(data) == ([[0x40903C00, 0xC0000000]], 0)
    assert link.write_packet([[0x40903C00, 0xC0000000]]) == (data, 1)
    link.usb_reset()
    assert link.setting == 0


def test_midi2_protocol_needs_no_extra_setup() -> None:
    link = zm.UsbLink(protocol=2)
    assert link.read(bytes([0x09, 0x90, 0x3C, 0x64])) == ([[0x40903C00, 0xC9240000]], 0)


def test_bad_values() -> None:
    with pytest.raises(zm.InvalidValueError):
        zm.UsbLink(max_packet=2)
    with pytest.raises(zm.InvalidValueError):
        zm.UsbLink(protocol=7)


# USB descriptors and Group Terminal Block requests (REQ-PY-USB-002 to REQ-PY-USB-004). Vectors:
# USB-MIDI v2.0 Appendix B, Tables B-3 to B-20 (the function), B-21 and B-22 (the Group Terminal
# Block set), B-23 and B-24 (the requests); the same as crates/zmidi-ffi/tests/c/usb_desc.c.
APPENDIX_B = bytes(
    [0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00]  # B-3 standard AC interface
    + [0x09, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x01, 0x01]  # B-4 AC header
    + [0x09, 0x04, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x00]  # B-5 MS interface, alt 0
    + [0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00]  # B-6 MS header, bcdMSC 0x0100
    + [0x06, 0x24, 0x02, 0x01, 0x01, 0x00]  # B-7 Embedded MIDI IN Jack 1
    + [0x06, 0x24, 0x02, 0x02, 0x02, 0x00]  # B-8 External MIDI IN Jack 2
    + [0x09, 0x24, 0x03, 0x01, 0x03, 0x01, 0x02, 0x01, 0x00]  # B-9 Embedded MIDI OUT Jack 3
    + [0x09, 0x24, 0x03, 0x02, 0x04, 0x01, 0x01, 0x01, 0x00]  # B-10 External MIDI OUT Jack 4
    + [0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00]  # B-11 bulk OUT endpoint 1
    + [0x05, 0x25, 0x01, 0x01, 0x01]  # B-12 MS_GENERAL, Jack 1
    + [0x09, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00]  # B-13 bulk IN endpoint 1
    + [0x05, 0x25, 0x01, 0x01, 0x03]  # B-14 MS_GENERAL, Jack 3
    + [0x09, 0x04, 0x01, 0x01, 0x02, 0x01, 0x03, 0x00, 0x00]  # B-15 MS interface, alt 1
    + [0x07, 0x24, 0x01, 0x00, 0x02, 0x07, 0x00]  # B-16 MS header, bcdMSC 0x0200
    + [0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00]  # B-17 bulk OUT endpoint 1
    + [0x05, 0x25, 0x02, 0x01, 0x01]  # B-18 MS_GENERAL_2_0, block 1
    + [0x07, 0x05, 0x81, 0x03, 0x40, 0x00, 0x01]  # B-19 interrupt IN endpoint 1
    + [0x05, 0x25, 0x02, 0x01, 0x01]  # B-20 MS_GENERAL_2_0, block 1
)
B21_B22 = bytes(
    [0x05, 0x26, 0x01, 0x12, 0x00]
    + [0x0D, 0x26, 0x02, 0x01, 0x00, 0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00]
)
B23 = bytes([0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x05, 0x00])
B24 = bytes([0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x12, 0x00])


def b22(id: int = 1, first_group: int = 0, group_count: int = 1) -> zm.UsbBlock:
    """The block of Table B-22: bidirectional, iBlockItem 4, bMIDIProtocol 0, wMaxInputBandwidth 1."""
    return zm.UsbBlock(
        id, first_group, group_count=group_count, string=4, protocol=0x00, max_input_bandwidth=1
    )


def out_ep(**kw: object) -> zm.UsbEndpoint:
    return zm.UsbEndpoint("out", 1, **kw)  # type: ignore[arg-type]


def appendix_b() -> zm.UsbFunction:
    s = zm.UsbSetting(
        [out_ep(), zm.UsbEndpoint("in", 1, transfer="interrupt", interval=1)], blocks=[b22()]
    )
    return zm.UsbFunction(0, 1, settings=[s])


def test_function_of_appendix_b() -> None:
    f = appendix_b()
    assert f.descriptors() == APPENDIX_B
    assert f.endpoints() == [
        (0, 0x01, 2, 0, 64),
        (0, 0x81, 2, 0, 64),
        (1, 0x01, 2, 0, 64),
        (1, 0x81, 3, 1, 64),
    ]
    assert [
        f.group_mask(0, "out"),
        f.group_mask(1, "out"),
        f.group_mask(1, "in"),
        f.group_mask(2, "out"),
    ] == [
        0xFFFF,
        1,
        1,
        0,
    ]
    assert f.gtb_set(1) == B21_B22
    with pytest.raises(zm.InvalidValueError):
        f.gtb_set(0)


def test_group_terminal_block_requests() -> None:
    f = appendix_b()
    assert f.answer_setup(B23) == ("answered", B21_B22[:5])
    assert f.answer_setup(B24) == ("answered", B21_B22)
    other = bytearray(B24)
    other[4] = 0x02  # another interface
    assert f.answer_setup(bytes(other)) == ("stall", b"")
    assert f.answer_setup(bytes([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00])) == (
        "not_handled",
        b"",
    )
    with pytest.raises(zm.InvalidValueError):
        f.answer_setup(bytes([1, 2, 3]))
    # The host side.
    assert zm.gtb_request(1, 1, 5) == B23
    assert zm.gtb_request(1, 1, 0x12) == B24
    assert zm.gtb_total_length(B21_B22[:5]) == 0x12
    with pytest.raises(zm.RejectedError):
        zm.gtb_total_length(B23)
    assert zm.gtb_blocks(B21_B22) == [(1, 0, 0, 1, 4, 0, 1, 0)]
    assert zm.gtb_blocks(B21_B22[:5]) == []


def test_default_function_and_capacities() -> None:
    d = zm.UsbFunction.default_function(0, 1)
    data = d.descriptors()
    assert data[:123] == APPENDIX_B[:123]
    assert (data[123], data[126]) == (0x02, 0x00)  # bulk IN, bInterval 0
    assert zm.gtb_blocks(d.gtb_set(1)) == [(1, 0, 0, 1, 0, 0x11, 0, 0)]
    assert zm.UsbFunction.default_function(0, 1, high_speed=True).endpoints()[3][4] == 512
    # 16 blocks, one for each Group, on one MIDI OUT and one MIDI IN endpoint that name all 16.
    ids = list(range(1, 17))
    s = zm.UsbSetting(
        [zm.UsbEndpoint("out", 1, blocks=ids), zm.UsbEndpoint("in", 1, blocks=ids)],
        blocks=[b22(i, i - 1) for i in ids],
    )
    a = zm.UsbFunction(0, 1, settings=[s])
    assert (a.group_mask(1, "out"), a.group_mask(1, "in")) == (0xFFFF, 0xFFFF)
    assert len(a.gtb_set(1)) == 5 + 13 * 16
    # Alternate Setting 0 only, with strings: the 92 bytes of USB-MIDI v1.0 Appendix B.
    only = zm.UsbFunction(0, 1, ac_string=5, alt0=zm.UsbAlt0(string=6))
    o = only.descriptors()
    assert (len(o), o[8], o[26]) == (92, 5, 6)
    assert only.answer_setup(B24)[0] == "stall"


def test_usb_descriptor_rules_and_bad_values() -> None:
    both = [out_ep(), zm.UsbEndpoint("in", 1)]
    # The same interface number twice; no endpoint; no block; no Group; no port. The text names
    # the rule.
    with pytest.raises(zm.RejectedError, match="same"):
        zm.UsbFunction(1, 1)
    for s in (
        zm.UsbSetting([]),
        zm.UsbSetting([out_ep()], blocks=[]),
        zm.UsbSetting(both, blocks=[b22(group_count=0)]),
    ):
        with pytest.raises(zm.RejectedError):
            zm.UsbFunction(0, 1, settings=[s])
    with pytest.raises(zm.RejectedError):
        zm.UsbFunction(0, 1, alt0=zm.UsbAlt0(out_cables=0, in_cables=0))
    # The capacities.
    s = zm.UsbSetting(both)
    with pytest.raises(zm.InvalidValueError):
        zm.UsbFunction(0, 1, settings=[s, s, s])
    with pytest.raises(zm.InvalidValueError):
        zm.UsbSetting([out_ep()] * 5)
    with pytest.raises(zm.InvalidValueError):
        out_ep(blocks=[])
    # Values out of range.
    with pytest.raises(OverflowError):
        zm.UsbFunction(256, 1)
    with pytest.raises(zm.InvalidValueError):
        zm.UsbEndpoint("out", 16)
    with pytest.raises(zm.InvalidValueError):
        zm.UsbEndpoint("x", 1)
    with pytest.raises(zm.InvalidValueError):
        b22(first_group=16)
    with pytest.raises(zm.InvalidValueError):
        zm.UsbBlock(1, 0, kind="other")
    with pytest.raises(zm.InvalidValueError):
        appendix_b().group_mask(1, "x")
