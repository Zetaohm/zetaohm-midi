// MIDI Clip Files (zmidi-file through the C ABI, M2-116): a reader of all messages and a writer that
// checks each call against the rules of zmidi-file.

import { InvalidValueError, MalformedError, RejectedError } from "./errors.js";
import {
  Blocks,
  Status,
  alloc,
  allocBytes,
  check,
  fn,
  free,
  int,
  layout,
  readBytes,
  readWords,
  scoped,
  sizeOf,
  u64,
  umpWords,
  view,
  writeBytes,
  writeWords,
} from "./wasm.js";

/** One message of a clip. */
export interface ClipEvent {
  /** "config": the Clip Configuration Header; "sequence": the Clip Sequence Data. */
  readonly section: "config" | "sequence";
  /** Absolute ticks since the start of the file, in the unit of the DCTPQ in force. */
  readonly ticks: number;
  /** The words of the UMP. */
  readonly words: readonly number[];
}

/** One Set Profile On message of the Clip Configuration Header. */
export interface ClipProfile {
  /** Profile ID, 5 bytes of 7-bit data. */
  readonly profileId: Uint8Array;
  /** "channel" (one Channel, `channel`), "group" or "functionBlock". */
  readonly destination: "channel" | "group" | "functionBlock";
  /** Channel, 0 to 15, when the destination is "channel". */
  readonly channel?: number;
  /** Number of Channels Requested (14 bits). */
  readonly channels?: number;
}

/** The Clip Configuration Header of a new clip. */
export interface ClipOptions {
  /** Ticks Per Quarter Note of the DCTPQ, 1 to 65535 (default 96). */
  readonly ticksPerQuarterNote?: number;
  /** Group of the Flex Data and Profile messages, 0 to 15 (default 0). */
  readonly group?: number;
  /** Set Tempo in units of 10 ns per quarter note, when given. */
  readonly tempo10ns?: number;
  /** Set Time Signature `[numerator, denominator as a power of 2, 1/32 notes]`, when given. */
  readonly timeSignature?: readonly [number, number, number];
  /** Set Profile On messages (at most 16). */
  readonly profiles?: readonly ClipProfile[];
}

const EVENT = layout([
  ["section", "u8"],
  ["ticks", "u64"],
  ["word_count", "u8"],
  ["words", ["u32", 4]],
]);

const PROFILE = layout([
  ["profile_id", ["u8", 5]],
  ["destination", "u8"],
  ["channel", "u8"],
  ["channels", "u16"],
]);

const CONFIG = layout([
  ["tpq", "u16"],
  ["group", "u8"],
  ["has_tempo", "bool"],
  ["tempo", "u32"],
  ["has_ts", "bool"],
  ["numerator", "u8"],
  ["denominator", "u8"],
  ["thirty_second", "u8"],
  ["profiles", "ptr"],
  ["profile_count", "usize"],
]);

/** ZmidiClipError values that the package reads. */
const CLIP_BUFFER_TOO_SMALL = 4;

function clipError(code: number, what: string): Error {
  const text = [
    "no error",
    "the bytes do not start with the File Header",
    "a trailing partial word, or a UMP cut by the end of the data",
    "a byte follows End of Clip",
    "the buffer cannot hold the bytes",
    "a bad clip configuration",
    "the words are not one whole UMP",
    "a tick earlier than the last one",
    "a Property Exchange message",
    "a message that the writer writes itself",
    "Set Tempo off the 1/24 quarter note grid",
    "Set Time Signature not at a bar boundary",
    "a different tick inside one multi-UMP Lyrics or Ruby message",
    "the call is not allowed in the current phase",
  ][code] ?? `clip error ${code}`;
  const message = `${what}: ${text}`;
  if (code >= 1 && code <= 3) {
    return new MalformedError(message);
  }
  if (code === 5 || code === 6) {
    return new InvalidValueError(message);
  }
  return new RejectedError(message);
}

