//! A streaming zlib (RFC 1950) and deflate (RFC 1951) decoder with no alloc and about 1 KB of
//! state (OPT-010, DEC-PE-004).
//!
//! - The Huffman decoding follows `puff.c` of Mark Adler (the zlib reference decoder): canonical
//!   codes with a count and a symbol table for each code, no lookup tables. A code set that is
//!   over-subscribed is an error; an incomplete literal/length or distance code set is an error
//!   unless it has one code of length 1 (RFC 1951 3.2.7, as puff and zlib accept); the code
//!   length code set must be complete; a literal/length code set must have a code for 256.
//! - Streaming: the input comes in parts. Each step (a header field, a symbol and its extra
//!   bits, a length, a distance, a stored byte run) checks that its bits are there before it
//!   consumes any, so a step that needs more input stops with no change and goes on at the
//!   next call. The bits are held in a 64-bit buffer.
//! - The output goes to the caller and to the window (a power of two, at most 32 KiB), from
//!   which the matches are copied. A distance beyond the output so far or beyond the window is an
//!   error. The Adler-32 of the output is checked against the stream trailer.

/// The base lengths of length symbols 257 to 285 (RFC 1951 3.2.5).
const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// The extra bits of length symbols 257 to 285.
const LEXT: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// The base distances of distance symbols 0 to 29.
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
/// The extra bits of distance symbols 0 to 29.
const DEXT: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order of the code length code lengths (RFC 1951 3.2.7).
const ORDER: [u8; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];
const MAX_BITS: usize = 15;
const MAX_LCODES: usize = 286;
const MAX_DCODES: usize = 30;
const FIX_LCODES: usize = 288;

/// A decode error: the data is not a valid zlib stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Invalid;

/// What one call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Step {
    /// Input bytes taken (into the bit buffer or as stored data).
    pub consumed: usize,
    /// Output bytes written to `out` (and to the window).
    pub written: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Header,
    Block,
    StoredLen,
    Stored { left: u16 },
    DynCounts,
    DynCodeLens { i: u8 },
    DynLens { i: u16 },
    Codes,
    Dist { len: u16 },
    Copy { len: u16, dist: u16 },
    Trailer,
    Done,
}

/// A canonical Huffman code: the number of codes of each length and the symbols in code order.
#[derive(Debug, Clone, Copy)]
struct Huffman<const N: usize> {
    count: [u16; MAX_BITS + 1],
    symbol: [u16; N],
}

impl<const N: usize> Huffman<N> {
    const EMPTY: Self = Huffman {
        count: [0; MAX_BITS + 1],
        symbol: [0; N],
    };

    /// Builds the code of `lengths` (puff `construct`). Gives the number of unused codes: 0 for
    /// a complete code, below 0 for an over-subscribed one, above 0 for an incomplete one.
    fn build(&mut self, lengths: &[u8]) -> i32 {
        self.count = [0; MAX_BITS + 1];
        for &l in lengths {
            self.count[usize::from(l)] += 1;
        }
        if usize::from(self.count[0]) == lengths.len() {
            return 0; // no codes: complete, but decoding gives an error
        }
        let mut left: i32 = 1;
        for len in 1..=MAX_BITS {
            left <<= 1;
            left -= i32::from(self.count[len]);
            if left < 0 {
                return left;
            }
        }
        let mut offs = [0u16; MAX_BITS + 1];
        for len in 1..MAX_BITS {
            offs[len + 1] = offs[len] + self.count[len];
        }
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let o = &mut offs[usize::from(l)];
                self.symbol[usize::from(*o)] = s as u16;
                *o += 1;
            }
        }
        left
    }

    /// Decodes one symbol from the low `avail` bits of `bits` (first bit = bit 0). Gives the
    /// symbol and its code length; `Ok(None)` when the bits end before a code ends;
    /// `Err` for a bit pattern that is no code.
    fn decode(&self, bits: u64, avail: u32) -> Result<Option<(u16, u32)>, Invalid> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=MAX_BITS as u32 {
            if len > avail {
                return Ok(None);
            }
            code |= ((bits >> (len - 1)) & 1) as i32;
            let count = i32::from(self.count[len as usize]);
            if code - count < first {
                let i = (index + (code - first)) as usize;
                return Ok(Some((self.symbol[i], len)));
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Invalid)
    }
}

