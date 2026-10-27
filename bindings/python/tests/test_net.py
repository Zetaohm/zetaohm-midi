"""Network MIDI 2.0 (REQ-PY-NET-*): a Host and a Client in memory, M2-124."""

import pytest

import zetaohm_midi as zm

HOST = ("192.168.1.10", 5673)
CLIENT = ("192.168.1.20", 50000)


def pump(
    host: zm.NetHost, client: zm.NetClient, now: int
) -> tuple[list[zm.NetEvent], list[zm.NetEvent]]:
    at_host: list[zm.NetEvent] = []
    at_client: list[zm.NetEvent] = []
    for _ in range(20):
        moved = False
        for _to, payload in client.write_pending(now):
            events, _ = host.receive(now, CLIENT, payload)
            at_host += events
            moved = True
        for to, payload in host.write_pending(now):
            assert to == CLIENT
            events, _ = client.receive(now, HOST, payload)
            at_client += events
            moved = True
        if not moved:
            break
        now += 10
    return at_host, at_client


def kinds(events: list[zm.NetEvent]) -> list[str]:
    return [e.kind for e in events]


def test_session_data_ping_bye() -> None:
    host, client = zm.NetHost("zmidi Host", "host-1"), zm.NetClient("zmidi Client", "client-1")
    client.connect(0, HOST)
    h, c = pump(host, client, 0)
    assert "established" in kinds(h) and "established" in kinds(c)

    client.send_ump(HOST, [0x40903C00, 0xC0000000])
    h, _ = pump(host, client, 100)
    assert [e.umps for e in h if e.kind == "data"] == [[[0x40903C00, 0xC0000000]]]

    pid = client.ping(200, HOST)
    _, c = pump(host, client, 200)
    assert [e.code for e in c if e.kind == "ping_reply"] == [pid]

    c = client.bye(300, HOST, 1)
    h, c2 = pump(host, client, 300)
    assert "closed" in kinds(h) and "closed" in kinds(c + c2)


def test_bad_values() -> None:
    with pytest.raises(zm.InvalidValueError):
        zm.NetHost("", "x")
    host = zm.NetHost("h", "p")
    with pytest.raises(zm.InvalidValueError):
        host.send_ump(("not an ip", 1), [0])
    with pytest.raises(zm.RejectedError):
        host.send_ump(CLIENT, [0x20903C64])


# --- Authentication (M2-124 6.7 to 6.10, zmidi-net auth) ---

SHARED_SECRET, USER = 1, 2


def one(payloads: list[tuple[tuple[str, int], bytes]]) -> bytes:
    assert len(payloads) == 1
    return payloads[0][1]


def test_auth_shared_secret_example_6_9() -> None:
    """M2-124 6.9: the CryptoNonce, the shared secret "5483" and the Authentication Digest."""
    nonce = b"nUWrn*@#$hjfwnkL"
    digest = bytes.fromhex("676EBE82587CECA8F82FC333D787951EEC2B00AD31613CC4DB18CF27373AFB82")
    host = zm.NetHost("Host", "P1", auth=SHARED_SECRET)
    host.set_shared_secret(b"5483")
    # The Host makes each CryptoNonce character from a random byte plus 32 (M2-124 6.7).
    host.add_random(bytes(b - 32 for b in nonce))
    client = zm.NetClient("C", "c1", auth=SHARED_SECRET)
    client.set_shared_secret(b"5483")
    client.connect(0, HOST)

    # Invitation (0x01) with the Capabilities bit D0 (Table 10).
    inv = one(client.write_pending(0))
    assert inv[4] == 0x01 and inv[7] & 0x03 == 0x01
    assert host.receive(0, CLIENT, inv)[0] == [] and host.random_left == 0
    # Invitation Reply: Authentication Required (0x12), state 0x00, the CryptoNonce (Table 15).
    req = one(host.write_pending(0))
    assert (req[4], req[7]) == (0x12, 0x00) and req[8:24] == nonce
    events, _ = client.receive(10, HOST, req)
    assert [(e.kind, e.method, e.auth_state, e.code, e.value) for e in events] == [
        ("auth_requested", SHARED_SECRET, 0, SHARED_SECRET, 0)
    ]
    # Invitation with Authentication (0x02, Table 18): pl 8, the digest of the example.
    ia = one(client.write_pending(10))
    assert ia[:8] == b"MIDI\x02\x08\x00\x00" and ia[8:40] == digest
    events, _ = host.receive(20, CLIENT, ia)
    assert [(e.kind, e.method, e.code) for e in events] == [
        ("established", SHARED_SECRET, SHARED_SECRET)
    ]
    acc = one(host.write_pending(20))
    assert acc[4] == 0x10
    events, _ = client.receive(30, HOST, acc)
    assert [(e.kind, e.method) for e in events] == [("established", SHARED_SECRET)]


