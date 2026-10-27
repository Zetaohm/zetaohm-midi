// Loading of the WASM build (REQ-TS-PKG-001): DEC-019.
import { readFile } from "node:fs/promises";

import { describe, expect, test } from "vitest";

import { MidiError, abiVersion, init, initSync, wasmUrl, wordCount } from "./index.js";

/**
 * A WASM module with one page of memory and a `zmidi_abi_version` that gives the value `leb` (a
 * signed LEB128 i32.const of at most 3 bytes).
 */
function fakeAbi(leb: readonly number[]): Uint8Array {
  const name = [...new TextEncoder().encode("zmidi_abi_version")];
  const body = [0x00, 0x41, ...leb, 0x0b];
  return new Uint8Array([
    ...[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
    ...[0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f], // type: () -> i32
    ...[0x03, 0x02, 0x01, 0x00], // function 0
    ...[0x05, 0x03, 0x01, 0x00, 0x01], // memory: 1 page
    ...[0x07, 1 + 9 + 1 + name.length + 2, 0x02, 0x06, ...new TextEncoder().encode("memory"), 0x02, 0x00, name.length, ...name, 0x00, 0x00],
    ...[0x0a, body.length + 2, 0x01, body.length, ...body],
  ]);
}

describe("init", () => {
  // Before the other tests: a refused WASM file leaves the package not set up.
  test("refuses a WASM file of C ABI 0.1 to 0.6 or 1.x", () => {
    expect(() => initSync(fakeAbi([0x01]))).toThrow(MidiError); // 0.1: below ABI_MINOR 7
    expect(() => initSync(fakeAbi([0x02]))).toThrow(MidiError); // 0.2: no Property Exchange engine
    expect(() => initSync(fakeAbi([0x03]))).toThrow(MidiError); // 0.3: no zmidi_ci_pe_end
    expect(() => initSync(fakeAbi([0x04]))).toThrow(MidiError); // 0.4: no USB descriptor writer
    expect(() => initSync(fakeAbi([0x06]))).toThrow(MidiError); // 0.6: no zmidi_bytes_to_ump_feed
    expect(() => initSync(fakeAbi([0x82, 0x80, 0x04]))).toThrow(MidiError); // 1.2: another major
    expect(() => abiVersion()).toThrow(MidiError);
  });

  test("loads the WASM file once", async () => {
    const v = await init();
    expect(v >>> 16).toBe(0);
    expect(v & 0xffff).toBeGreaterThanOrEqual(7);
    expect(await init()).toBe(v);
    expect(abiVersion()).toBe(v);
    expect(wordCount(0x40000000)).toBe(2);
  });

  test("initSync takes bytes", async () => {
    const bytes = await readFile(wasmUrl);
    expect(initSync(bytes) >>> 16).toBe(0);
    expect(new WebAssembly.Module(bytes)).toBeInstanceOf(WebAssembly.Module);
  });
});
