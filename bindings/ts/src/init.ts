// Loads the WASM build of the C ABI (wasm/zmidi.wasm): from the file beside the package in Node,
// with fetch in a browser, or from bytes or a compiled module that the program gives.

import { attach, attached, fn } from "./wasm.js";

/** What `init` can load: bytes of the WASM file, a compiled module, a URL or a fetch response. */
export type WasmSource = BufferSource | WebAssembly.Module | URL | string | Response | Promise<Response>;

/** The location of the WASM file of the package. */
export const wasmUrl: URL = new URL("../wasm/zmidi.wasm", import.meta.url);

async function bytesOf(url: URL): Promise<BufferSource> {
  if (url.protocol === "file:") {
    const fs = await import("node:fs/promises");
    return fs.readFile(url);
  }
  const r = await fetch(url);
  return r.arrayBuffer();
}

/**
 * Loads the WASM build and sets up the package. Call it once before the other functions; a second
 * call does nothing. Gives the C ABI version (major << 16 | minor).
 */
export async function init(source?: WasmSource): Promise<number> {
  if (attached()) {
    return abiVersion();
  }
  let src: WasmSource = source ?? wasmUrl;
  if (typeof src === "string") {
    src = new URL(src, import.meta.url);
  }
  if (src instanceof URL) {
    src = await bytesOf(src);
  }
  src = await src;
  let instance: WebAssembly.Instance;
  if (src instanceof WebAssembly.Module) {
    instance = await WebAssembly.instantiate(src, {});
  } else if (typeof Response !== "undefined" && src instanceof Response) {
    instance = (await WebAssembly.instantiate(await src.arrayBuffer(), {})).instance;
  } else {
    instance = (await WebAssembly.instantiate(src as BufferSource, {})).instance;
  }
  return attach(instance);
}

/** The synchronous form of `init`, for bytes or a compiled module. Gives the C ABI version. */
export function initSync(source: BufferSource | WebAssembly.Module): number {
  if (attached()) {
    return abiVersion();
  }
  const module = source instanceof WebAssembly.Module ? source : new WebAssembly.Module(source);
  return attach(new WebAssembly.Instance(module, {}));
}

/** The C ABI version of the loaded WASM build (major << 16 | minor). */
export function abiVersion(): number {
  return fn("zmidi_abi_version")() >>> 0;
}
