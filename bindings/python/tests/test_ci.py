"""MIDI-CI and one-chunk Property Exchange (REQ-PY-CI-*): two devices, M2-101 and M2-103."""

import pytest

import zetaohm_midi as zm

LIST = b'[{"resource":"DeviceInfo"},{"resource":"X-Volume","canSet":"full"}]'


def device(seed: int) -> zm.CiDevice:
    return zm.CiDevice(
        b"\x00\x21\x7f",
        family=1,
        model=2,
        revision=b"\x00\x00\x01\x00",
        product_instance_id="zmidi-py",
        resource_list=LIST,
        seed=seed,
    )


def deliver(dev: zm.CiDevice, messages: list[bytes]) -> tuple[list[bytes], "zm.CiEvent | None"]:
    replies: list[bytes] = []
    event = None
    for m in messages:
        r, event = dev.receive(0, m, 1000)
        replies += [msg for _, msg in r]
    return replies, event


def test_discovery_pe_get_and_set() -> None:
    a, b = device(1), device(2)
    assert a.muid != b.muid
    with pytest.raises(zm.RejectedError):
        a.pe_get(b.muid, b"{}")
    replies, _ = deliver(b, [a.discovery()])
    _, ev = deliver(a, replies)
    assert ev is not None and ev.kind == "discovered" and ev.muid == b.muid
    replies, _ = deliver(b, [a.endpoint_info_inquiry(b.muid)])
    assert deliver(a, replies)[1].kind == "endpoint_info"  # type: ignore[union-attr]
    replies, _ = deliver(b, [a.pe_capabilities_inquiry(b.muid)])
    assert deliver(a, replies)[1].kind == "pe_capabilities"  # type: ignore[union-attr]

    rid, chunks = a.pe_get(b.muid, b'{"resource":"DeviceInfo"}')
    _, ev = deliver(b, chunks)
    assert (
        ev is not None
        and ev.kind == "pe_get"
        and ev.resource == "DeviceInfo"
        and ev.request_id == rid
    )
    info = b'{"manufacturerId":[0,33,127]}'
    _, ev = deliver(a, b.pe_reply(200, info))
    assert (
        ev is not None
        and ev.kind == "pe_get_reply"
        and ev.header == b'{"status":200}'
        and ev.data == info
    )
    with pytest.raises(zm.RejectedError):
        b.pe_reply(200)

    _, chunks = a.pe_get(b.muid, b'{"resource":"ResourceList"}')
    deliver(b, chunks)
    assert deliver(a, b.pe_reply(404))[1].data == LIST  # type: ignore[union-attr]

    _, chunks = a.pe_get(b.muid, b'{"resource":"X-None"}')
    deliver(b, chunks)
    assert deliver(a, b.pe_reply(404))[1].header == b'{"status":404}'  # type: ignore[union-attr]

    _, chunks = a.pe_set(b.muid, b'{"resource":"X-Volume"}', b'{"value":100}')
    _, ev = deliver(b, chunks)
    assert ev is not None and ev.kind == "pe_set" and ev.data == b'{"value":100}'
    assert deliver(a, b.pe_reply(200))[1].kind == "pe_set_reply"  # type: ignore[union-attr]


def test_bad_configuration() -> None:
    with pytest.raises(zm.InvalidValueError):
        zm.CiDevice(b"\x00\x21")
    with pytest.raises(zm.InvalidValueError):
        zm.CiDevice(b"\x7d", max_sysex_size=100)


# --- Profile Configuration (M2-101 7) and Process Inquiry (M2-101 9) ---

BROADCAST = 0x0FFFFFFF
# A Standard Defined Profile (M2-101 Table 19): Bank 0, Number 1, Version 1, Level 0x01 Minimum.
PROF = bytes([0x7E, 0x00, 0x01, 0x01, 0x01])


def muid_bytes(m: int) -> bytes:
    """M2-101 3.2: a MUID as 4 bytes of 7 bits, LSB first."""
    return bytes((m >> s) & 0x7F for s in (0, 7, 14, 21))


def ci_header(device_id: int, sub_id2: int, src: int, dst: int) -> bytes:
    """M2-101 Table 3: the 13-byte header, Message Version 0x02 (no F0 and F7)."""
    return bytes([0x7E, device_id, 0x0D, sub_id2, 0x02]) + muid_bytes(src) + muid_bytes(dst)


def full_device(seed: int) -> zm.CiDevice:
    return zm.CiDevice(
        b"\x00\x21\x7f",
        family=1,
        model=2,
        revision=b"\x00\x00\x01\x00",
        product_instance_id="zmidi-py",
        resource_list=LIST,
        profiles=True,
        process_inquiry=True,
        seed=seed,
    )