/// The decoder state of one zlib stream.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inflate {
    state: State,
    bits: u64,
    nbits: u32,
    last: bool,
    // Dynamic block header: the counts, the code length code, the lengths.
    nlen: u16,
    ndist: u8,
    ncode: u8,
    lengths: [u8; MAX_LCODES + MAX_DCODES],
    lencode: Huffman<FIX_LCODES>,
    distcode: Huffman<MAX_DCODES>,
    // Output so far (saturating) and its Adler-32 (RFC 1950 8.2).
    total: usize,
    a: u32,
    b: u32,
}

impl Inflate {
    pub(crate) const fn new() -> Self {
        Inflate {
            state: State::Header,
            bits: 0,
            nbits: 0,
            last: false,
            nlen: 0,
            ndist: 0,
            ncode: 0,
            lengths: [0; MAX_LCODES + MAX_DCODES],
            lencode: Huffman::EMPTY,
            distcode: Huffman::EMPTY,
            total: 0,
            a: 1,
            b: 0,
        }
    }

    pub(crate) fn is_done(&self) -> bool {
        self.state == State::Done
    }

    fn fill(&mut self, input: &[u8], used: &mut usize) {
        while self.nbits <= 56 {
            let Some(&byte) = input.get(*used) else { break };
            self.bits |= u64::from(byte) << self.nbits;
            self.nbits += 8;
            *used += 1;
        }
    }

    fn peek(&self, n: u32) -> Option<u32> {
        (self.nbits >= n).then(|| (self.bits & ((1u64 << n) - 1)) as u32)
    }

    fn drop_bits(&mut self, n: u32) {
        self.bits >>= n;
        self.nbits -= n;
    }

    fn emit(&mut self, byte: u8, window: &mut [u8], out: &mut [u8], written: &mut usize) {
        let mask = window.len() - 1;
        window[self.total & mask] = byte;
        out[*written] = byte;
        *written += 1;
        self.total = self.total.saturating_add(1);
        self.a = (self.a + u32::from(byte)) % 65521;
        self.b = (self.b + self.a) % 65521;
    }

    /// Decodes `input` into `out`, keeping the last output in `window` (a power of two, at most
    /// 32 KiB, at least the size that the header declares; the caller checks it). It stops when
    /// `out` is full, when it needs more input, or at the end of the stream.
    pub(crate) fn inflate(
        &mut self,
        input: &[u8],
        window: &mut [u8],
        out: &mut [u8],
    ) -> Result<Step, Invalid> {
        let mut used = 0;
        let mut written = 0;
        if window.is_empty() {
            return Err(Invalid);
        }
        loop {
            self.fill(input, &mut used);
            let progress = self.step(window, out, &mut written, input, &mut used)?;
            if !progress {
                return Ok(Step {
                    consumed: used,
                    written,
                });
            }
        }
    }

