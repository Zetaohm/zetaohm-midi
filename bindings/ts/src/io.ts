// The MIDI 1.0 byte stream to UMP and back (zmidi-io through the C ABI): M2-104 Appendix D, MIDI 1.0
// running status and System Real Time between data bytes.

import {
  Blocks,
  alloc,
  check,
  fn,
  int,
  protocol,
  readBytes,
  readWords,
  sizeOf,
  splitUmps,
  umpWords,
  view,
  writeBytes,
  writeWords,
} from "./wasm.js";

/** The most bytes that one feed call copies into WASM memory. */
const FEED_CHUNK = 4096;

/** Reads a MIDI 1.0 byte stream and gives UMPs: MIDI 1.0 Protocol (Message Type 0x2) or MIDI 2.0 (0x4). */
export class BytesToUmp {
  readonly #b = new Blocks(this);

  /** A reader for Group `group` (0 to 15) and protocol `protocol` (1: MIDI 1.0, 2: MIDI 2.0). */
  constructor(group = 0, protocolNumber = 1) {
    const p = this.#b.add(alloc(sizeOf("zmidi_bytes_to_ump_size")));
    check(fn("zmidi_bytes_to_ump_init")(p, int("group", group, 15), protocol(protocolNumber)), "BytesToUmp");
  }

  /** Reads one byte; gives the UMPs that it completes. */
  push(byte: number): number[][] {
    int("byte", byte, 255);
    const [out, count] = this.#b.carve([16, 4]);
    check(fn("zmidi_bytes_to_ump_push")(this.#b.main, byte, out, 4, count), "push");
    return splitUmps(readWords(out, view().getUint32(count, true)));
  }

  /** Reads bytes; gives the UMPs that they complete, in order. One C call reads many bytes (OPT-035). */
  feed(data: Uint8Array): number[][] {
    const out: number[][] = [];
    const chunk = Math.min(data.length, FEED_CHUNK);
    // At most 4 words for each byte, but far fewer in a real stream: the call stops when the
    // buffer is nearly full, and the loop gives it the rest.
    const cap = 2 * chunk + 4;
    const [pd, pw, read, count] = this.#b.carve([chunk, 4 * cap, 4, 4]);
    for (let at = 0; at < data.length; ) {
      const part = data.subarray(at, at + chunk);
      writeBytes(pd, part);
      for (let done = 0; done < part.length; ) {
        check(fn("zmidi_bytes_to_ump_feed")(this.#b.main, pd + done, part.length - done, pw, cap, read, count), "feed");
        const v = view();
        out.push(...splitUmps(readWords(pw, v.getUint32(count, true))));
        done += v.getUint32(read, true);
      }
      at += part.length;
    }
    return out;
  }

  /** Frees the WASM memory of the reader now. */
  free(): void {
    this.#b.release();
  }
}

/** Writes UMPs (Message Types 0x1, 0x2, 0x3) as a MIDI 1.0 byte stream. */
export class UmpToBytes {
  readonly #b = new Blocks(this);

  /** A writer; `runningStatus` leaves out a status byte equal to the last one. */
  constructor(runningStatus = false) {
    const p = this.#b.add(alloc(sizeOf("zmidi_ump_to_bytes_size")));
    check(fn("zmidi_ump_to_bytes_init")(p, runningStatus ? 1 : 0), "UmpToBytes");
  }

  /** Writes one UMP; gives its bytes (empty for a UMP with no MIDI 1.0 form). */
  write(words: readonly number[]): Uint8Array {
    const w = umpWords(words);
    const [pw, out, count] = this.#b.carve([16, 16, 4]);
    writeWords(pw, w);
    check(fn("zmidi_ump_to_bytes_write")(this.#b.main, pw, w.length, out, 16, count), "write");
    return readBytes(out, view().getUint32(count, true));
  }

  /** Makes the next message send its status byte (a refresh of running status). */
  refreshStatus(): void {
    check(fn("zmidi_ump_to_bytes_refresh_status")(this.#b.main), "refreshStatus");
  }

  /** Frees the WASM memory of the writer now. */
  free(): void {
    this.#b.release();
  }
}
