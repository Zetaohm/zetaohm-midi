"""Cost of a call through the Python package (docs/optimization-backlog.md OPT-029 step 4).

Wall-clock time on the host, not a gate: the numbers change with the machine and its load. Each
row gives the time of one call and the time of each MIDI message, so that the fixed cost of a call
(PyO3, the C ABI storage) shows against the work in the library.

    uv run python scripts/bench.py
"""

import timeit

import zetaohm_midi as zm

NOTES = bytes(b for i in range(64) for b in (0x90, i, 0x40))
WORDS = [0x20900040 | (i << 8) for i in range(64)]


def row(name: str, stmt: object, messages: int, number: int = 2000) -> None:
    best = min(timeit.repeat(stmt, number=number, repeat=5)) / number  # type: ignore[arg-type]
    print(f"{name:40} {best * 1e6:9.2f} us/call {best * 1e9 / messages:9.1f} ns/message")


def main() -> None:
    r = zm.BytesToUmp()
    w = zm.UmpToBytes(running_status=True)
    row("BytesToUmp.push, one byte", lambda: r.push(0xF8), 1, 20000)
    row("BytesToUmp.feed, 64 notes (192 bytes)", lambda: r.feed(NOTES), 64)
    row("BytesToUmp.feed, one note (3 bytes)", lambda: r.feed(NOTES[:3]), 1, 20000)
    row("UmpToBytes.write, one note", lambda: w.write(WORDS[0:1]), 1, 20000)


if __name__ == "__main__":
    main()