/** Reads all messages of a MIDI Clip File. Throws MalformedError for a bad file. */
export function readClip(data: Uint8Array): ClipEvent[] {
  const pd = allocBytes(data);
  try {
    return scoped([sizeOf("zmidi_clip_reader_size"), EVENT.size, 4, 4], (reader, ev, has, err) => {
      const st = fn("zmidi_clip_reader_init")(reader, pd, data.length, err);
      if (st !== Status.OK) {
        throw clipError(view().getInt32(err, true), "readClip");
      }
      const out: ClipEvent[] = [];
      for (;;) {
        const s = fn("zmidi_clip_reader_next")(reader, ev, has, err);
        if (s !== Status.OK) {
          throw clipError(view().getInt32(err, true), "readClip");
        }
        const v = view();
        if (v.getUint8(has) === 0) {
          return out;
        }
        out.push({
          section: v.getUint8(ev + EVENT.off["section"]!) === 0 ? "config" : "sequence",
          ticks: Number(v.getBigUint64(ev + EVENT.off["ticks"]!, true)),
          words: readWords(ev + EVENT.off["words"]!, v.getUint8(ev + EVENT.off["word_count"]!)),
        });
      }
    });
  } finally {
    free(pd);
  }
}

type Step = { kind: "config"; words: readonly number[] } | { kind: "start"; tick: number; tempo10ns: number | undefined } | { kind: "event"; tick: number; words: readonly number[] };

/**
 * Writes a MIDI Clip File. Each call is checked at once against the rules of zmidi-file; a call
 * that fails changes nothing. `finish` ends the clip and gives its bytes.
 */
export class ClipWriter {
  readonly #b = new Blocks(this);
  readonly #group: number;
  readonly #steps: Step[] = [];
  readonly #config: Uint8Array;
  #buf = 0;
  #capacity = 1024;

  /** A clip with the Clip Configuration Header of `options`. */
  constructor(options: ClipOptions = {}) {
    const tpq = int("ticksPerQuarterNote", options.ticksPerQuarterNote ?? 96, 0xffff);
    this.#group = int("group", options.group ?? 0, 15);
    const profiles = options.profiles ?? [];
    if (profiles.length > 16) {
      throw new InvalidValueError("a clip has at most 16 Set Profile On messages");
    }
    // The configuration as bytes: CONFIG, then the profiles. The C writer copies it at init.
    const bytes = new Uint8Array(CONFIG.size + PROFILE.size * profiles.length);
    const v = new DataView(bytes.buffer);
    v.setUint16(CONFIG.off["tpq"]!, tpq, true);
    v.setUint8(CONFIG.off["group"]!, this.#group);
    if (options.tempo10ns !== undefined) {
      v.setUint8(CONFIG.off["has_tempo"]!, 1);
      v.setUint32(CONFIG.off["tempo"]!, int("tempo10ns", options.tempo10ns, 0xffffffff), true);
    }
    if (options.timeSignature !== undefined) {
      const [n, d, t] = options.timeSignature;
      v.setUint8(CONFIG.off["has_ts"]!, 1);
      v.setUint8(CONFIG.off["numerator"]!, int("numerator", n, 255));
      v.setUint8(CONFIG.off["denominator"]!, int("denominator", d, 255));
      v.setUint8(CONFIG.off["thirty_second"]!, int("thirtySecondNotes", t, 255));
    }
    v.setUint32(CONFIG.off["profile_count"]!, profiles.length, true);
    profiles.forEach((p, i) => {
      const at = CONFIG.size + PROFILE.size * i;
      if (p.profileId.length !== 5) {
        throw new InvalidValueError("a Profile ID has 5 bytes");
      }
      bytes.set(p.profileId, at + PROFILE.off["profile_id"]!);
      const dest = { channel: 0, group: 1, functionBlock: 2 }[p.destination];
      if (dest === undefined) {
        throw new InvalidValueError("the destination is channel, group or functionBlock");
      }
      v.setUint8(at + PROFILE.off["destination"]!, dest);
      v.setUint8(at + PROFILE.off["channel"]!, int("channel", p.channel ?? 0, 15));
      v.setUint16(at + PROFILE.off["channels"]!, int("channels", p.channels ?? 0, 0x3fff), true);
    });
    this.#config = bytes;
    this.#b.add(alloc(sizeOf("zmidi_clip_writer_size")));
    this.#rebuild();
  }

  /** Writes one more message of the Clip Configuration Header. */
  configEvent(words: readonly number[]): void {
    this.#step({ kind: "config", words: umpWords(words).slice() });
  }

  /** Writes Start of Clip at `tick`, with a Set Tempo when given, and starts the Clip Sequence Data. */
  start(tick = 0, tempo10ns?: number): void {
    u64("tick", tick);
    if (tempo10ns !== undefined) {
      int("tempo10ns", tempo10ns, 0xffffffff);
    }
    this.#step({ kind: "start", tick, tempo10ns });
  }

  /** Writes one message of the Clip Sequence Data at absolute `tick`. */
  event(tick: number, words: readonly number[]): void {
    u64("tick", tick);
    this.#step({ kind: "event", tick, words: umpWords(words).slice() });
  }

  /** Writes End of Clip at `tick` and gives the bytes of the clip. Later calls throw RejectedError. */
  finish(tick: number): Uint8Array {
    const end = u64("tick", tick);
    for (;;) {
      const r = scoped([4, 4], (err, len) => {
        const st = fn("zmidi_clip_writer_finish")(this.#b.main, end, len, err);
        return this.#failed(st, err, "finish") ? undefined : readBytes(this.#buf, view().getUint32(len, true));
      });
      if (r !== undefined) {
        this.#b.drop(this.#buf);
        this.#buf = 0;
        return r;
      }
      this.#grow();
    }
  }

  /** Frees the WASM memory of the writer now. */
  free(): void {
    this.#buf = 0;
    this.#b.release();
  }

  /** Runs `step` on the C writer and keeps it; a buffer too small makes a larger one first. */
  #step(step: Step): void {
    if (this.#buf === 0) {
      throw new RejectedError("the clip is finished");
    }
    for (;;) {
      let full: boolean;
      try {
        full = this.#call(step);
      } catch (e) {
        // A failed call can leave the C writer in a changed state: make it again from the steps.
        this.#rebuild();
        throw e;
      }
      if (!full) {
        this.#steps.push(step);
        return;
      }
      this.#grow();
    }
  }

  /** Doubles the buffer and runs the kept steps again. */
  #grow(): void {
    this.#capacity *= 2;
    this.#rebuild();
  }

