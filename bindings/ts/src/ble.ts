// BLE-MIDI v1.0 packets (zmidi-ble through the C ABI): a packet reader that gives UMPs with their
// 13-bit timestamps, and a packet writer with running status (BLE-MIDI v1.0 sections 7 and 8).

import { Blocks, alloc, check, fn, int, layout, protocol, readBytes, readWords, scoped, sizeOf, u64, umpWords, view, writeBytes, writeWords } from "./wasm.js";

/** One UMP of a BLE-MIDI packet with the time of its message in milliseconds. */
export interface BleUmp {
  /** Read: the 13-bit timestamp of the sender clock (0 to 8191). Write: the time of the program. */
  readonly millis: number;
  /** The words of the UMP. */
  readonly words: readonly number[];
}

const READ_UMP = layout([
  ["timestamp_ms", "u16"],
  ["word_count", "u8"],
  ["words", ["u32", 4]],
]);

const TIMED_UMP = layout([
  ["millis", "u64"],
  ["word_count", "u8"],
  ["words", ["u32", 4]],
]);

/** Reads BLE-MIDI packets and gives UMPs. */
export class BleReader {
  readonly #b = new Blocks(this);

  /** A reader that gives UMPs on `group` (0 to 15) with protocol 1 (MIDI 1.0) or 2 (MIDI 2.0). */
  constructor(group = 0, protocolNumber = 1) {
    const p = this.#b.add(alloc(sizeOf("zmidi_ble_reader_size")));
    check(fn("zmidi_ble_reader_init")(p, int("group", group, 15), protocol(protocolNumber)), "BleReader");
  }

  /** Reads one packet. Gives its UMPs and the count of the parts of the packet that the reader dropped. */
  read(packet: Uint8Array): { umps: BleUmp[]; dropped: number } {
    const cap = Math.max(1, packet.length);
    const [pp, out, count, dropped] = this.#b.carve([cap, READ_UMP.size * cap, 4, 4]);
    writeBytes(pp, packet);
    check(fn("zmidi_ble_read")(this.#b.main, pp, packet.length, out, cap, count, dropped), "read");
    const v = view();
    const umps: BleUmp[] = [];
    for (let i = 0; i < v.getUint32(count, true); i++) {
      const at = out + READ_UMP.size * i;
      umps.push({
        millis: v.getUint16(at + READ_UMP.off["timestamp_ms"]!, true),
        words: readWords(at + READ_UMP.off["words"]!, v.getUint8(at + READ_UMP.off["word_count"]!)),
      });
    }
    return { umps, dropped: v.getUint32(dropped, true) };
  }

  /** Drops an open SysEx message and the last timestamp, for example after a new connection. */
  reset(): void {
    check(fn("zmidi_ble_reader_reset")(this.#b.main), "reset");
  }

  /** Frees the WASM memory of the reader now. */
  free(): void {
    this.#b.release();
  }
}

/** Writes UMPs into BLE-MIDI packets. */
export class BleWriter {
  readonly #b = new Blocks(this);

  /** A writer for the ATT MTU `attMtu` (23 or more), with running status when `runningStatus`. */
  constructor(attMtu = 23, runningStatus = true) {
    const p = this.#b.add(alloc(sizeOf("zmidi_ble_writer_size")));
    check(fn("zmidi_ble_writer_init")(p, int("attMtu", attMtu, 0xffff)), "BleWriter");
    check(fn("zmidi_ble_writer_set_running_status")(p, runningStatus ? 1 : 0), "BleWriter");
  }

  /** Sets a new ATT MTU (23 or more). */
  setAttMtu(attMtu: number): void {
    check(fn("zmidi_ble_writer_set_att_mtu")(this.#b.main, int("attMtu", attMtu, 0xffff)), "setAttMtu");
  }

  /** The largest packet in bytes: the ATT MTU less 3. */
  get packetLen(): number {
    return fn("zmidi_ble_writer_packet_len")(this.#b.main) >>> 0;
  }

  /** True when the writer holds bytes for the next packet (a SysEx part). */
  get hasHeld(): boolean {
    return fn("zmidi_ble_writer_has_held")(this.#b.main) !== 0;
  }

  /** Drops the held bytes and the running status; the next packet starts a new stream. */
  reset(): void {
    check(fn("zmidi_ble_writer_reset")(this.#b.main), "reset");
  }

  /**
   * Writes one packet from the held bytes and then from `umps`. Gives the packet (empty: nothing to
   * send) and the count of UMPs taken from the start of `umps`. Times never decrease in a stream.
   */
  writePacket(umps: readonly BleUmp[]): { packet: Uint8Array; taken: number } {
    const n = this.packetLen;
    const count = Math.max(1, umps.length);
    const [pu, out, len, taken] = this.#b.carve([TIMED_UMP.size * count, n, 4, 4]);
    umps.forEach((u, i) => {
      const w = umpWords(u.words);
      const at = pu + TIMED_UMP.size * i;
      const v = view();
      v.setBigUint64(at + TIMED_UMP.off["millis"]!, u64("millis", u.millis), true);
      v.setUint8(at + TIMED_UMP.off["word_count"]!, w.length);
      writeWords(at + TIMED_UMP.off["words"]!, w);
    });
    check(fn("zmidi_ble_write_packet")(this.#b.main, pu, umps.length, out, n, len, taken), "writePacket");
    const v = view();
    return { packet: readBytes(out, v.getUint32(len, true)), taken: v.getUint32(taken, true) };
  }

  /** Frees the WASM memory of the writer now. */
  free(): void {
    this.#b.release();
  }
}
