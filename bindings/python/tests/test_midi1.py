"""MIDI 1.0 typed messages and receiver states (REQ-PY-MIDI1-*): M1 v4.2.1, RP-015, RP-018."""

import pytest

import zetaohm_midi as zm


def test_reader_gives_typed_messages_and_whole_sysex() -> None:
    # M1 p. 5 running status; p. 30 a Timing Clock inside a System Exclusive message.
    r = zm.Midi1Reader()
    ms = r.feed(bytes([0x90, 0x3C, 0x40, 0x3E, 0x40, 0xF0, 0x7E, 0x7F, 0xF8, 0x06, 0x01, 0xF7]))
    assert [m.kind for m in ms] == ["channel_voice", "channel_voice", "system", "sysex"]
    assert (ms[0].status, ms[0].data1, ms[0].data2) == (0x90, 0x3C, 0x40)
    assert ms[1] == zm.Midi1Message(0x90, 0x3E, 0x40)
    assert ms[2] == zm.Midi1Message(0xF8)
    assert ms[3].data == bytes([0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]) and ms[3].length == 6
    # A System Exclusive message longer than the buffer gives its length only.
    small = zm.Midi1Reader(sysex_capacity=4)
    (m,) = small.feed(bytes([0xF0, 1, 2, 3, 4, 0xF7]))
    assert (m.kind, m.length, m.data) == ("sysex_too_long", 6, b"")
    # AMB-M1-13: the reset of the program clears running status.
    r.push(0x90)
    r.reset()
    assert r.feed(bytes([0x3C, 0x40])) == []


def test_writer_running_status_and_sysex() -> None:
    w = zm.Midi1Writer(running_status=True)
    assert w.write(zm.Midi1Message(0x90, 0x3C, 0x40)) == bytes([0x90, 0x3C, 0x40])
    assert w.write(zm.Midi1Message(0x90, 0x3E, 0x40)) == bytes([0x3E, 0x40])
    w.refresh_status()
    assert w.write(zm.Midi1Message(0x90, 0x3E, 0x40)) == bytes([0x90, 0x3E, 0x40])
    sysex = bytes([0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7])
    assert w.write(zm.Midi1Message.sysex(sysex)) == sysex
    assert w.write(zm.Midi1Message(0xF2, 0x10, 0x02)) == bytes([0xF2, 0x10, 0x02])


def test_mode_state_modes_and_actions() -> None:
    # M1 pp. 20 to 26: Omni Off selects Mode 3; All Notes Off acts on the instrument.
    s = zm.ModeState(basic_channel=0)
    assert (s.mode, s.local) == (1, True)
    a = s.receive(zm.Midi1Message(0xB0, 0x7C, 0x00))
    assert (a.kind, a.changed, a.alternate) == ("mode_change", True, False)
    assert s.mode == 3
    a = s.receive(zm.Midi1Message(0xB0, 0x7B, 0x00))
    assert (a.kind, a.channel) == ("all_notes_off", None)
    assert s.receive(zm.Midi1Message(0x91, 0x3C, 0x40)).kind == "ignore"
    assert s.receive(zm.Midi1Message(0x90, 0x3C, 0x40)).kind == "voice"
    # Mode 4 with M = 4: a voice number for each channel from the Basic Channel.
    s.set_mono(4)
    assert (s.mode, s.m) == (4, 4)
    assert s.receive(zm.Midi1Message(0x92, 0x3C, 0x40)).voice == 2
    s.system_reset()
    assert (s.mode, s.local, s.m) == (1, True, 4)
    s.basic_channel = 5
    s.local = False
    assert (s.basic_channel, s.local) == (5, False)
    # A mode that is not implemented gives the alternate mode (p. 23).
    t = zm.ModeState(implemented=[1, 3])
    t.receive(zm.Midi1Message(0xB0, 0x7C, 0x00))
    a = t.receive(zm.Midi1Message(0xB0, 0x7E, 0x00))
    assert (t.requested_mode, t.mode, t.is_alternate, a.alternate) == (4, 1, True, True)


