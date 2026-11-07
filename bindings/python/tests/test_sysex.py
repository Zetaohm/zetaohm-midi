"""Universal System Exclusive, MIDI Time Code and MMC (REQ-PY-SX-*): M1 v4.2.1, RP-024, CA-025, the
MIDI Tuning specification, RP-004 and RP-013."""

import pytest

import zetaohm_midi as zm


def test_sysex_base_m1_table_vii() -> None:
    # M1 Table VII: a 3-byte manufacturer ID; Table VIIa: a Universal header with Sub-ID#2.
    s = zm.parse_sysex(bytes([0xF0, 0x00, 0x20, 0x6B, 0x01, 0x02, 0xF7]))
    assert (s.kind, s.manufacturer, s.data, s.device_id) == (
        "manufacturer",
        b"\x00\x20\x6b",
        b"\x01\x02",
        None,
    )
    u = zm.parse_sysex(bytes([0x7E, 0x10, 0x06, 0x01]))
    assert (u.kind, u.device_id, u.sub_id1, u.sub_id2, u.data) == (
        "non_real_time",
        0x10,
        0x06,
        0x01,
        b"",
    )
    with pytest.raises(zm.MalformedError):
        zm.parse_sysex(bytes([0xF0, 0xF7]))
    assert zm.sysex_addressed(0x7F, 0x10) and not zm.sysex_addressed(0x11, 0x10)
    assert zm.manufacturer_sysex(b"\x00\x20\x6b", b"\x01\x02") == bytes(
        [0xF0, 0x00, 0x20, 0x6B, 0x01, 0x02, 0xF7]
    )
    with pytest.raises(zm.InvalidValueError):
        zm.manufacturer_sysex(b"\x7e", b"")
    assert zm.universal_sysex(False, 0x10, 0x06, 0x01, b"") == bytes(
        [0xF0, 0x7E, 0x10, 0x06, 0x01, 0xF7]
    )


def test_inquiry_gm_device_control() -> None:
    # M1 General Information: the responder answers a request to all devices with its own ID.
    ident = zm.Identity(b"\x00\x20\x6b", 0x0102, 3, bytes([0, 0, 0, 1]))
    reply = zm.respond_identity(bytes([0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]), 0x10, ident)
    assert reply is not None
    assert reply == bytes(
        [0xF0, 0x7E, 0x10, 0x06, 0x02, 0x00, 0x20, 0x6B, 0x02, 0x02, 0x03, 0x00, 0, 0, 0, 1, 0xF7]
    )
    m = zm.read_inquiry(reply)
    assert m is not None and m.is_reply and m.device_id == 0x10 and m.identity == ident
    assert zm.respond_identity(reply, 0x10, ident) is None
    assert zm.identity_request(0x7F) == bytes([0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7])
    assert zm.identity_reply(0x10, ident) == reply
    # RP-024 4.9.1: GM2 System On.
    on = bytes([0xF0, 0x7E, 0x7F, 0x09, 0x03, 0xF7])
    assert zm.read_gm_system(on) == (0x7F, 3) and zm.gm_system(0x7F, 3) == on
    assert zm.read_gm_system(reply) is None
    # M1 Device Control, CA-025.
    vol = bytes([0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x7F, 0x7F, 0xF7])
    d = zm.read_device_control(vol)
    assert d is not None and (d.kind, d.device_id, d.value) == ("master_volume", 0x7F, 0x3FFF)
    assert zm.DeviceControl("master_volume", 0x7F, 0x3FFF).to_bytes() == vol
    with pytest.raises(zm.InvalidValueError):
        zm.DeviceControl("master_coarse_tuning", 0x7F, 0x80)
    assert (zm.fine_tuning_offset(0x2000), zm.coarse_tuning_semitones(0x40)) == (0, 0)


def test_tuning_messages() -> None:
    # M1 MIDI Tuning: the frequency word 3CH 00H 00H is middle C.
    assert (
        zm.frequency_word(bytes([0x3C, 0, 0])) == (0x3C, 0)
        and zm.frequency_word_bytes(0x3C, 0) == b"\x3c\x00\x00"
    )
    words = [(k, k * 64) for k in range(128)]
    dump = zm.tuning_key_dump(0x7F, 2, b"zmidi tuning    ", words, bank=1)
    t = zm.read_tuning(dump)
    assert t is not None and (t.kind, t.bank, t.program, t.checksum_ok) == ("key_dump", 1, 2, True)
    assert (
        t.name == b"zmidi tuning    " and t.notes[100] == (100, 100, 6400) and len(t.notes) == 128
    )
    change = zm.tuning_note_change(True, 0x7F, 5, [(60, 0x3C, 0), (61, 0x3D, 0x2000)])
    c = zm.read_tuning(change)
    assert c is not None and (c.kind, c.real_time, c.notes) == (
        "note_change",
        True,
        [(60, 0x3C, 0), (61, 0x3D, 0x2000)],
    )
    so = zm.read_tuning(zm.tuning_scale_octave(False, 0x7F, 0xFFFF, [-64] + [0] * 10 + [63], False))
    assert so is not None and (so.kind, so.channels, so.offsets[0], so.two_byte) == (
        "scale_octave",
        0xFFFF,
        -64,
        False,
    )
    with pytest.raises(zm.InvalidValueError):
        zm.tuning_scale_octave(False, 0x7F, 0xFFFF, [-65] + [0] * 11, False)
    req = zm.read_tuning(bytes([0x7E, 0x7F, 0x08, 0x00, 0x05]))
    assert req is not None and req.kind == "dump_request"