def discovered_pair() -> tuple[zm.CiDevice, zm.CiDevice]:
    a, b = full_device(11), full_device(12)
    replies, _ = deliver(b, [a.discovery()])
    _, ev = deliver(a, replies)
    assert ev is not None and ev.kind == "discovered"
    assert ev.device_id is None and ev.profile is None and ev.bitmaps is None
    return a, b


def test_profiles_m2_101_tables() -> None:
    a, b = discovered_pair()
    ma, mb = a.muid, b.muid

    # Profile Added Report to Broadcast (Table 20) for a disabled Profile on Channel 1.
    reports = b.add_profile(0x00, 0, PROF, max_channels=1)
    assert reports == [(0, ci_header(0x00, 0x26, mb, BROADCAST) + PROF)]
    with pytest.raises(zm.RejectedError):
        b.add_profile(0x00, 0, PROF, max_channels=1)

    # Set Profile On before a Profile Inquiry: rejected (M2-101 2.2).
    with pytest.raises(zm.RejectedError):
        a.set_profile_on(mb, 0x00, PROF, 1)

    # Profile Inquiry (Table 17) and the automatic Reply to Profile Inquiry (Table 18).
    msg = a.profile_inquiry(mb, 0x00)
    assert msg == ci_header(0x00, 0x20, ma, mb)
    replies, _ = deliver(b, [msg])
    assert replies == [ci_header(0x00, 0x21, mb, ma) + bytes([0, 0, 1, 0]) + PROF]
    _, ev = deliver(a, replies)
    assert ev is not None and ev.kind == "profile_inquiry_reply" and ev.muid == mb
    assert ev.device_id == 0x00 and ev.group == 0
    assert ev.enabled_profiles == [] and ev.disabled_profiles == [PROF]

    # Set Profile On (Table 24), Level 0x7F, 1 Channel; B enables the Profile and sends the
    # Profile Enabled Report (Table 26).
    msg = a.set_profile_on(mb, 0x00, PROF, 1)
    assert msg == ci_header(0x00, 0x22, ma, mb) + PROF[:4] + bytes([0x7F, 0x01, 0x00])
    replies, ev = deliver(b, [msg])
    assert replies == [] and ev is not None and ev.kind == "set_profile" and ev.muid == ma
    assert ev.on is True and ev.channels == 1 and ev.device_id == 0x00 and ev.group == 0
    assert ev.profile is not None and ev.profile[:4] == PROF[:4]
    group, msg = b.set_profile_state(ev.device_id, ev.group, ev.profile, True, ev.channels)
    assert (group, msg) == (0, ci_header(0x00, 0x24, mb, BROADCAST) + PROF + bytes([0x01, 0x00]))
    _, ev = deliver(a, [msg])
    assert ev is not None and ev.kind == "profile_report" and ev.report_kind == "enabled"
    assert ev.channels == 1 and ev.profile == PROF

    # Profile Details Inquiry (Table 22), Inquiry Target 0x00: B replies by itself with the
    # Number of MIDI Channels (M2-102 2.5.1): 1 in use, maximum 1.
    msg = a.profile_details_inquiry(mb, 0x00, PROF, 0x00)
    assert msg == ci_header(0x00, 0x28, ma, mb) + PROF + bytes([0x00])
    replies, _ = deliver(b, [msg])
    _, ev = deliver(a, replies)
    assert ev is not None and ev.kind == "profile_details_reply"
    assert ev.target == 0 and ev.notification is False and ev.data == bytes([1, 0, 1, 0])

    # Inquiry Target 0x40: the program of B answers (Table 23).
    replies, ev = deliver(b, [a.profile_details_inquiry(mb, 0x00, PROF, 0x40)])
    assert replies == [] and ev is not None and ev.kind == "profile_details_inquiry"
    assert ev.target == 0x40 and ev.device_id == 0x00 and ev.profile == PROF
    assert ev.profile is not None
    msg = b.profile_details_reply(ma, ev.device_id, ev.profile, ev.target, b"\x01\x02")
    assert msg == ci_header(0x00, 0x29, mb, ma) + PROF + bytes([0x40, 0x02, 0x00, 0x01, 0x02])
    _, ev = deliver(a, [msg])
    assert ev is not None and ev.target == 0x40 and ev.data == b"\x01\x02"

    # Profile Specific Data (Table 28): a 4-byte length, LSB first.
    msg = a.profile_specific_data(mb, 0x00, PROF, b"\x10\x20\x30")
    assert msg == ci_header(0x00, 0x2F, ma, mb) + PROF + bytes([3, 0, 0, 0, 0x10, 0x20, 0x30])
    _, ev = deliver(b, [msg])
    assert ev is not None and ev.kind == "profile_specific_data" and ev.data == b"\x10\x20\x30"

    # Set Profile Off (Table 25), then the Profile Disabled Report (Table 27).
    msg = a.set_profile_off(mb, 0x00, PROF)
    assert msg == ci_header(0x00, 0x23, ma, mb) + PROF + bytes([0, 0])
    _, ev = deliver(b, [msg])
    assert ev is not None and ev.kind == "set_profile" and ev.on is False
    _, msg = b.set_profile_state(0x00, 0, PROF, False, 1)
    assert msg == ci_header(0x00, 0x25, mb, BROADCAST) + PROF + bytes([0x01, 0x00])
    _, ev = deliver(a, [msg])
    assert ev is not None and ev.report_kind == "disabled"

    # Removing the disabled Profile gives the Profile Removed Report only (Table 21).
    assert b.remove_profile(0x00, 0, PROF) == [(0, ci_header(0x00, 0x27, mb, BROADCAST) + PROF)]
    with pytest.raises(zm.RejectedError):
        b.remove_profile(0x00, 0, PROF)
    with pytest.raises(zm.RejectedError):
        b.set_profile_state(0x00, 0, PROF, True, 1)

    # An enabled Profile: Added then Enabled (M2-101 7.4); removed: Disabled then Removed (7.5).
    reports = b.add_profile(0x7E, 0, PROF, enabled=True)
    assert [m[3] for _, m in reports] == [0x26, 0x24]
    assert [m[3] for _, m in b.remove_profile(0x7E, 0, PROF)] == [0x25, 0x27]


