"""The Property Exchange engine (REQ-PY-PE-*): two devices, A (Initiator) and B (Responder), each with a
CiDevice and a PeEngine. M2-103: Get with many Chunks (5.3), ResourceList (9), 404 (8.6.2), Set with
many Chunks, Subscription start, full and end (11), timeouts (5.3, 12), MIDI-CI ACK 0x10 and NAK 0x20."""

import pytest

import zetaohm_midi as zm

NOW = 1_000_000


def device(seed: int) -> zm.CiDevice:
    return zm.CiDevice(b"\x00\x21\x7f", family=1, model=2, revision=b"\x00\x00\x01\x00", seed=seed)


def deliver(
    dev: zm.CiDevice, eng: zm.PeEngine, messages: list[bytes]
) -> tuple[list[bytes], list[zm.PeEvent]]:
    """Gives each message to `dev` and then to `eng`; gives the automatic replies and the engine events."""
    replies: list[bytes] = []
    events: list[zm.PeEvent] = []
    for m in messages:
        r, _ = dev.receive(0, m, NOW)
        replies += [msg for _, msg in r]
        ev = eng.receive(dev, NOW)
        if ev is not None:
            events.append(ev)
    return replies, events


def pending(dev: zm.CiDevice, eng: zm.PeEngine) -> list[bytes]:
    out: list[bytes] = []
    while (m := eng.write_pending(dev, NOW)) is not None:
        out.append(m)
    return out


def send_all(
    dev: zm.CiDevice, eng: zm.PeEngine, peer: int, rid: int, role: str, data: bytes = b""
) -> list[bytes]:
    """Sends `data` as the Chunks of the transaction."""
    chunks: list[bytes] = []
    sent = 0
    while True:
        room = eng.data_room(dev, peer, rid, role)
        part = data[sent : sent + room]
        sent += len(part)
        chunks.append(eng.send(dev, peer, rid, role, part, sent == len(data), NOW))
        if sent == len(data):
            return chunks


TABLE = [
    zm.PeResource("DeviceInfo"),
    zm.PeResource("X-Big", can_set="full"),
    zm.PeResource("X-Sub", can_subscribe=True),
    zm.PeResource("X-Mc", encodings=["ASCII", "Mcoded7"], schema=b'{"type":"object"}'),
]


def pair() -> tuple[zm.CiDevice, zm.PeEngine, zm.CiDevice, zm.PeEngine]:
    """A and B after Discovery and PE Capabilities in both directions."""
    a, b = device(1), device(2)
    pa, pb = zm.PeEngine(), zm.PeEngine(TABLE)
    for x, y, px, py in ((a, b, pa, pb), (b, a, pb, pa)):
        replies, _ = deliver(y, py, [x.discovery()])
        deliver(x, px, replies)
        replies, _ = deliver(y, py, [x.pe_capabilities_inquiry(y.muid)])
        _, events = deliver(x, px, replies)
        assert events == []
    return a, pa, b, pb


def test_resource_checks() -> None:
    """REQ-PY-PE-001: the Resource entries are checked when they are made."""
    r = TABLE[3]
    assert r.resource == "X-Mc" and r.can_set == "none" and r.encodings == ["ASCII", "Mcoded7"]
    assert r.schema == b'{"type":"object"}' and r.media_type is None
    for bad in ("Bad-Name", "X-", "", "A" * 37):
        with pytest.raises(zm.InvalidValueError):
            zm.PeResource(bad)
    with pytest.raises(zm.InvalidValueError):
        zm.PeResource("DeviceInfo", can_set="some")
    with pytest.raises(zm.InvalidValueError):
        zm.PeResource("DeviceInfo", encodings=[])
    with pytest.raises(zm.InvalidValueError):
        zm.PeResource("DeviceInfo", encodings=["UTF-8"])
    with pytest.raises(zm.InvalidValueError):
        zm.PeResource("DeviceInfo", media_type="json")
    assert (
        zm.PeResource("DeviceInfo", media_type="application/octet-stream").media_type
        == "application/octet-stream"
    )
    with pytest.raises(zm.InvalidValueError):
        zm.PeEngine([zm.PeResource("DeviceInfo", schema=b"x" * 600)])
    with pytest.raises(zm.InvalidValueError):
        zm.PeEngine().request(device(1), 0x1000_0000, "get", "DeviceInfo")