def test_controller_state_rp015_and_portamento() -> None:
    s = zm.ControllerState()
    e = s.receive(zm.Midi1Message(0xB0, 0x01, 0x12))
    assert e is not None and (e.kind, e.controller, e.value) == ("controller14", 1, 0x12 << 7)
    e = s.receive(zm.Midi1Message(0xB0, 0x21, 0x34))
    assert e is not None and e.value == 0x12 << 7 | 0x34
    # p. 17, Example 2: B0 54 3C, 90 40 40 glides from note 60, a later Note On has no source.
    assert s.receive(zm.Midi1Message(0xB0, 0x54, 0x3C)) is None
    e = s.receive(zm.Midi1Message(0x90, 0x40, 0x40))
    assert e is not None and (e.kind, e.source) == ("portamento", 0x3C)
    assert s.receive(zm.Midi1Message(0x90, 0x40, 0x40)) is None
    # pp. 13, 14: the bank and program writer and the bank state.
    ms = zm.bank_program_messages(0, 0x0102, 0x11)
    assert [(m.status, m.data1, m.data2) for m in ms] == [(0xB0, 0, 2), (0xB0, 0x20, 2), (0xC0, 0x11, 0)]
    events = [s.receive(m) for m in ms]
    assert events[:2] == [None, None]
    assert events[2] is not None and (events[2].program, events[2].bank) == (0x11, 0x0102)
    # RP-015: Reset All Controllers keeps the bank; Expression 127, Balance kept (AMB-RP-015-04).
    e = s.receive(zm.Midi1Message(0xB0, 0x79, 0x00))
    assert e is not None and e.kind == "reset_all_controllers"
    assert s.bank == 0x0102
    assert (zm.reset_value(11), zm.reset_value(64), zm.reset_value(8), zm.reset_value(7)) == (127, 0, None, None)
    ms = zm.controller14_messages(5, 1, 0x1234)
    assert [(m.status, m.data1, m.data2) for m in ms] == [(0xB5, 1, 0x24), (0xB5, 0x21, 0x34)]


def test_parameter_state_rp018_example() -> None:
    # RP-018 example: B0 65 00, B0 64 00, B0 60 00, B0 60 00, B0 65 7F, B0 64 7F.
    s = zm.ParameterState()
    assert (s.rpn_reception, s.nrpn_reception) == (True, False)
    pbs = 2 << 7
    for c, v in [(0x65, 0), (0x64, 0), (0x60, 0), (0x60, 0), (0x65, 0x7F), (0x64, 0x7F)]:
        e = s.receive(zm.Midi1Message(0xB0, c, v))
        if e is not None:
            assert (e.change, e.parameter) == ("increment", zm.Parameter(0, 0))
            pbs = e.apply(pbs)
    assert pbs == 2 << 7 | 2
    assert s.selection is None
    # p. 50: Bn 64 03 65 00 06 tt selects Tuning Program tt.
    for c, v in [(0x64, 3), (0x65, 0)]:
        assert s.receive(zm.Midi1Message(0xB0, c, v)) is None
    e = s.receive(zm.Midi1Message(0xB0, 0x06, 5))
    assert e is not None and (e.change, e.value, e.parameter) == ("data_entry_msb", 5, zm.Parameter(0, 3))
    # RP-015: Reset All Controllers selects no parameter.
    assert s.receive(zm.Midi1Message(0xB0, 0x79, 0)) is None
    assert s.selection is None
    s.nrpn_reception = True
    assert s.nrpn_reception
    ms = zm.value_messages(3, zm.Parameter(0, 0), 2 << 7 | 50)
    assert [(m.data1, m.data2) for m in ms] == [(0x65, 0), (0x64, 0), (0x06, 2), (0x26, 50)]
    null = zm.select_messages(0, zm.Parameter(0x7F, 0x7F))
    assert [(m.data1, m.data2) for m in null] == [(0x65, 0x7F), (0x64, 0x7F)]
    assert zm.change_message(0, "decrement") == zm.Midi1Message(0xB0, 0x61, 0)