    /// One step; false when it can not go on now (more input or output space needed, or done).
    fn step(
        &mut self,
        window: &mut [u8],
        out: &mut [u8],
        written: &mut usize,
        input: &[u8],
        used: &mut usize,
    ) -> Result<bool, Invalid> {
        match self.state {
            State::Header => {
                let Some(h) = self.peek(16) else {
                    return Ok(false);
                };
                let (cmf, flg) = (h & 0xFF, h >> 8);
                // RFC 1950 2.2: CM 8, CINFO 7 or less, FCHECK, no preset dictionary.
                if cmf & 0x0F != 8 || cmf >> 4 > 7 || (cmf * 256 + flg) % 31 != 0 || flg & 0x20 != 0
                {
                    return Err(Invalid);
                }
                if (1usize << ((cmf >> 4) + 8)) > window.len() {
                    return Err(Invalid);
                }
                self.drop_bits(16);
                self.state = State::Block;
            }
            State::Block => {
                if self.last {
                    // Byte boundary, then the Adler-32 trailer (RFC 1950 2.2).
                    self.drop_bits(self.nbits % 8);
                    self.state = State::Trailer;
                    return Ok(true);
                }
                let Some(h) = self.peek(3) else {
                    return Ok(false);
                };
                self.drop_bits(3);
                self.last = h & 1 == 1;
                self.state = match h >> 1 {
                    0 => {
                        self.drop_bits(self.nbits % 8);
                        State::StoredLen
                    }
                    1 => {
                        self.fixed();
                        State::Codes
                    }
                    2 => State::DynCounts,
                    _ => return Err(Invalid),
                };
            }
            State::StoredLen => {
                let Some(v) = self.peek(32) else {
                    return Ok(false);
                };
                let (len, nlen) = (v & 0xFFFF, v >> 16);
                if len != !nlen & 0xFFFF {
                    return Err(Invalid);
                }
                self.drop_bits(32);
                self.state = State::Stored { left: len as u16 };
            }
            State::Stored { left } => {
                if left == 0 {
                    self.state = State::Block;
                    return Ok(true);
                }
                if *written == out.len() {
                    return Ok(false);
                }
                let byte = if let Some(b) = self.peek(8) {
                    self.drop_bits(8);
                    b as u8
                } else if let Some(&b) = input.get(*used) {
                    *used += 1;
                    b
                } else {
                    return Ok(false);
                };
                self.emit(byte, window, out, written);
                self.state = State::Stored { left: left - 1 };
            }
            State::DynCounts => {
                let Some(h) = self.peek(14) else {
                    return Ok(false);
                };
                self.drop_bits(14);
                self.nlen = (h & 0x1F) as u16 + 257;
                self.ndist = ((h >> 5) & 0x1F) as u8 + 1;
                self.ncode = ((h >> 10) & 0x0F) as u8 + 4;
                if usize::from(self.nlen) > MAX_LCODES || usize::from(self.ndist) > MAX_DCODES {
                    return Err(Invalid);
                }
                self.lengths = [0; MAX_LCODES + MAX_DCODES];
                self.state = State::DynCodeLens { i: 0 };
            }
            State::DynCodeLens { i } => {
                if i == self.ncode {
                    // The code length code (RFC 1951 3.2.7): its lengths are in `lengths` in the
                    // ORDER sequence, and `lencode` holds it until the real tables are built.
                    let mut cl = [0u8; 19];
                    for (k, &o) in ORDER.iter().enumerate() {
                        cl[usize::from(o)] = self.lengths[k];
                    }
                    if self.lencode.build(&cl) != 0 {
                        return Err(Invalid); // must be complete
                    }
                    self.lengths = [0; MAX_LCODES + MAX_DCODES];
                    self.state = State::DynLens { i: 0 };
                    return Ok(true);
                }
                let Some(v) = self.peek(3) else {
                    return Ok(false);
                };
                self.drop_bits(3);
                self.lengths[usize::from(i)] = v as u8;
                self.state = State::DynCodeLens { i: i + 1 };
            }
            State::DynLens { i } => {
                let total = self.nlen + u16::from(self.ndist);
                if i == total {
                    return self.dyn_tables().map(|()| true);
                }
                let Some((sym, l)) = self.lencode.decode(self.bits, self.nbits)? else {
                    return Ok(false);
                };
                let (extra, base, value) = match sym {
                    0..=15 => (0, sym as u8, None),
                    16 => (2, 3, Some(i.checked_sub(1).ok_or(Invalid)?)),
                    17 => (3, 3, None),
                    _ => (7, 11, None),
                };
                if self.nbits < l + extra {
                    return Ok(false);
                }
                let rep = ((self.bits >> l) & ((1u64 << extra) - 1)) as u16;
                self.drop_bits(l + extra);
                if sym < 16 {
                    self.lengths[usize::from(i)] = base;
                    self.state = State::DynLens { i: i + 1 };
                } else {
                    let len = value.map_or(0, |p| self.lengths[usize::from(p)]);
                    let n = u16::from(base) + rep;
                    if i + n > total {
                        return Err(Invalid);
                    }
                    for k in i..i + n {
                        self.lengths[usize::from(k)] = len;
                    }
                    self.state = State::DynLens { i: i + n };
                }
            }
            State::Codes => {
                let Some((sym, l)) = self.lencode.decode(self.bits, self.nbits)? else {
                    return Ok(false);
                };
                if sym < 256 {
                    if *written == out.len() {
                        return Ok(false);
                    }
                    self.drop_bits(l);
                    self.emit(sym as u8, window, out, written);
                } else if sym == 256 {
                    self.drop_bits(l);
                    self.state = State::Block;
                } else {
                    let s = usize::from(sym - 257);
                    if s >= LBASE.len() {
                        return Err(Invalid);
                    }
                    let extra = u32::from(LEXT[s]);
                    if self.nbits < l + extra {
                        return Ok(false);
                    }
                    let e = ((self.bits >> l) & ((1u64 << extra) - 1)) as u16;
                    self.drop_bits(l + extra);
                    self.state = State::Dist { len: LBASE[s] + e };
                }
            }
            State::Dist { len } => {
                let Some((sym, l)) = self.distcode.decode(self.bits, self.nbits)? else {
                    return Ok(false);
                };
                let s = usize::from(sym);
                if s >= DBASE.len() {
                    return Err(Invalid);
                }
                let extra = u32::from(DEXT[s]);
                if self.nbits < l + extra {
                    return Ok(false);
                }
                let e = ((self.bits >> l) & ((1u64 << extra) - 1)) as u16;
                let dist = DBASE[s] + e;
                if usize::from(dist) > self.total || usize::from(dist) > window.len() {
                    return Err(Invalid);
                }
                self.drop_bits(l + extra);
                self.state = State::Copy { len, dist };
            }
            State::Copy { len, dist } => {
                if len == 0 {
                    self.state = State::Codes;
                    return Ok(true);
                }
                if *written == out.len() {
                    return Ok(false);
                }
                let mask = window.len() - 1;
                let byte = window[(self.total - usize::from(dist)) & mask];
                self.emit(byte, window, out, written);
                self.state = State::Copy { len: len - 1, dist };
            }
            State::Trailer => {
                let Some(v) = self.peek(32) else {
                    return Ok(false);
                };
                self.drop_bits(32);
                let want = u32::from_be_bytes(v.to_le_bytes());
                if want != (self.b << 16) | self.a {
                    return Err(Invalid);
                }
                self.state = State::Done;
            }
            State::Done => return Ok(false),
        }
        Ok(true)
    }

