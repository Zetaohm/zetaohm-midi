// The WASM build of the zmidi-ffi C ABI (DEC-019): the instance, the memory of the program inside
// the WASM linear memory, the C struct layouts of wasm32, and the map of a status to an error.
// The package is the "program" of the C ABI: it gives the storage of each object and frees it.

import { InvalidValueError, MidiError, RejectedError } from "./errors.js";

/** A function of the C ABI. Every function returns an i32 in wasm32; a u64 argument is a bigint. */
export type CFn = (...args: (number | bigint)[]) => number;

/** The major version of the C ABI that this package reads (ZMIDI_ABI_MAJOR). */
export const ABI_MAJOR = 0;

/**
 * The lowest minor version of the C ABI that this package needs (ZMIDI_ABI_MINOR): 2 gives the
 * MIDI-CI Profile Configuration and Process Inquiry calls and the Network MIDI 2.0 authentication;
 * 3 gives the Property Exchange engine (zmidi_pe_*); 4 gives zmidi_ci_pe_end and the free of the
 * Request ID at the last Chunk of a reply; 5 gives the USB descriptor writer and the Group Terminal
 * Block requests (zmidi_usb_function_*, zmidi_usb_gtb_*); 7 gives zmidi_bytes_to_ump_feed (many bytes in one
 * call). 6 adds only a C call that the package does not use.
 */
export const ABI_MINOR = 7;

interface State {
  exports: Record<string, CFn>;
  memory: WebAssembly.Memory;
  /** Start of the heap of the package: the end of the memory that Rust uses. */
  base: number;
  /** End of the used heap. */
  top: number;
  /** Free blocks: address to size, both multiples of 16. */
  free: Map<number, number>;
  /** Live blocks: address to size. */
  live: Map<number, number>;
}

let state: State | undefined;

/** Sets up the package from an instance of the WASM build. Gives the ABI version. */
export function attach(instance: WebAssembly.Instance): number {
  const exports = instance.exports as unknown as Record<string, CFn> & { memory: WebAssembly.Memory };
  const version = exports.zmidi_abi_version!() >>> 0;
  if (version >>> 16 !== ABI_MAJOR || (version & 0xffff) < ABI_MINOR) {
    throw new MidiError(
      `the WASM file has C ABI ${version >>> 16}.${version & 0xffff}; this package reads ${ABI_MAJOR}.${ABI_MINOR} or a later ${ABI_MAJOR}.x`,
    );
  }
  // Rust uses the memory up to its initial size (stack and data) and has no allocator, so the
  // heap of the package starts at the initial end of the memory.
  const base = exports.memory.buffer.byteLength;
  state = { exports, memory: exports.memory, base, top: base, free: new Map(), live: new Map() };
  return version;
}

/** True when `init` or `initSync` set up the package. */
export function attached(): boolean {
  return state !== undefined;
}

function s(): State {
  if (state === undefined) {
    throw new MidiError("call init() or initSync() before you use the package");
  }
  return state;
}

/** The function `name` of the C ABI. */
export function fn(name: string): CFn {
  const f = s().exports[name];
  if (f === undefined) {
    throw new MidiError(`the WASM file has no function ${name}`);
  }
  return f;
}

/** The size in bytes of a Rust type (a `*_size` function, DEC-019 rule 6). */
export function sizeOf(name: string): number {
  return fn(name)() >>> 0;
}

// --- memory ---

const ALIGN = 16;

/** Gives `size` bytes of zeroed memory, aligned to 16 (the largest alignment of the C ABI). */
export function alloc(size: number): number {
  const st = s();
  const n = Math.max(ALIGN, Math.ceil(size / ALIGN) * ALIGN);
  let ptr = -1;
  for (const [at, len] of st.free) {
    if (len >= n) {
      ptr = at;
      st.free.delete(at);
      if (len > n) {
        st.free.set(at + n, len - n);
      }
      break;
    }
  }
  if (ptr < 0) {
    ptr = st.top;
    st.top += n;
    const have = st.memory.buffer.byteLength;
    if (st.top > have) {
      st.memory.grow(Math.ceil((st.top - have) / 65536));
    }
  }
  st.live.set(ptr, n);
  new Uint8Array(st.memory.buffer, ptr, n).fill(0);
  return ptr;
}