def test_resource_list_and_get_in_many_chunks() -> None:
    """REQ-PY-PE-002, -003, -005: the ResourceList of B, then a Get with a reply of 3 Chunks."""
    a, pa, b, pb = pair()
    # The engine sees no Property Exchange event in a message that is not Property Exchange.
    assert pa.receive(a, NOW) is None

    rid, msg = pa.request_resource_list(a, b.muid, NOW)
    _, events = deliver(b, pb, [msg])
    (ev,) = events
    assert (
        ev.kind == "message" and ev.role == "responder" and ev.txn == "get" and ev.end == "complete"
    )
    assert ev.resource == "ResourceList" and ev.resource_index is None and ev.request_id == rid
    assert ev.muid == a.muid and ev.header == b'{"resource":"ResourceList"}'
    listing = pb.resource_list()
    assert listing.startswith(b"[") and listing.endswith(b"]")
    assert b'"resource":"X-Sub","canSubscribe":true' in listing
    assert b'"schema":{"type":"object"}' in listing
    pb.reply(b, a.muid, rid, 200, length=len(listing))
    _, events = deliver(a, pa, send_all(b, pb, a.muid, rid, "responder", listing))
    assert (
        events[0].status == 200 and events[0].role == "initiator" and events[-1].end == "complete"
    )
    assert b"".join(e.data for e in events) == listing

    rid, missing = pa.request(a, b.muid, "get", "DeviceInfo")
    assert not missing  # A has the ResourceList of B now
    _, events = deliver(b, pb, send_all(a, pa, b.muid, rid, "initiator"))
    assert events[0].resource == "DeviceInfo" and events[0].resource_index == 0
    body = b'"' + bytes(ord("a") + i % 26 for i in range(1198)) + b'"'
    pb.reply(b, a.muid, rid, 200, length=len(body))
    with pytest.raises(zm.RejectedError):
        pb.reply(b, a.muid, rid, 200, length=len(body))  # the transaction sends now
    with pytest.raises(zm.InvalidValueError):
        pb.send(b, a.muid, rid, "responder", body, False, NOW)  # larger than the room
    chunks = send_all(b, pb, a.muid, rid, "responder", body)
    assert len(chunks) == 3
    _, events = deliver(a, pa, chunks)
    assert [e.end for e in events] == ["more", "more", "complete"]
    assert events[0].header != b"" and events[1].header == b"" and events[0].status == 200
    assert b"".join(e.data for e in events) == body
    # The same CiDevice event is not given to the engine again.
    assert pa.receive(a, NOW) is None
    with pytest.raises(zm.RejectedError):
        pb.send(b, a.muid, rid, "responder", b"", True, NOW)  # closed


def test_404_set_and_refusal() -> None:
    """REQ-PY-PE-003, -005: 404 for an unknown Resource, a refusal by the ResourceList of the peer, a
    Set of 2 Chunks."""
    a, pa, b, pb = pair()
    rid, missing = pa.request(a, b.muid, "get", "X-None")
    assert missing
    _, events = deliver(b, pb, send_all(a, pa, b.muid, rid, "initiator"))
    assert events == []
    _, events = deliver(a, pa, pending(b, pb))
    assert events[0].status == 404 and events[0].end == "complete"

    # REQ-PY-CI-007: a request between CiDevice.receive and PeEngine.receive of the last Chunk of
    # a reply gets another Request ID; the engine then closes only the old request.
    id1, _ = pa.request(a, b.muid, "get", "X-None")
    deliver(b, pb, send_all(a, pa, b.muid, id1, "initiator"))
    [reply] = pending(b, pb)
    a.receive(0, reply, NOW)
    id2, _ = pa.request(a, b.muid, "get", "X-None")
    assert id2 != id1
    ev = pa.receive(a, NOW)
    assert ev is not None and ev.request_id == id1 and ev.status == 404
    deliver(b, pb, send_all(a, pa, b.muid, id2, "initiator"))
    _, events = deliver(a, pa, pending(b, pb))
    assert events[0].request_id == id2 and events[0].status == 404

    with pytest.raises(zm.RejectedError, match="405"):
        pa.request(a, b.muid, "set", "DeviceInfo", length=2, caps=TABLE[0])
    with pytest.raises(zm.InvalidValueError):
        pa.request(a, b.muid, "put", "DeviceInfo")
    with pytest.raises(zm.InvalidValueError):
        pa.request(a, b.muid, "get", "DeviceInfo", encoding="UTF-8")
    with pytest.raises(zm.InvalidValueError):
        pa.request(a, b.muid, "get", "DeviceInfo", offset=0)

    data = bytes(range(32, 127)) * 7 + b"x" * 35  # 700 bytes
    rid, _ = pa.request(a, b.muid, "set", "X-Big", length=len(data), caps=TABLE[1])
    chunks = send_all(a, pa, b.muid, rid, "initiator", data)
    assert len(chunks) == 2
    _, events = deliver(b, pb, chunks)
    assert events[0].txn == "set" and events[0].resource_index == 1 and events[1].end == "complete"
    assert b"".join(e.data for e in events) == data
    pb.reply(b, a.muid, rid, 200)
    _, events = deliver(a, pa, send_all(b, pb, a.muid, rid, "responder"))
    assert events[0].txn == "set" and events[0].status == 200