    /// The fixed codes of RFC 1951 3.2.6.
    fn fixed(&mut self) {
        let mut l = [0u8; FIX_LCODES];
        for (s, v) in l.iter_mut().enumerate() {
            *v = match s {
                0..=143 => 8,
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
        }
        self.lencode.build(&l);
        self.distcode.build(&[5; MAX_DCODES]);
    }

    /// The literal/length and distance codes of a dynamic block (puff `dynamic`).
    fn dyn_tables(&mut self) -> Result<(), Invalid> {
        let nlen = usize::from(self.nlen);
        let lengths = self.lengths;
        if lengths[256] == 0 {
            return Err(Invalid); // no end-of-block code
        }
        let err = self.lencode.build(&lengths[..nlen]);
        let single = |h: &[u16; MAX_BITS + 1], n: usize| usize::from(h[0]) + usize::from(h[1]) == n;
        if err < 0 || (err > 0 && !single(&self.lencode.count, nlen)) {
            return Err(Invalid);
        }
        let nd = usize::from(self.ndist);
        let err = self.distcode.build(&lengths[nlen..nlen + nd]);
        if err < 0 || (err > 0 && !single(&self.distcode.count, nd)) {
            return Err(Invalid);
        }
        self.state = State::Codes;
        Ok(())
    }
}