def test_auth_user_example_6_10() -> None:
    """M2-124 6.10: the CryptoNonce, the Username "Rosa", the password and the digest."""
    nonce = b"XI|~=NNRVaD;XCPL"
    digest = bytes.fromhex("3A2D5EBEEF92E8463535C27FB4B7B3E2D1ACF57A844C6D0808E041C102B7FF1A")
    host = zm.NetHost("Host", "P1", auth=USER)
    host.set_users({"Ana": b"other", "Rosa": b"RPBqBno"})
    host.add_random(bytes(b - 32 for b in nonce))
    client = zm.NetClient("C", "c1", auth=USER)
    client.set_user("Rosa", b"RPBqBno")
    client.connect(0, HOST)

    inv = one(client.write_pending(0))
    assert inv[7] & 0x03 == 0x02
    host.receive(0, CLIENT, inv)
    # Invitation Reply: User Authentication Required (0x13), state 0x00 (Table 17).
    req = one(host.write_pending(0))
    assert (req[4], req[7]) == (0x13, 0x00) and req[8:24] == nonce
    events, _ = client.receive(10, HOST, req)
    assert [(e.kind, e.method, e.auth_state) for e in events] == [("auth_requested", USER, 0)]
    # Invitation with User Authentication (0x03, Table 19): pl 9, the digest, then "Rosa".
    ia = one(client.write_pending(10))
    assert ia[:8] == b"MIDI\x03\x09\x00\x00" and ia[8:40] == digest and ia[40:44] == b"Rosa"
    events, _ = host.receive(20, CLIENT, ia)
    assert [(e.kind, e.method) for e in events] == [("established", USER)]


def pump_auth(
    host: zm.NetHost, client: zm.NetClient, now: int
) -> tuple[list[zm.NetEvent], list[zm.NetEvent]]:
    """Moves all payloads (pending and replies); when nothing moves, goes to the next short
    deadline and polls."""
    at_host: list[zm.NetEvent] = []
    at_client: list[zm.NetEvent] = []
    to_host: list[bytes] = []
    to_client: list[bytes] = []
    for _ in range(60):
        to_host += [p for _, p in client.write_pending(now)]
        to_client += [p for _, p in host.write_pending(now)]
        moved = bool(to_host or to_client)
        while to_host or to_client:
            if to_host:
                events, replies = host.receive(now, CLIENT, to_host.pop(0))
                at_host += events
                to_client += [p for _, p in replies]
            if to_client:
                events, replies = client.receive(now, HOST, to_client.pop(0))
                at_client += events
                to_host += [p for _, p in replies]
        if moved:
            now += 10
            continue
        deadlines = [d for d in (host.next_deadline(), client.next_deadline()) if d is not None]
        if not deadlines or min(deadlines) > now + 5000:
            break
        now = max(now, min(deadlines))
        at_host += host.poll(now)
        at_client += client.poll(now)
    return at_host, at_client


@pytest.mark.parametrize(
    ("name", "password", "not_found", "state", "bye"),
    [("Rosa", b"wrong", False, 0x01, 0x43), ("Nobody", b"RPBqBno", True, 0x02, 0x44)],
)
def test_auth_failures_end_with_bye(
    name: str, password: bytes, not_found: bool, state: int, bye: int
) -> None:
    """A wrong password or an unknown Username fails up to the failure limit (5); then the Host
    ends the Invitation with Bye 0x43 or 0x44 (M2-124 6.10, Table 27)."""
    host = zm.NetHost("Host", "P1", auth=USER)
    host.set_users({"Rosa": b"RPBqBno"})
    host.add_random(bytes(range(64)))
    client = zm.NetClient("zmidi Client", "client-1", auth=USER)
    client.set_user(name, password)
    client.connect(0, HOST)
    h, c = pump_auth(host, client, 0)
    failed = [e for e in h if e.kind == "auth_failed"]
    assert len(failed) == 5 and failed[-1].value == 5
    assert all(e.method == USER and e.username_not_found is not_found for e in failed)
    requested = [e for e in c if e.kind == "auth_requested"]
    assert len(requested) >= 2 and requested[-1].auth_state == state
    assert "established" not in kinds(h) + kinds(c)
    closed = [e for e in c if e.kind == "closed"]
    assert [(e.code, e.value) for e in closed] == [(0, bye)]
    assert host.random_left == 48