  /** Sets up the C writer in a new buffer of `#capacity` bytes and runs the kept steps. */
  #rebuild(): void {
    for (;;) {
      this.#b.drop(this.#buf);
      this.#buf = this.#b.add(alloc(this.#capacity));
      const cfg = this.#config;
      const full = scoped([cfg.length, 4], (pc, err) => {
        writeBytes(pc, cfg);
        if (cfg.length > CONFIG.size) {
          view().setUint32(pc + CONFIG.off["profiles"]!, pc + CONFIG.size, true);
        }
        return this.#failed(fn("zmidi_clip_writer_init")(this.#b.main, this.#buf, this.#capacity, pc, err), err, "ClipWriter");
      });
      if (!full && !this.#steps.some((s) => this.#call(s))) {
        return;
      }
      this.#capacity *= 2;
    }
  }

  /** Runs one step on the C writer; true when the buffer is too small. */
  #call(s: Step): boolean {
    return scoped([16, 4], (pw, err) => {
      let st: number;
      if (s.kind === "start") {
        const has = s.tempo10ns !== undefined;
        st = fn("zmidi_clip_writer_start")(this.#b.main, BigInt(s.tick), has ? 1 : 0, this.#group, s.tempo10ns ?? 0, err);
      } else {
        writeWords(pw, s.words);
        st =
          s.kind === "config"
            ? fn("zmidi_clip_writer_config_event")(this.#b.main, pw, s.words.length, err)
            : fn("zmidi_clip_writer_event")(this.#b.main, BigInt(s.tick), pw, s.words.length, err);
      }
      return this.#failed(st, err, s.kind);
    });
  }

  /** False for OK, true for a buffer too small; throws the error of any other status. */
  #failed(st: number, err: number, what: string): boolean {
    if (st === Status.OK) {
      return false;
    }
    const code = view().getInt32(err, true);
    if (code === CLIP_BUFFER_TOO_SMALL) {
      return true;
    }
    if (code === 0) {
      check(st, what);
    }
    throw clipError(code, what);
  }
}