/** Frees a block from `alloc`. A second free of the same block does nothing. */
export function free(ptr: number): void {
  const st = state;
  if (st === undefined) {
    return;
  }
  const n = st.live.get(ptr);
  if (n === undefined) {
    return;
  }
  st.live.delete(ptr);
  let at = ptr;
  let len = n;
  // Join the free block that ends at `at` and the one that starts after this block.
  for (const [a, l] of st.free) {
    if (a + l === at) {
      st.free.delete(a);
      at = a;
      len += l;
      break;
    }
  }
  const next = st.free.get(at + len);
  if (next !== undefined) {
    st.free.delete(at + len);
    len += next;
  }
  if (at + len === st.top) {
    st.top = at;
  } else {
    st.free.set(at, len);
  }
}

/** Runs `body` with blocks of the given sizes and frees them after. */
export function scoped<T>(sizes: number[], body: (...ptrs: number[]) => T): T {
  const ptrs = sizes.map(alloc);
  try {
    return body(...ptrs);
  } finally {
    for (const p of ptrs) {
      free(p);
    }
  }
}

/** A view of the memory now (a grow of the memory makes old views empty). */
export function view(): DataView {
  return new DataView(s().memory.buffer);
}

/** A copy of `len` bytes at `ptr`. */
export function readBytes(ptr: number, len: number): Uint8Array {
  return new Uint8Array(s().memory.buffer, ptr, len).slice();
}

/** Copies `data` to `ptr`. */
export function writeBytes(ptr: number, data: Uint8Array): void {
  new Uint8Array(s().memory.buffer, ptr, data.length).set(data);
}

/** Copies `data` into a new block; the caller frees it. */
export function allocBytes(data: Uint8Array): number {
  const p = alloc(data.length);
  writeBytes(p, data);
  return p;
}

/** `count` u32 words at `ptr`. */
export function readWords(ptr: number, count: number): number[] {
  const v = view();
  const out: number[] = [];
  for (let i = 0; i < count; i++) {
    out.push(v.getUint32(ptr + 4 * i, true));
  }
  return out;
}

/** Writes u32 words to `ptr`. */
export function writeWords(ptr: number, words: readonly number[]): void {
  const v = view();
  words.forEach((w, i) => v.setUint32(ptr + 4 * i, w >>> 0, true));
}

/** The usize at `ptr`. */
export function readUsize(ptr: number): number {
  return view().getUint32(ptr, true);
}

/** Text as NUL-terminated UTF-8 in a new block; the caller frees it. */
export function allocCString(text: string): number {
  const b = new TextEncoder().encode(text);
  const p = alloc(b.length + 1);
  writeBytes(p, b);
  return p;
}

// --- values ---

/** Checks that `value` is an integer from 0 to `max`; gives it. */
export function int(name: string, value: number, max: number): number {
  if (!Number.isInteger(value) || value < 0 || value > max) {
    throw new InvalidValueError(`${name} must be an integer from 0 to ${max}, not ${value}`);
  }
  return value;
}

/** Checks a time or a tick count (0 to 2^53 - 1); gives it as a bigint for a u64 argument. */
export function u64(name: string, value: number): bigint {
  return BigInt(int(name, value, Number.MAX_SAFE_INTEGER));
}

/** Checks the words of one UMP: 1 to 4 u32 values. */
export function umpWords(words: readonly number[]): readonly number[] {
  if (words.length < 1 || words.length > 4) {
    throw new InvalidValueError(`a UMP has 1 to 4 words, not ${words.length}`);
  }
  words.forEach((w, i) => int(`word ${i}`, w, 0xffffffff));
  return words;
}

/** Checks a protocol number: 1 (MIDI 1.0) or 2 (MIDI 2.0). */
export function protocol(p: number): number {
  if (p !== 1 && p !== 2) {
    throw new InvalidValueError(`the protocol is 1 (MIDI 1.0) or 2 (MIDI 2.0), not ${p}`);
  }
  return p;
}

/** Splits words into UMPs by the word count of each first word (M2-104 Table 4). */
export function splitUmps(words: readonly number[]): number[][] {
  const count = fn("zmidi_ump_word_count");
  const out: number[][] = [];
  let at = 0;
  while (at < words.length) {
    const n = Math.max(1, count(words[at]!) >>> 0);
    out.push(words.slice(at, at + n));
    at += n;
  }
  return out;
}

// --- status ---

/** The ZmidiStatus values. */
export const Status = {
  OK: 0,
  INVALID_ARGUMENT: 1,
  BUFFER_TOO_SMALL: 2,
  NO_PENDING: 3,
  REJECTED: 4,
  NOT_SUPPORTED: 5,
} as const;