def test_auth_both_methods() -> None:
    """A Host with both methods (auth=3) asks a Client with both Capabilities bits for the user
    method (N-44) and a Client with D0 only for the shared secret method."""
    for client_auth, method in ((SHARED_SECRET | USER, USER), (SHARED_SECRET, SHARED_SECRET)):
        host = zm.NetHost("Host", "P1", auth=SHARED_SECRET | USER)
        host.set_shared_secret(b"5483")
        host.set_users({"Rosa": b"RPBqBno"})
        host.add_random(bytes(range(16)))
        client = zm.NetClient("zmidi Client", "client-1", auth=client_auth)
        client.set_shared_secret(b"5483")
        client.set_user("Rosa", b"RPBqBno")
        client.connect(0, HOST)
        h, c = pump_auth(host, client, 0)
        requested = [e for e in c if e.kind == "auth_requested"]
        assert requested and requested[0].method == method
        established = [e for e in h if e.kind == "established"]
        assert len(established) == 1 and established[0].method == method


def test_auth_random_source() -> None:
    """With no add_random, the Host takes its CryptoNonce bytes from os.urandom; a program can
    give its own source. Each new session gets a new CryptoNonce (M2-124 6.7)."""
    host = zm.NetHost("Host", "P1", auth=SHARED_SECRET)
    host.set_shared_secret(b"5483")
    for _ in range(2):
        client = zm.NetClient("C", "c1", auth=SHARED_SECRET)
        client.set_shared_secret(b"5483")
        client.connect(0, HOST)
        h, _ = pump_auth(host, client, 0)
        assert [e.method for e in h if e.kind == "established"] == [SHARED_SECRET]
        client.bye(0, HOST)
        pump_auth(host, client, 0)
    assert host.random_left == 32

    asked: list[int] = []

    def source(n: int) -> bytes:
        asked.append(n)
        return bytes(range(n))

    host = zm.NetHost("Host", "P1", auth=SHARED_SECRET, random=source)
    host.set_shared_secret(b"5483")
    client = zm.NetClient("C", "c1", auth=SHARED_SECRET)
    client.set_shared_secret(b"5483")
    client.connect(0, HOST)
    pump_auth(host, client, 0)
    assert asked == [64] and host.random_left == 48


def test_auth_credentials_errors() -> None:
    # A Host with no random bytes can not ask for authentication.
    host = zm.NetHost("Host", "P1", auth=SHARED_SECRET, random=lambda n: b"")
    host.set_shared_secret(b"5483")
    client = zm.NetClient("C", "c1", auth=SHARED_SECRET)
    client.set_shared_secret(b"5483")
    client.connect(0, HOST)
    h, c = pump_auth(host, client, 0)
    errors = [e for e in h if e.kind == "credentials_error"]
    assert errors and errors[0].credentials_error == "no_random" and errors[0].code == 0
    assert "established" not in kinds(h) + kinds(c)

    # A Client with an empty Username declines (Bye 0x80).
    host = zm.NetHost("Host", "P1", auth=USER)
    host.set_users({"Rosa": b"RPBqBno"})
    host.add_random(bytes(16))
    client = zm.NetClient("C", "c1", auth=USER)
    client.set_user("", b"RPBqBno")
    client.connect(0, HOST)
    h, c = pump_auth(host, client, 0)
    errors = [e for e in c if e.kind == "credentials_error"]
    assert errors and errors[0].credentials_error == "invalid_username" and errors[0].code == 4
    assert "established" not in kinds(h) + kinds(c)


def test_auth_bad_values() -> None:
    for auth in (4, 255):
        with pytest.raises(zm.InvalidValueError):
            zm.NetHost("Host", "P1", auth=auth)
        with pytest.raises(zm.InvalidValueError):
            zm.NetClient("C", "c1", auth=auth)
    with pytest.raises(zm.InvalidValueError):
        zm.NetClient("", "c1", auth=SHARED_SECRET)
    # A Client with no Capabilities bit gets Bye 0x45 from a Host that needs authentication.
    host = zm.NetHost("Host", "P1", auth=3)
    client = zm.NetClient("C", "c1")
    client.connect(0, HOST)
    h, c = pump_auth(host, client, 0)
    assert [(e.code, e.value) for e in c if e.kind == "closed"] == [(0, 0x45)]
    assert "established" not in kinds(h)