def test_bad_values() -> None:
    with pytest.raises(zm.InvalidValueError):
        zm.Midi1Message(0x90, 0x80)
    with pytest.raises(zm.InvalidValueError):
        zm.Midi1Message(0xF0)
    with pytest.raises(zm.InvalidValueError):
        zm.Midi1Message.sysex(bytes([0xF0, 0x80, 0xF7]))
    with pytest.raises(zm.InvalidValueError):
        zm.ModeState(basic_channel=16)
    with pytest.raises(zm.InvalidValueError):
        zm.ModeState(implemented=[5])
    with pytest.raises(zm.InvalidValueError):
        zm.ControllerState().receive(zm.Midi1Message(0xF8))
    with pytest.raises(zm.InvalidValueError):
        zm.controller14_messages(0, 32, 0)
    with pytest.raises(zm.InvalidValueError):
        zm.Parameter(0x80, 0)
    with pytest.raises(zm.InvalidValueError):
        zm.change_message(0, "jump")
    with pytest.raises(zm.InvalidValueError):
        zm.Midi1Writer().write(zm.Midi1Reader(sysex_capacity=0).feed(bytes([0xF0, 0xF7]))[0])


def test_sync_receiver_m1_examples() -> None:
    # M1 p. 28: SPP 4 (24 clocks), Continue and three Timing Clocks give the 27th clock.
    s = zm.SyncReceiver()
    e = s.receive(zm.Midi1Message(0xF2, 4, 0))
    assert e is not None and (e.kind, e.position) == ("locate", 24)
    e = s.receive(zm.Midi1Message(0xFB))
    assert e is not None and e.kind == "continue"
    assert s.run == "waiting"
    clocks = [s.receive(zm.Midi1Message(0xF8)) for _ in range(3)]
    assert [(c.kind, c.position) for c in clocks if c is not None] == [("clock", 24), ("clock", 25), ("clock", 26)]
    assert (s.position, s.run) == (27, "playing")
    # p. 32: Start while playing is ignored; the front panel has the same transitions.
    assert s.command("start") is None
    e = s.command("stop")
    assert e is not None and e.kind == "stop"
    assert s.receive(zm.Midi1Message(0xF8)) is None
    # p. 29: Song Select out of MIDI sync mode is ignored; the System Reset action stops at 0.
    s.sync_mode = False
    assert s.receive(zm.Midi1Message(0xF3, 5)) is None
    s.system_reset()
    assert (s.position, s.run, s.song, s.sync_mode) == (0, "stopped", 0, False)
    with pytest.raises(zm.InvalidValueError):
        s.command("pause")
    with pytest.raises(zm.InvalidValueError):
        s.receive(zm.Midi1Message(0x90, 60, 64))


def test_active_sensing_monitor_and_sender() -> None:
    # M1 p. 32: no timeout before FEH; over 330 ms with no byte gives one timeout; send FEH after
    # 270 ms with no byte.
    m = zm.ActiveSensingMonitor()
    assert m.timeout_us == 330_000 and not m.poll(10_000_000)
    m.receive_byte(0xFE, 0)
    m.receive_byte(0x90, 300_000)
    assert m.is_armed and not m.poll(630_000)
    assert m.poll(630_001) and not m.is_armed
    assert not m.poll(2_000_000)
    m.receive(True, 3_000_000)
    assert m.is_armed
    s = zm.ActiveSensingSender()
    assert s.interval_us == 270_000
    assert not s.poll(269_999) and s.poll(270_000)
    s.sent(400_000)
    assert not s.poll(669_999) and s.poll(670_000)
    # The defaults are the zmidi-io constants ACTIVE_SENSING_TIMEOUT_US and ACTIVE_SENSING_INTERVAL_US.
    assert (zm.ActiveSensingMonitor().timeout_us, zm.ActiveSensingSender().interval_us) == (330_000, 270_000)


def _mpe_feed(params: list[zm.ParameterState], rx: zm.MpeReceiver, *msgs: tuple[int, ...]) -> list[zm.MpeEvent]:
    out = []
    for b in msgs:
        m = zm.Midi1Message(*b)
        out.append(rx.receive(m, params[b[0] & 0x0F].receive(m)))
    return out