def test_process_inquiry_m2_101_tables() -> None:
    a, b = discovered_pair()
    ma, mb = a.muid, b.muid

    # Process Inquiry Capabilities (Table 40) and the automatic reply (Table 41: D0 set).
    msg = a.pi_capabilities_inquiry(mb)
    assert msg == ci_header(0x7F, 0x40, ma, mb)
    replies, _ = deliver(b, [msg])
    assert replies == [ci_header(0x7F, 0x41, mb, ma) + bytes([0x01])]
    _, ev = deliver(a, replies)
    assert ev is not None and ev.kind == "pi_capabilities" and ev.midi_message_report is True
    assert ev.profile is None

    # Inquiry: MIDI Message Report (Table 43): full report of MTC Quarter Frame, Control Change
    # and Notes.
    assert not a.report_open(mb)
    msg = a.message_report_inquiry(mb, 0x7F, 0x7F, (0x01, 0x02, 0x01))
    assert msg == ci_header(0x7F, 0x42, ma, mb) + bytes([0x7F, 0x01, 0x00, 0x02, 0x01])
    assert a.report_open(mb)
    replies, ev = deliver(b, [msg])
    assert replies == [] and ev is not None and ev.kind == "message_report"
    assert ev.device_id == 0x7F and ev.group == 0
    assert ev.data_control == 0x7F and ev.bitmaps == (0x01, 0x02, 0x01)
    assert ev.device_id is not None and ev.bitmaps is not None

    # Reply to MIDI Message Report (Table 45): only the bits both requested and supported.
    msg = b.message_report_reply(ma, ev.device_id, ev.bitmaps, (0x00, 0x03, 0x00))
    assert msg == ci_header(0x7F, 0x43, mb, ma) + bytes([0x00, 0x00, 0x02, 0x00])
    _, ev = deliver(a, [msg])
    assert ev is not None and ev.kind == "message_report_reply" and ev.bitmaps == (0, 0x02, 0)

    # End of MIDI Message Report (Table 46) closes the report.
    msg = b.message_report_end(ma, 0x7F)
    assert msg == ci_header(0x7F, 0x44, mb, ma)
    _, ev = deliver(a, [msg])
    assert ev is not None and ev.kind == "message_report_end" and not a.report_open(mb)

    # Abort: a NAK (Table 15) for Sub-ID#2 0x42 with Status Code 0x20, or 0x40 for an error
    # (M2-101 9.8.1).
    msg = b.abort_report(ma, 0x7F)
    assert msg[:13] == ci_header(0x7F, 0x7F, mb, ma) and msg[13] == 0x42 and msg[14] == 0x20
    assert b.abort_report(ma, 0x7F, error=True)[14] == 0x40