def test_mtc_rp004_examples() -> None:
    # RP-004 p. 4: 01:37:52:16 at 30 frames per second.
    t = zm.MtcTime(1, 37, 52, 16, 3)
    pieces = bytes([0x00, 0x11, 0x24, 0x33, 0x45, 0x52, 0x61, 0x76])
    assert t.quarter_frames() == pieces
    # RP-013 p. 10: 00:22:00:00 drop-frame is skipped; 00:22:00:02 is 00:21:58:22 non-drop.
    with pytest.raises(zm.InvalidValueError):
        zm.MtcTime(0, 22, 0, 0, 2)
    df = zm.MtcTime(0, 22, 0, 2, 2)
    assert zm.MtcTime.from_frame_count(df.to_frame_count(), 3) == zm.MtcTime(0, 21, 58, 22, 3)
    # The generator gives the pieces; the reader gives the current time, +2 frames forward.
    g, r = zm.MtcGenerator(t), zm.MtcReader()
    data = [g.next_byte() for _ in range(8)]
    assert all(b is not None for b in data)
    events = [r.receive(b) for b in data if b is not None]
    assert events[7].raw == t and events[7].current == zm.MtcTime(1, 37, 52, 18, 3)
    assert r.is_locked and r.is_running and r.direction == "forward"
    g.set_running(False)
    assert g.next_byte() is None
    # p. 5: the Full Message stops the reader with its time.
    full = zm.mtc_full(0x7F, t)
    assert full == bytes([0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x61, 37, 52, 16, 0xF7])
    got = zm.read_mtc_full(full)
    assert got is not None
    dev, ft = got
    r.full_message(ft)
    assert (dev, r.is_running, r.time) == (0x7F, False, t)
    # p. 6: User Bits.
    ub = zm.mtc_user_bits(0x7F, [1, 2, 3, 4, 5, 6, 7, 8], 2)
    assert zm.read_mtc_user_bits(ub) == (0x7F, bytes([1, 2, 3, 4, 5, 6, 7, 8]), 2)


def test_mmc_transport_rp013_examples() -> None:
    # RP-013 Example 2A (p. 76): 01:02:03:06 at 30 frames, status form.
    tc = zm.TimeCode.from_bytes(bytes([0x61, 0x02, 0x03, 0x26, 0x00]))
    assert (tc.time, tc.status_form, tc.no_time_code) == (zm.MtcTime(1, 2, 3, 6, 3), True, False)
    assert tc.to_bytes() == bytes([0x61, 0x02, 0x03, 0x26, 0x00])
    # p. 32 MOVE: {st} to {ff} gives subframes 0; {ff} to {st} gives no time code.
    ff = tc.to_subframes_form()
    assert (ff.status_form, ff.subframes) == (False, 0)
    st = ff.to_status_form(True)
    assert (st.status_form, st.no_time_code, st.video_field1) == (True, True, True)
    assert (zm.mmc_short_name(0x06), zm.mmc_full_name(0x26), zm.mmc_full_name(0x20)) == (
        0x26,
        0x06,
        None,
    )
    # STOP and LOCATE [TARGET] 01:02:03:04.05 (p. 28).
    cmds = bytes(
        [0xF0, 0x7F, 0x7F, 0x06, 0x01, 0x44, 0x06, 0x01, 0x61, 0x02, 0x03, 0x04, 0x05, 0xF7]
    )
    m = zm.read_mmc(cmds)
    assert (
        m is not None
        and not m.response
        and [c.kind for c in m.commands] == ["stop", "locate_target"]
    )
    target = m.commands[1].target
    assert target is not None and target.subframes == 5
    assert zm.mmc_commands(0x7F, m.commands) == cmds
    with pytest.raises(zm.MalformedError):
        zm.read_mmc(bytes([0xF0, 0x7F, 0x7F, 0x06, 0x44, 0x06, 0x01, 0xF7]))
    with pytest.raises(zm.InvalidValueError):
        zm.MmcCommand("locate_field", register=0x07)
    # A response: SELECTED TIME CODE {st} from device 01H, not negative; 7FH is not a source.
    f = zm.MmcField("time_code", field=0x01, code=tc)
    resp = zm.mmc_responses(0x01, [f])
    assert resp == bytes([0xF0, 0x7F, 0x01, 0x07, 0x01, 0x61, 0x02, 0x03, 0x26, 0x00, 0xF7])
    r = zm.read_mmc(resp)
    assert (
        r is not None
        and r.response
        and r.fields[0].field == 0x01
        and r.fields[0].data == bytes([0x61, 2, 3, 0x26, 0])
    )
    with pytest.raises(zm.InvalidValueError):
        zm.mmc_responses(0x7F, [f])
    with pytest.raises(zm.InvalidValueError):
        zm.MmcField(
            "time_code",
            field=0x01,
            code=zm.TimeCode(zm.MtcTime(1, 0, 0, 0, 3), negative=True, status_form=True),
        )
    w = zm.MmcCommand("write", data=zm.mmc_fields([f]))
    assert [x.field for x in zm.read_mmc_fields(w.data)] == [0x01]