def test_mpe_receiver_m1_100_examples() -> None:
    # M1-100 Appendix B.5 with the corrected data byte 0x0B (AMB-M1-100-04): Upper 5 to 15,
    # Lower keeps 2 to 4; the reset mask is the Upper Zone after the MCM.
    params = [zm.ParameterState() for _ in range(16)]
    rx = zm.MpeReceiver()
    *_, e = _mpe_feed(params, rx, (0xB0, 0x65, 0), (0xB0, 0x64, 6), (0xB0, 0x06, 7))
    assert (e.kind, e.reset) == ("layout", 0x00FF)
    *_, e = _mpe_feed(params, rx, (0xBF, 0x65, 0), (0xBF, 0x64, 6), (0xBF, 0x06, 0x0B))
    assert (e.kind, e.reset) == ("layout", 0xFFF0)
    assert rx.layout == zm.MpeLayout(3, 11)
    assert rx.layout.role(4) == ("member", "upper") and rx.layout.role(0) == ("manager", "lower")
    assert (rx.sensitivity("upper", True), rx.sensitivity("upper", False), rx.mode("upper")) == ((2, 0), (48, 0), 3)
    # 2.2.5: RPN 0 on one Member Channel sets the Member sensitivity of the Zone.
    *_, e = _mpe_feed(params, rx, (0xB5, 0x65, 0), (0xB5, 0x64, 0), (0xB5, 0x06, 24))
    assert (e.kind, e.zone, e.manager) == ("sensitivity", "upper", False)
    # 0x41 << 7 | 0x2B at 24 semitones is a quarter tone (AMB-M1-100-08).
    (e,) = _mpe_feed(params, rx, (0xE6, 0x2B, 0x41))
    assert (e.kind, e.zone, e.manager) == ("zone", "upper", False)
    offset = rx.pitch_offset(6)
    assert offset is not None and abs(offset - 5_000) <= 50
    assert (rx.pitch_bend(6), rx.pressure(6), rx.brightness(6)) == (8363, 0, 64)
    # Appendix E: Polyphonic Key Pressure on a Member Channel is ignored; All Sound Off on the
    # Manager Channel is given (AMB-M1-100-12).
    assert _mpe_feed(params, rx, (0xA6, 60, 64))[0].kind == "ignored"
    assert _mpe_feed(params, rx, (0xBF, 0x78, 0))[0].kind == "zone"
    assert zm.mpe_class("all_sound_off", manager=True, transmit=False) == "prohibited"
    assert zm.mpe_class("program_change", manager=False, transmit=True, mono=True) == "optional"
    # B.6: the Upper Zone with 14 Member Channels leaves the Lower Zone none.
    *_, e = _mpe_feed(params, rx, (0xBF, 0x65, 0), (0xBF, 0x64, 6), (0xBF, 0x06, 0x0E))
    assert rx.layout == zm.MpeLayout(0, 14) and rx.layout.role(0) == ("no_zone", None)
    assert rx.set_layout(zm.MpeLayout.DEFAULT) == 0xFFFF and rx.layout.lowest_member("lower") == 1