def test_profile_and_process_bad_values() -> None:
    a, b = discovered_pair()
    mb = b.muid
    with pytest.raises(zm.InvalidValueError):
        b.add_profile(0x10, 0, PROF)  # a reserved Device ID (M2-101 Table 4)
    with pytest.raises(zm.InvalidValueError):
        b.add_profile(0x00, 16, PROF)  # a Group over 15
    with pytest.raises(zm.InvalidValueError):
        b.add_profile(0x00, 0, PROF[:4])  # a Profile ID has 5 bytes
    with pytest.raises(zm.InvalidValueError):
        b.add_profile(0x00, 0, b"\x80\x00\x01\x01\x01")  # a byte over 0x7F
    with pytest.raises(zm.InvalidValueError):
        b.add_profile(0x00, 0, PROF, channels=0x4000)  # 14 bits
    with pytest.raises(zm.InvalidValueError):
        a.profile_inquiry(mb, 0x10)
    with pytest.raises(zm.InvalidValueError):
        a.profile_inquiry(1 << 28, 0x00)  # a MUID has 28 bits
    with pytest.raises(zm.InvalidValueError):
        a.profile_details_inquiry(mb, 0x00, PROF, 0x80)
    with pytest.raises(zm.InvalidValueError):
        a.message_report_inquiry(mb, 0x7F, 0x80, (0, 0, 0))
    with pytest.raises(zm.InvalidValueError):
        b.message_report_reply(a.muid, 0x7F, (0x80, 0, 0), (0, 0, 0))
    # The table has 8 Profiles: the ninth is rejected.
    for n in range(8):
        b.add_profile(0x00, 0, bytes([0x7E, 0x00, n + 2, 0x01, 0x01]))
    with pytest.raises(zm.RejectedError):
        b.add_profile(0x00, 0, PROF)


def u14(v: int) -> bytes:
    return bytes([v & 0x7F, v >> 7])


def get_reply_chunk(src: int, dst: int, rid: int, header: bytes, total: int, number: int, data: bytes) -> bytes:
    """M2-101 Table 33: a Reply to Get Property Data Chunk."""
    return (
        ci_header(0x7F, 0x35, src, dst)
        + bytes([rid])
        + u14(len(header))
        + header
        + u14(total)
        + u14(number)
        + u14(len(data))
        + data
    )


def test_request_ids_are_freed() -> None:
    """REQ-PY-CI-007: the last Chunk of a reply frees the Request ID at the next receive; pe_end
    frees the ID of a request with no reply. The device has 4 Request IDs."""
    a, b = device(1), device(2)
    replies, _ = deliver(b, [a.discovery()])
    deliver(a, replies)
    replies, _ = deliver(b, [a.pe_capabilities_inquiry(b.muid)])
    deliver(a, replies)
    get = b'{"resource":"DeviceInfo"}'
    rid = -1
    for _ in range(6):
        rid, chunks = a.pe_get(b.muid, get)
        deliver(b, chunks)
        _, ev = deliver(a, b.pe_reply(200, b"{}"))
        assert ev is not None and ev.kind == "pe_get_reply" and ev.request_id == rid
    # Until the next receive, a new request gets another ID.
    second, chunks = a.pe_get(b.muid, get)
    assert second != rid
    deliver(b, chunks)
    deliver(a, b.pe_reply(200, b"{}"))

    a.pe_end(b.muid, second)
    ids = [a.pe_get(b.muid, b"{}")[0] for _ in range(4)]
    assert ids[0] == second
    replies, _ = deliver(b, [a.endpoint_info_inquiry(b.muid)])
    deliver(a, replies)
    with pytest.raises(zm.RejectedError):
        a.pe_get(b.muid, b"{}")
    a.pe_end(b.muid, ids[2])
    assert a.pe_get(b.muid, b"{}")[0] == ids[2]
    with pytest.raises(zm.InvalidValueError):
        a.pe_end(b.muid, 0x80)
    with pytest.raises(zm.InvalidValueError):
        a.pe_end(0x10000000, 0)
    for i in ids:
        a.pe_end(b.muid, i)

    # A reply of 2 Chunks (M2-101 Table 29): only the last Chunk frees the ID.
    first, _ = a.pe_get(b.muid, b"{}")
    _, ev = deliver(a, [get_reply_chunk(b.muid, a.muid, first, b'{"status":200}', 2, 1, b'{"a":')])
    assert ev is not None and ev.chunk_number == 1 and ev.chunk_total == 2
    _, ev = deliver(a, [get_reply_chunk(b.muid, a.muid, first, b"", 2, 2, b"1}")])
    assert ev is not None and ev.chunk_number == 2 and ev.request_id == first
    assert a.pe_get(b.muid, b"{}")[0] != first