def test_subscription() -> None:
    """REQ-PY-PE-004, -005, -006: a Subscription "start", a "full" update, refresh and "end"."""
    a, pa, b, pb = pair()
    rid, _ = pa.subscribe(a, b.muid, "X-Sub", caps=TABLE[2])
    _, events = deliver(b, pb, send_all(a, pa, b.muid, rid, "initiator"))
    (index,) = [e.subscription for e in events if e.kind == "subscription_started"]
    assert index is not None
    sub = pb.subscription(index)
    assert (
        sub is not None
        and sub.muid == a.muid
        and sub.role == "responder"
        and sub.resource == "X-Sub"
    )
    _, events = deliver(a, pa, pending(b, pb))
    (ev,) = events
    assert (
        ev.txn == "subscription"
        and ev.status == 200
        and ev.started is not None
        and ev.start_error is None
    )
    assert ev.subscribe_id == sub.subscribe_id
    mine = pa.subscription(ev.started)
    assert (
        mine is not None
        and mine.role == "initiator"
        and mine.muid == b.muid
        and mine.res_id is None
    )
    assert pa.subscription(3) is None and pa.subscription(99) is None

    with pytest.raises(zm.InvalidValueError):
        pb.update(b, index, "start")
    rid = pb.update(b, index, "full", length=20)
    _, events = deliver(a, pa, send_all(b, pb, a.muid, rid, "initiator", b"0123456789abcdefghij"))
    (ev,) = events
    assert ev.command == "full" and ev.role == "responder" and ev.subscription == 0
    assert ev.data == b"0123456789abcdefghij"
    _, events = deliver(b, pb, pending(a, pa))
    assert events[0].status == 200 and events[0].role == "initiator"

    with pytest.raises(zm.RejectedError):
        pa.refresh(99)
    pa.refresh(0)
    _, events = deliver(b, pb, pending(a, pa))
    (ev,) = events
    assert ev.txn == "get" and ev.resource == "X-Sub" and ev.request_id is not None
    pb.reply(b, a.muid, ev.request_id, 200, length=2)
    _, events = deliver(a, pa, send_all(b, pb, a.muid, ev.request_id, "responder", b"{}"))
    assert events[0].subscription == 0 and events[0].data == b"{}"

    rid = pa.unsubscribe(a, 0)
    assert pa.subscription(0) is None
    _, events = deliver(b, pb, send_all(a, pa, b.muid, rid, "initiator"))
    ended = events[-1]
    assert ended.kind == "subscription_ended" and ended.muid == a.muid and ended.resource == "X-Sub"
    assert pb.subscription(index) is None
    deliver(a, pa, pending(b, pb))
    assert pa.take_ended() is None


def test_wait_abort_timeouts() -> None:
    """REQ-PY-PE-003, -007: ACK 0x10, NAK 0x20, the 3-second timeout and a late reply (M2-103 12)."""
    a, pa, b, pb = pair()
    rid, _ = pa.request(a, b.muid, "get", "DeviceInfo")
    deliver(b, pb, send_all(a, pa, b.muid, rid, "initiator"))
    for bad in (0, 12701):
        with pytest.raises(zm.InvalidValueError):
            pb.wait(a.muid, rid, "responder", bad)
    with pytest.raises(zm.InvalidValueError):
        pb.wait(a.muid, rid, "observer", 500)
    pb.wait(a.muid, rid, "responder", 500)
    (ack,) = pending(b, pb)
    assert ack[3] == 0x7D  # MIDI-CI ACK
    deliver(a, pa, [ack])
    with pytest.raises(zm.RejectedError):
        pb.corrupt(a.muid, rid, "responder")  # no Flow Control
    pb.abort(b, a.muid, rid, "responder")
    with pytest.raises(zm.RejectedError):
        pb.abort(b, a.muid, rid, "responder")
    (nak,) = pending(b, pb)
    assert nak[3] == 0x7F  # MIDI-CI NAK
    _, events = deliver(a, pa, [nak])
    assert (
        events[0].kind == "closed"
        and events[0].reason == "terminated"
        and events[0].request_id == rid
    )

    rid, _ = pa.request(a, b.muid, "get", "DeviceInfo")
    send_all(a, pa, b.muid, rid, "initiator")
    assert pa.poll(a, NOW + 1_000_000) is None
    ev = pa.poll(a, NOW + 3_002_000)
    assert (
        ev is not None and ev.kind == "closed" and ev.reason == "timeout" and ev.request_id == rid
    )
    assert pa.poll(a, NOW + 3_002_000) is None
    (nak,) = pending(a, pa)
    assert (
        nak[3] == 0x7F and nak[13] == 0x34 and nak[14] == 0x42
    )  # NAK of a Get, 0x42 (M2-101 Table 16)

    rid, _ = pa.request(a, b.muid, "get", "DeviceInfo")
    deliver(b, pb, send_all(a, pa, b.muid, rid, "initiator"))
    with pytest.raises(zm.RejectedError):
        pb.reply(b, a.muid, rid, 200, length=2, now_us=NOW + 4_000_000)
    with pytest.raises(zm.RejectedError):
        pb.reply_status(b, a.muid, rid, 404)

    rid, _ = pa.request(a, b.muid, "get", "DeviceInfo")
    deliver(b, pb, send_all(a, pa, b.muid, rid, "initiator"))
    pb.reply_status(b, a.muid, rid, 413)
    _, events = deliver(a, pa, pending(b, pb))
    assert events[0].status == 413