def test_mpe_sender_helpers() -> None:
    # C.4 and C.6: +7 semitones at 48 is 9387; 16383 at 2 is 2 semitones.
    assert zm.PITCH_UNITS_PER_SEMITONE == 10_000
    assert zm.pitch_bend_value(70_000, 48) == 9387
    assert zm.pitch_bend_value(20_000, 2) == 16383
    assert zm.pitch_offset(16383, 2) == 20_000
    assert abs(zm.pitch_offset(9387, 48) - 70_000) <= 100
    # Table 4 with the corrected Pitch Bend (AMB-M1-100-08): a quarter tone up at 48 semitones.
    ms = zm.note_messages(2, 0x3C, 0x38, zm.pitch_bend_value(5_000, 48), 0x40, 0)
    assert [(m.status, m.data1, m.data2) for m in ms] == [
        (0xE2, 0x55, 0x40), (0xB2, 0x4A, 0x40), (0xD2, 0x00, 0), (0x92, 0x3C, 0x38),
    ]
    # Appendix B.1 and the Mode message on the lowest Member Channel with M = 7.
    assert [(m.status, m.data1, m.data2) for m in zm.mcm_messages("lower", 15)] == [
        (0xB0, 0x65, 0), (0xB0, 0x64, 6), (0xB0, 0x06, 0x0F),
    ]
    two = zm.MpeLayout(7, 7)
    m = zm.mode_message(two, "upper", True)
    assert (m.status, m.data1, m.data2) == (0xB8, 0x7E, 7)
    zs = zm.zone_sensitivity_messages(two, "upper", 2, 48)
    assert len(zs) == 24 and zs[2] == zm.Midi1Message(0xBF, 0x06, 2) and zs[5] == zm.Midi1Message(0xB8, 0x06, 48)
    assert zm.sensitivity_messages(3, 12)[2] == zm.Midi1Message(0xB3, 0x06, 12)
    assert two.with_mcm("upper", 14) == zm.MpeLayout(0, 14)
    # A.3: an unoccupied channel first, then the oldest last Note Off; with the same-note option,
    # the free channel that last played the note.
    small = zm.MpeLayout(3, 0)
    a = zm.MpeNoteAllocator()
    assert [a.note_on(small, "lower", n) for n in (60, 62, 64)] == [1, 2, 3]
    a.note_off(2)
    a.note_off(1)
    assert a.note_on(small, "lower", 70) == 2 and a.active_notes(2) == 1
    assert a.note_on(small, "upper", 70) is None
    b = zm.MpeNoteAllocator(same_note=True)
    b.note_on(small, "lower", 60)
    b.note_on(small, "lower", 62)
    b.note_off(1)
    b.note_off(2)
    assert b.note_on(small, "lower", 60) == 1
    b.reset()
    assert b.same_note and b.active_notes(1) == 0
    for bad in (
        lambda: zm.MpeLayout(7, 8),
        lambda: zm.mcm_messages("lower", 16),
        lambda: zm.mcm_messages("middle", 1),
        lambda: zm.note_messages(2, 60, 0),
        lambda: zm.mode_message(small, "upper", True),
        lambda: zm.mpe_class("jump", True, True),
        lambda: zm.pitch_offset(16384, 48),
    ):
        with pytest.raises(zm.InvalidValueError):
            bad()


def test_gm_receiver_rp024() -> None:
    # RP-024 2.4: channel 10 is a Rhythm Channel at the start; a bank MSB 78H program selection
    # makes channel 11 one too. 3.5.6: Mono On with M = 1 on a Rhythm Channel gives All Notes Off.
    gm = zm.GmReceiver()
    assert (gm.level, gm.rhythm_channels, gm.kind(9), gm.kind(0)) == (
        "gm2",
        0x0200,
        "rhythm",
        "melody",
    )
    gm.program(10, 0x78 << 7)
    assert gm.kind(10) == "rhythm" and gm.rhythm_channels == 0x0600
    a = gm.receive(zm.Midi1Message(0xB0, 0x7E, 0x01))
    assert a.kind == "mode_change" and gm.is_mono(0)
    assert gm.receive(zm.Midi1Message(0xB9, 0x7E, 0x01)).kind == "all_notes_off" and not gm.is_mono(
        9
    )
    # 4.9.2, 4.9.3: GM1 System On resets the channels; GM System Off gives the level off.
    gm.reset("gm1")
    assert (gm.level, gm.rhythm_channels, gm.is_mono(0)) == ("gm1", 0x0200, False)
    gm.system_off()
    assert gm.level == "off"
    with pytest.raises(zm.InvalidValueError):
        gm.reset("gm3")
    # 3.3: the GM reset of a controller state holds the Rhythm bank on channel 10.
    s = zm.ControllerState()
    zm.gm_reset_controller_state(s, 9)
    assert s.bank == 0x78 << 7
    assert zm.gm2_controller_default(7) == 100 and zm.gm2_controller_default(120) is None
    assert zm.rhythm_ignores_controller(1) and not zm.rhythm_ignores_controller(7)
    assert zm.rhythm_ignores_parameter(zm.Parameter(0, 0))
    assert zm.rhythm_ignores_message(zm.Midi1Message(0xE9, 0x00, 0x40))
    # 3.4.4: the GM2 default 00H 40H is 50 cents.
    assert zm.modulation_depth_range(0x40) == 5000