/** Throws the error of a status that is not OK. `what` names the call. */
export function check(status: number, what: string): void {
  switch (status) {
    case Status.OK:
      return;
    case Status.INVALID_ARGUMENT:
      throw new InvalidValueError(`${what}: a value is out of its range`);
    case Status.NO_PENDING:
      throw new RejectedError(`${what}: no pending request`);
    case Status.REJECTED:
      throw new RejectedError(`${what}: refused in the current state`);
    case Status.NOT_SUPPORTED:
      throw new RejectedError(`${what}: not supported by this build`);
    default:
      // The package sizes every buffer, so BUFFER_TOO_SMALL is a defect of the package.
      throw new MidiError(`${what}: status ${status}`);
  }
}

// --- struct layouts (C rules, wasm32: pointer and usize 4 bytes, u64 aligned to 8) ---

/** A C field type: a scalar, or `[type, count]` for an array, or a nested layout. */
export type CType = "u8" | "bool" | "u16" | "u32" | "i32" | "u64" | "ptr" | "usize" | Layout | readonly [CType, number];

/** The offsets, size and alignment of one C struct. */
export interface Layout {
  readonly size: number;
  readonly align: number;
  readonly off: Readonly<Record<string, number>>;
}

function sizeAlign(t: CType): [number, number] {
  if (typeof t === "string") {
    const n = { u8: 1, bool: 1, u16: 2, u32: 4, i32: 4, u64: 8, ptr: 4, usize: 4 }[t];
    return [n, n];
  }
  if (Array.isArray(t)) {
    const [inner, count] = t as readonly [CType, number];
    const [n, a] = sizeAlign(inner);
    return [n * count, a];
  }
  const l = t as Layout;
  return [l.size, l.align];
}

/** The C layout of the fields, in order. */
export function layout(fields: readonly (readonly [string, CType])[]): Layout {
  const off: Record<string, number> = {};
  let at = 0;
  let align = 1;
  for (const [name, t] of fields) {
    const [n, a] = sizeAlign(t);
    at = Math.ceil(at / a) * a;
    off[name] = at;
    at += n;
    align = Math.max(align, a);
  }
  return { size: Math.ceil(at / align) * align, align, off };
}

// --- ownership ---

const registry = new FinalizationRegistry<number[]>((ptrs) => ptrs.forEach(free));

/**
 * The WASM blocks of one object of the package. The object frees them with `release`, or the
 * garbage collector frees them when the object is gone.
 */
export class Blocks {
  #ptrs: number[] = [];
  readonly #owner: object;

  constructor(owner: object) {
    this.#owner = owner;
    registry.register(owner, this.#ptrs, this);
  }

  /** Adds a block that `alloc` gave; gives it. */
  add(ptr: number): number {
    this.#ptrs.push(ptr);
    return ptr;
  }

  /** Frees a block that `add` took, now. */
  drop(ptr: number): void {
    const i = this.#ptrs.indexOf(ptr);
    if (i > 0) {
      this.#ptrs.splice(i, 1);
      free(ptr);
    }
  }

  /** Gives the first block; throws when the object is freed. */
  get main(): number {
    const p = this.#ptrs[0];
    if (p === undefined) {
      throw new RejectedError(`the ${this.#owner.constructor.name} is freed`);
    }
    return p;
  }

  /**
   * A block of at least `size` bytes that this object keeps for the arguments of its calls, so
   * that a call on a hot path allocates no WASM memory. A larger size replaces the block. The
   * content is not kept from call to call.
   */
  scratch(size: number): number {
    // A freed object has no main block: a new block must not take its place.
    void this.main;
    if (size > this.#scratchSize || this.#scratch === 0) {
      if (this.#scratch !== 0) {
        this.drop(this.#scratch);
      }
      this.#scratchSize = Math.max(64, Math.ceil(size / 16) * 16);
      this.#scratch = this.add(alloc(this.#scratchSize));
    }
    return this.#scratch;
  }

  /** The parts of [`scratch`] for blocks of `sizes` bytes, each aligned to 16. */
  carve(sizes: readonly number[]): number[] {
    const offsets: number[] = [];
    let at = 0;
    for (const n of sizes) {
      offsets.push(at);
      at += Math.ceil(Math.max(1, n) / ALIGN) * ALIGN;
    }
    const base = this.scratch(at);
    return offsets.map((o) => base + o);
  }

  #scratch = 0;
  #scratchSize = 0;

  /** Frees every block now. */
  release(): void {
    registry.unregister(this);
    this.#ptrs.forEach(free);
    this.#ptrs.length = 0;
    this.#scratch = 0;
    this.#scratchSize = 0;
  }
}
