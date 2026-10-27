//! The JSON layer (M2-103-UM v1.2 sections 6.1 and 7.1; ECMA-404).
//!
//! - [`JsonReader`] is an incremental tokenizer: it takes the bytes in pieces (one Chunk at a
//!   time) and gives each complete token. A string is decoded into a fixed buffer of `S` bytes;
//!   a longer string is cut at a character boundary and marked as truncated. Nesting is limited
//!   to 64 levels with a bit stack (DEC-002: no alloc). A value can be captured as raw JSON text
//!   ("schema", "propData": DEC-007).
//! - [`JsonWriter`] writes into a caller buffer with the M2-103 rules: names in camelCase with
//!   a-z, A-Z, 0-9 only and not only digits (REQ-PE-JSON-002, -003), decimal numbers
//!   (REQ-PE-JSON-005), ECMA-404 escapes and `\u` lower-case hex for each non-ASCII UTF-16 code
//!   unit (REQ-PE-JSON-006, -007), no whitespace (REQ-PE-HDR-005).

/// Maximum nesting depth of the reader.
pub const MAX_DEPTH: u8 = 64;

/// Error of the JSON reader and writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JsonError {
    /// The bytes are not valid JSON.
    #[error("JSON syntax error")]
    Syntax,
    /// Nesting deeper than 64 levels.
    #[error("JSON nesting too deep")]
    Depth,
    /// The caller buffer is full.
    #[error("buffer overflow")]
    Overflow,
    /// A name that breaks REQ-PE-JSON-002 or -003, or is longer than its limit.
    #[error("invalid property name")]
    InvalidName,
    /// A value that the writer cannot write, or a number out of range.
    #[error("invalid value")]
    InvalidValue,
}

/// The text of a JSON number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Number<'a> {
    text: &'a str,
}

impl<'a> Number<'a> {
    /// The number as received.
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The number as an integer with no fraction, exponent or sign.
    pub fn as_u64(&self) -> Result<u64, JsonError> {
        let t = self.text;
        if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
            return Err(JsonError::InvalidValue);
        }
        t.parse::<u64>().map_err(|_| JsonError::InvalidValue)
    }

    /// The number as a `u32` integer (REQ-PE-HDR-011, -013).
    pub fn as_u32(&self) -> Result<u32, JsonError> {
        u32::try_from(self.as_u64()?).map_err(|_| JsonError::InvalidValue)
    }

    /// The number as an integer with an optional minus sign.
    pub fn as_i64(&self) -> Result<i64, JsonError> {
        let digits = self.text.strip_prefix('-').unwrap_or(self.text);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(JsonError::InvalidValue);
        }
        self.text
            .parse::<i64>()
            .map_err(|_| JsonError::InvalidValue)
    }
}

/// A Number over checked number text.
pub(crate) fn number(text: &str) -> Number<'_> {
    Number { text }
}

/// Reads number text as a `u32` with no fraction, exponent or sign.
pub(crate) fn number_u32(text: &str) -> Result<u32, JsonError> {
    Number { text }.as_u32()
}

/// Reads one flat JSON object from `bytes` (one piece) and gives each member to `f`: the name
/// (`None` when it is longer than 24 bytes) and the raw JSON text of its value, as a part of
/// `bytes`. The Header and Link readers use it.
pub(crate) fn for_each_member<'a, E: From<JsonError>>(
    bytes: &'a [u8],
    mut f: impl FnMut(Option<&str>, &'a [u8]) -> Result<(), E>,
) -> Result<(), E> {
    let mut reader = JsonReader::<24>::new();
    let mut name = [0u8; 24];
    let mut name_len: Option<usize> = None;
    {
        let mut tokens = reader.push(bytes);
        let mut first = true;
        loop {
            let mut capture = false;
            let mut range = None;
            match tokens.next_token() {
                None => break,
                Some(t) => match t? {
                    Token::BeginObject if first => {}
                    _ if first => return Err(JsonError::Syntax.into()),
                    Token::Name { text, truncated } => {
                        name_len = (!truncated && text.len() <= name.len()).then(|| {
                            name[..text.len()].copy_from_slice(text.as_bytes());
                            text.len()
                        });
                        capture = true;
                    }
                    Token::Raw(raw) => {
                        // The Raw slice is a part of `bytes`: take its range there.
                        let start = raw.as_ptr() as usize - bytes.as_ptr() as usize;
                        range = Some(start..start + raw.len());
                    }
                    _ => {}
                },
            }
            first = false;
            if capture {
                tokens.capture_value();
            }
            if let Some(r) = range {
                let n = name_len
                    .take()
                    .and_then(|n| core::str::from_utf8(&name[..n]).ok());
                f(n, &bytes[r])?;
            }
        }
    }
    if reader.finish()?.is_some() {
        return Err(JsonError::Syntax.into());
    }
    Ok(())
}

/// The members of a JSON object that `bytes` holds whole (Header Data, REQ-PE-HDR-014): `f` gets
/// each top-level name and the raw bytes of its value, in order, as [`for_each_member`] gives
/// them, with no streaming reader (OPT-020). The object must be valid JSON (RFC 8259): a syntax
/// error or a nesting deeper than [`MAX_DEPTH`] stops the walk with that error. A name is decoded
/// (escapes included) into 24 bytes; a longer name, or one with a character that is not ASCII
/// (also an invalid UTF-8 byte), is given as `None`, so it is never a known header property.
pub(crate) fn for_each_member_slice<'a, E: From<JsonError>>(
    bytes: &'a [u8],
    mut f: impl FnMut(Option<&[u8]>, &'a [u8]) -> Result<(), E>,
) -> Result<(), E> {
    let mut s = Scan { b: bytes, i: 0 };
    s.ws();
    if s.next() != Some(b'{') {
        return Err(JsonError::Syntax.into());
    }
    s.ws();
    if s.peek() == Some(b'}') {
        s.i += 1;
    } else {
        loop {
            let mut name = [0u8; 24];
            let n = s.name(&mut name)?;
            s.ws();
            if s.next() != Some(b':') {
                return Err(JsonError::Syntax.into());
            }
            s.ws();
            let start = s.i;
            s.value(1)?;
            let raw = &bytes[start..s.i];
            f(n.map(|n| &name[..n]), raw)?;
            s.ws();
            match s.next() {
                Some(b',') => s.ws(),
                Some(b'}') => break,
                _ => return Err(JsonError::Syntax.into()),
            }
        }
    }
    s.ws();
    if s.i != bytes.len() {
        return Err(JsonError::Syntax.into());
    }
    Ok(())
}

/// Gives each member of a flat Header Data object to `f`, as [`for_each_member_slice`] does, for
/// the Header Data rules of M2-103 7.1.1 only: no whitespace, and each value a string, a number,
/// `true`, `false` or `null`. A string is checked as JSON; a number or a literal is taken up to
/// the next `,` or `}` with no check. An object or an array as a value, or any whitespace, gives
/// [`JsonError::Syntax`]. The one-chunk Responder of `one_chunk::respond_flat` uses it.
pub(crate) fn for_each_flat_member<'a, E: From<JsonError>>(
    bytes: &'a [u8],
    mut f: impl FnMut(Option<&[u8]>, &'a [u8]) -> Result<(), E>,
) -> Result<(), E> {
    let mut s = Scan { b: bytes, i: 0 };
    if s.next() != Some(b'{') {
        return Err(JsonError::Syntax.into());
    }
    if s.peek() == Some(b'}') {
        s.i += 1;
    } else {
        loop {
            let mut name = [0u8; 24];
            let n = s.name(&mut name)?;
            if s.next() != Some(b':') {
                return Err(JsonError::Syntax.into());
            }
            let start = s.i;
            match s.peek() {
                Some(b'"') => drop(s.string(&mut [])?),
                _ => {
                    while let Some(c) = s.peek() {
                        match c {
                            b',' | b'}' => break,
                            b'{' | b'[' | b'"' | b' ' | b'\t' | b'\n' | b'\r' => {
                                return Err(JsonError::Syntax.into())
                            }
                            _ => s.i += 1,
                        }
                    }
                    if s.i == start {
                        return Err(JsonError::Syntax.into());
                    }
                }
            }
            f(n.map(|n| &name[..n]), &bytes[start..s.i])?;
            match s.next() {
                Some(b',') => {}
                Some(b'}') => break,
                _ => return Err(JsonError::Syntax.into()),
            }
        }
    }
    if s.i != bytes.len() {
        return Err(JsonError::Syntax.into());
    }
    Ok(())
}

pub(crate) struct Scan<'a> {
    b: &'a [u8],
    i: usize,
}

impl Scan<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }
    #[inline(never)]
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn hex4(&mut self) -> Result<u16, JsonError> {
        let mut v = 0u16;
        for _ in 0..4 {
            let d = match self.next() {
                Some(c @ b'0'..=b'9') => c - b'0',
                Some(c @ b'a'..=b'f') => c - b'a' + 10,
                Some(c @ b'A'..=b'F') => c - b'A' + 10,
                _ => return Err(JsonError::Syntax),
            };
            v = v << 4 | u16::from(d);
        }
        Ok(v)
    }
    /// A string, decoded into `out`: the length when every character is ASCII and fits, else
    /// `None` (the string is still read to its end). One body for every caller, so the decoder
    /// is linked once (OPT-051).
    #[inline(never)]
    fn string(&mut self, out: &mut [u8]) -> Result<Option<usize>, JsonError> {
        if self.next() != Some(b'"') {
            return Err(JsonError::Syntax);
        }
        let (mut n, mut ok) = (0usize, true);
        loop {
            let c = match self.next().ok_or(JsonError::Syntax)? {
                b'"' => return Ok(ok.then_some(n)),
                b'\\' => match self.next().ok_or(JsonError::Syntax)? {
                    b'"' => b'"',
                    b'\\' => b'\\',
                    b'/' => b'/',
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'u' => u8::try_from(self.hex4()?).unwrap_or(0x80),
                    _ => return Err(JsonError::Syntax),
                },
                c if c < 0x20 => return Err(JsonError::Syntax),
                c => c,
            };
            match out.get_mut(n) {
                Some(slot) if c.is_ascii() => {
                    *slot = c;
                    n += 1;
                }
                _ => ok = false,
            }
        }
    }
    /// Decodes the string token at the start of `raw` (escapes included) into `out`; `None`
    /// for a value that is not a string, or one longer than `out` or with a character that is
    /// not ASCII (OPT-019).
    pub(crate) fn decode_into(raw: &[u8], out: &mut [u8]) -> Option<usize> {
        Scan { b: raw, i: 0 }.string(out).ok().flatten()
    }

    fn name(&mut self, out: &mut [u8; 24]) -> Result<Option<usize>, JsonError> {
        self.string(out)
    }
    fn digits(&mut self) -> usize {
        let s = self.i;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - s
    }
    fn value(&mut self, depth: u8) -> Result<(), JsonError> {
        match self.peek().ok_or(JsonError::Syntax)? {
            b'"' => self.string(&mut []).map(drop),
            b'{' | b'[' => {
                if depth >= MAX_DEPTH {
                    return Err(JsonError::Depth);
                }
                let close = if self.next() == Some(b'{') {
                    b'}'
                } else {
                    b']'
                };
                self.ws();
                if self.peek() == Some(close) {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    if close == b'}' {
                        self.string(&mut [])?;
                        self.ws();
                        if self.next() != Some(b':') {
                            return Err(JsonError::Syntax);
                        }
                        self.ws();
                    }
                    self.value(depth + 1)?;
                    self.ws();
                    match self.next() {
                        Some(b',') => self.ws(),
                        Some(c) if c == close => return Ok(()),
                        _ => return Err(JsonError::Syntax),
                    }
                }
            }
            b't' | b'f' | b'n' => {
                for lit in [&b"true"[..], b"false", b"null"] {
                    if self.b[self.i..].starts_with(lit) {
                        self.i += lit.len();
                        return Ok(());
                    }
                }
                Err(JsonError::Syntax)
            }
            _ => {
                // Number: -? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?
                if self.peek() == Some(b'-') {
                    self.i += 1;
                }
                match self.peek() {
                    Some(b'0') => self.i += 1,
                    Some(b'1'..=b'9') => {
                        self.digits();
                    }
                    _ => return Err(JsonError::Syntax),
                }
                if self.peek() == Some(b'.') {
                    self.i += 1;
                    if self.digits() == 0 {
                        return Err(JsonError::Syntax);
                    }
                }
                if matches!(self.peek(), Some(b'e' | b'E')) {
                    self.i += 1;
                    if matches!(self.peek(), Some(b'+' | b'-')) {
                        self.i += 1;
                    }
                    if self.digits() == 0 {
                        return Err(JsonError::Syntax);
                    }
                }
                Ok(())
            }
        }
    }
}

/// One JSON token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    /// `{`
    BeginObject,
    /// `}`
    EndObject,
    /// `[`
    BeginArray,
    /// `]`
    EndArray,
    /// An object name, decoded; `truncated` when longer than the string buffer.
    Name { text: &'a str, truncated: bool },
    /// A string value, decoded; `truncated` when longer than the string buffer.
    String { text: &'a str, truncated: bool },
    /// A number.
    Number(Number<'a>),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
    /// A piece of the unchanged JSON text of a captured value.
    Raw(&'a [u8]),
}

/// What the parser expects next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// A value (top level, after a colon, or after a comma in an array).
    Value,
    /// A value or `]` (after `[`).
    ValueOrEnd,
    /// A name or `}` (after `{`).
    NameOrEnd,
    /// A name (after a comma in an object).
    Name,
    /// `:` after a name.
    Colon,
    /// `,` or the end of the container.
    CommaOrEnd,
    /// Only whitespace: the top-level value is complete.
    Done,
}

/// Lexer state inside a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lex {
    Between,
    /// In a string; `name` when it is an object name.
    Str {
        name: bool,
        escape: Escape,
    },
    Number,
    /// In `true`, `false` or `null`: the literal and the next index.
    Literal(&'static [u8], u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escape {
    None,
    Backslash,
    /// `\u` and the hex digits read so far.
    Hex(u8, u16),
}

/// What one step gives.
enum Step {
    /// Nothing complete yet.
    None,
    /// A token is complete; for String/Name/Number the text is in the buffers.
    Token(Kind),
    /// A token is complete and the byte must be processed again (end of a number).
    TokenAgain(Kind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    BeginObject,
    EndObject,
    BeginArray,
    EndArray,
    Name,
    String,
    Number,
    Bool(bool),
    Null,
}

/// Incremental JSON tokenizer with a string buffer of `S` bytes.
#[derive(Debug, Clone)]
pub struct JsonReader<const S: usize> {
    expect: Expect,
    lex: Lex,
    /// Bit n set: level n+1 is an object.
    stack: u64,
    depth: u8,
    buf: [u8; S],
    len: usize,
    truncated: bool,
    /// High surrogate that waits for its low surrogate.
    high: Option<u16>,
    num: [u8; 32],
    num_len: usize,
    /// Capture: requested, and the depth at which the captured value started.
    capture_next: bool,
    capture: Option<u8>,
    failed: Option<JsonError>,
}

#[allow(clippy::new_without_default)]
impl<const S: usize> JsonReader<S> {
    /// A reader for one JSON value.
    pub const fn new() -> Self {
        JsonReader {
            expect: Expect::Value,
            lex: Lex::Between,
            stack: 0,
            depth: 0,
            buf: [0; S],
            len: 0,
            truncated: false,
            high: None,
            num: [0; 32],
            num_len: 0,
            capture_next: false,
            capture: None,
            failed: None,
        }
    }

    /// Current nesting depth.
    pub fn depth(&self) -> u8 {
        self.depth
    }

    /// The next value is given as `Raw` pieces of its unchanged text (DEC-007).
    pub fn capture_value(&mut self) {
        self.capture_next = true;
    }

    /// Takes the next piece of bytes and gives the tokens that are complete.
    pub fn push<'r, 'i>(&'r mut self, input: &'i [u8]) -> Tokens<'r, 'i, S> {
        Tokens {
            reader: self,
            input,
            pos: 0,
            raw_start: None,
        }
    }

    /// The end of the data. Gives the last number of a top-level number value, or an error
    /// when a value is not complete.
    pub fn finish(&mut self) -> Result<Option<Token<'_>>, JsonError> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        if self.lex == Lex::Number && self.depth == 0 {
            self.lex = Lex::Between;
            self.expect = Expect::Done;
            return Ok(Some(Token::Number(Number {
                text: self.num_text()?,
            })));
        }
        if self.expect == Expect::Done && self.lex == Lex::Between {
            Ok(None)
        } else {
            Err(JsonError::Syntax)
        }
    }

    fn num_text(&self) -> Result<&str, JsonError> {
        let t = core::str::from_utf8(&self.num[..self.num_len]).map_err(|_| JsonError::Syntax)?;
        // ECMA-404 number grammar.
        let b = t.as_bytes();
        let mut i = 0;
        if b.get(i) == Some(&b'-') {
            i += 1;
        }
        match b.get(i) {
            Some(b'0') => i += 1,
            Some(c) if c.is_ascii_digit() => {
                while b.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            _ => return Err(JsonError::Syntax),
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            let s = i;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == s {
                return Err(JsonError::Syntax);
            }
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(b.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let s = i;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == s {
                return Err(JsonError::Syntax);
            }
        }
        if i != b.len() {
            return Err(JsonError::Syntax);
        }
        Ok(t)
    }

    fn text(&self) -> &str {
        match core::str::from_utf8(&self.buf[..self.len]) {
            Ok(t) => t,
            Err(e) => core::str::from_utf8(&self.buf[..e.valid_up_to()]).unwrap_or(""),
        }
    }

    fn push_char(&mut self, c: char) {
        let mut tmp = [0u8; 4];
        let s = c.encode_utf8(&mut tmp).as_bytes();
        if self.truncated || self.len + s.len() > S {
            self.truncated = true;
            return;
        }
        self.buf[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
    }

    fn push_byte(&mut self, b: u8) {
        if self.truncated || self.len >= S {
            self.truncated = true;
            return;
        }
        self.buf[self.len] = b;
        self.len += 1;
    }

    fn flush_high(&mut self) {
        if self.high.take().is_some() {
            self.push_char(char::REPLACEMENT_CHARACTER);
        }
    }

    fn unicode(&mut self, v: u16) {
        match (self.high.take(), v) {
            (Some(h), 0xDC00..=0xDFFF) => {
                let c = 0x10000 + (((h as u32) - 0xD800) << 10) + (v as u32 - 0xDC00);
                self.push_char(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
            (h, _) => {
                if h.is_some() {
                    self.push_char(char::REPLACEMENT_CHARACTER);
                }
                match v {
                    0xD800..=0xDBFF => self.high = Some(v),
                    0xDC00..=0xDFFF => self.push_char(char::REPLACEMENT_CHARACTER),
                    _ => self
                        .push_char(char::from_u32(v as u32).unwrap_or(char::REPLACEMENT_CHARACTER)),
                }
            }
        }
    }

    /// A value is complete: what comes next.
    fn after_value(&mut self) {
        self.expect = if self.depth == 0 {
            Expect::Done
        } else {
            Expect::CommaOrEnd
        };
    }

    fn open(&mut self, object: bool) -> Result<(), JsonError> {
        if self.depth >= MAX_DEPTH {
            return Err(JsonError::Depth);
        }
        if object {
            self.stack |= 1 << self.depth;
        } else {
            self.stack &= !(1 << self.depth);
        }
        self.depth += 1;
        self.expect = if object {
            Expect::NameOrEnd
        } else {
            Expect::ValueOrEnd
        };
        Ok(())
    }

    fn in_object(&self) -> bool {
        self.depth > 0 && self.stack & (1 << (self.depth - 1)) != 0
    }

    fn close(&mut self, object: bool) -> Result<(), JsonError> {
        if self.depth == 0 || self.in_object() != object {
            return Err(JsonError::Syntax);
        }
        self.depth -= 1;
        self.after_value();
        Ok(())
    }

    fn value_allowed(&self) -> bool {
        matches!(self.expect, Expect::Value | Expect::ValueOrEnd)
    }

    fn step(&mut self, b: u8) -> Result<Step, JsonError> {
        match self.lex {
            Lex::Str { name, escape } => self.step_string(b, name, escape),
            Lex::Number => {
                if matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    if self.num_len >= self.num.len() {
                        return Err(JsonError::Syntax);
                    }
                    self.num[self.num_len] = b;
                    self.num_len += 1;
                    Ok(Step::None)
                } else {
                    self.num_text()?;
                    self.lex = Lex::Between;
                    self.after_value();
                    Ok(Step::TokenAgain(Kind::Number))
                }
            }
            Lex::Literal(lit, i) => {
                if lit.get(i as usize) != Some(&b) {
                    return Err(JsonError::Syntax);
                }
                if i as usize + 1 == lit.len() {
                    self.lex = Lex::Between;
                    self.after_value();
                    return Ok(Step::Token(match lit[0] {
                        b't' => Kind::Bool(true),
                        b'f' => Kind::Bool(false),
                        _ => Kind::Null,
                    }));
                }
                self.lex = Lex::Literal(lit, i + 1);
                Ok(Step::None)
            }
            Lex::Between => self.step_between(b),
        }
    }

    fn step_between(&mut self, b: u8) -> Result<Step, JsonError> {
        match b {
            b' ' | b'\t' | b'\n' | b'\r' => Ok(Step::None),
            b'{' | b'[' if self.value_allowed() => {
                self.open(b == b'{')?;
                Ok(Step::Token(if b == b'{' {
                    Kind::BeginObject
                } else {
                    Kind::BeginArray
                }))
            }
            b'}' if matches!(self.expect, Expect::NameOrEnd | Expect::CommaOrEnd) => {
                self.close(true)?;
                Ok(Step::Token(Kind::EndObject))
            }
            b']' if matches!(self.expect, Expect::ValueOrEnd | Expect::CommaOrEnd) => {
                self.close(false)?;
                Ok(Step::Token(Kind::EndArray))
            }
            b',' if self.expect == Expect::CommaOrEnd => {
                self.expect = if self.in_object() {
                    Expect::Name
                } else {
                    Expect::Value
                };
                Ok(Step::None)
            }
            b':' if self.expect == Expect::Colon => {
                self.expect = Expect::Value;
                Ok(Step::None)
            }
            b'"' if matches!(self.expect, Expect::Name | Expect::NameOrEnd)
                || self.value_allowed() =>
            {
                let name = matches!(self.expect, Expect::Name | Expect::NameOrEnd);
                self.len = 0;
                self.truncated = false;
                self.high = None;
                self.lex = Lex::Str {
                    name,
                    escape: Escape::None,
                };
                Ok(Step::None)
            }
            b'-' | b'0'..=b'9' if self.value_allowed() => {
                self.num[0] = b;
                self.num_len = 1;
                self.lex = Lex::Number;
                Ok(Step::None)
            }
            b't' | b'f' | b'n' if self.value_allowed() => {
                let lit: &'static [u8] = match b {
                    b't' => b"true",
                    b'f' => b"false",
                    _ => b"null",
                };
                self.lex = Lex::Literal(lit, 1);
                Ok(Step::None)
            }
            _ => Err(JsonError::Syntax),
        }
    }

    fn step_string(&mut self, b: u8, name: bool, escape: Escape) -> Result<Step, JsonError> {
        let set = |r: &mut Self, e: Escape| r.lex = Lex::Str { name, escape: e };
        match escape {
            Escape::None => match b {
                b'"' => {
                    self.flush_high();
                    self.lex = Lex::Between;
                    if name {
                        self.expect = Expect::Colon;
                        Ok(Step::Token(Kind::Name))
                    } else {
                        self.after_value();
                        Ok(Step::Token(Kind::String))
                    }
                }
                b'\\' => {
                    set(self, Escape::Backslash);
                    Ok(Step::None)
                }
                0x00..=0x1F => Err(JsonError::Syntax),
                _ => {
                    self.flush_high();
                    self.push_byte(b);
                    Ok(Step::None)
                }
            },
            Escape::Backslash => {
                let c = match b {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{8}',
                    b'f' => '\u{C}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => {
                        set(self, Escape::Hex(0, 0));
                        return Ok(Step::None);
                    }
                    _ => return Err(JsonError::Syntax),
                };
                self.flush_high();
                self.push_char(c);
                set(self, Escape::None);
                Ok(Step::None)
            }
            Escape::Hex(n, v) => {
                let d = (b as char).to_digit(16).ok_or(JsonError::Syntax)? as u16;
                let v = (v << 4) | d;
                if n == 3 {
                    self.unicode(v);
                    set(self, Escape::None);
                } else {
                    set(self, Escape::Hex(n + 1, v));
                }
                Ok(Step::None)
            }
        }
    }
}

/// The tokens of one piece of bytes. Call [`Tokens::next_token`] until it gives `None`.
#[derive(Debug)]
pub struct Tokens<'r, 'i, const S: usize> {
    reader: &'r mut JsonReader<S>,
    input: &'i [u8],
    pos: usize,
    /// Start of the raw text of a captured value in this piece.
    raw_start: Option<usize>,
}

impl<'i, const S: usize> Tokens<'_, 'i, S> {
    /// The next value is given as `Raw` pieces of its unchanged text, as
    /// [`JsonReader::capture_value`] does, while the tokens of this piece are read.
    pub fn capture_value(&mut self) {
        self.reader.capture_next = true;
    }

    /// True while a captured value is not complete.
    pub(crate) fn capturing(&self) -> bool {
        self.reader.capture.is_some() || self.reader.capture_next
    }

    /// The next complete token, or `None` when the piece has no more. A token borrows the
    /// reader buffer, so it is valid until the next call.
    pub fn next_token(&mut self) -> Option<Result<Token<'_>, JsonError>> {
        if let Some(e) = self.reader.failed {
            return if self.pos < self.input.len() {
                self.pos = self.input.len();
                Some(Err(e))
            } else {
                None
            };
        }
        loop {
            let Some(&b) = self.input.get(self.pos) else {
                // End of the piece: give the raw text of a captured value so far.
                if let Some(s) = self.raw_start.take() {
                    if s < self.pos {
                        return Some(Ok(Token::Raw(&self.input[s..self.pos])));
                    }
                }
                return None;
            };
            let r = &mut *self.reader;
            if r.capture_next
                && r.lex == Lex::Between
                && r.value_allowed()
                && !matches!(b, b' ' | b'\t' | b'\n' | b'\r')
            {
                r.capture_next = false;
                r.capture = Some(r.depth);
            }
            if r.capture.is_some() && self.raw_start.is_none() {
                self.raw_start = Some(self.pos);
            }
            let step = match r.step(b) {
                Ok(s) => s,
                Err(e) => {
                    r.failed = Some(e);
                    self.pos = self.input.len();
                    return Some(Err(e));
                }
            };
            let (kind, _again) = match step {
                Step::None => {
                    self.pos += 1;
                    continue;
                }
                Step::Token(k) => {
                    self.pos += 1;
                    (k, false)
                }
                Step::TokenAgain(k) => (k, true),
            };
            if let Some(d) = self.reader.capture {
                // The captured value ends when its depth comes back after a token.
                let closed =
                    self.reader.depth == d && !matches!(kind, Kind::BeginObject | Kind::BeginArray);
                if closed {
                    self.reader.capture = None;
                    let s = self.raw_start.take().unwrap_or(self.pos);
                    let end = self.pos;
                    // After a number, `pos` stays on its delimiter, which the next call reads.
                    return Some(Ok(Token::Raw(&self.input[s..end])));
                }
                continue;
            }
            let r = &*self.reader;
            return Some(Ok(match kind {
                Kind::BeginObject => Token::BeginObject,
                Kind::EndObject => Token::EndObject,
                Kind::BeginArray => Token::BeginArray,
                Kind::EndArray => Token::EndArray,
                Kind::Name => Token::Name {
                    text: r.text(),
                    truncated: r.truncated,
                },
                Kind::String => Token::String {
                    text: r.text(),
                    truncated: r.truncated,
                },
                Kind::Number => match r.num_text() {
                    Ok(text) => Token::Number(Number { text }),
                    Err(e) => return Some(Err(e)),
                },
                Kind::Bool(v) => Token::Bool(v),
                Kind::Null => Token::Null,
            }));
        }
    }
}

/// Writer of one JSON value into a caller buffer, with no whitespace (REQ-PE-HDR-005).
#[derive(Debug)]
pub struct JsonWriter<'b> {
    buf: &'b mut [u8],
    pos: usize,
    stack: u64,
    depth: u8,
    /// No element written yet at this level.
    first: bool,
    /// A name was written: a value comes next.
    after_name: bool,
}

const HEX: &[u8; 16] = b"0123456789abcdef";

impl<'b> JsonWriter<'b> {
    /// A writer over `buf`.
    pub fn new(buf: &'b mut [u8]) -> Self {
        JsonWriter {
            buf,
            pos: 0,
            stack: 0,
            depth: 0,
            first: true,
            after_name: false,
        }
    }

    /// Bytes written.
    pub fn len(&self) -> usize {
        self.pos
    }

    /// True when nothing is written.
    pub fn is_empty(&self) -> bool {
        self.pos == 0
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), JsonError> {
        let end = self
            .pos
            .checked_add(bytes.len())
            .ok_or(JsonError::Overflow)?;
        self.buf
            .get_mut(self.pos..end)
            .ok_or(JsonError::Overflow)?
            .copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }

    fn in_object(&self) -> bool {
        self.depth > 0 && self.stack & (1 << (self.depth - 1)) != 0
    }

    /// Before a value: the comma, and the check that an object member has a name.
    fn before_value(&mut self) -> Result<(), JsonError> {
        if self.in_object() {
            if !self.after_name {
                return Err(JsonError::InvalidValue);
            }
            self.after_name = false;
        } else {
            if self.depth == 0 && self.pos > 0 {
                return Err(JsonError::InvalidValue);
            }
            if !self.first {
                self.put(b",")?;
            }
        }
        self.first = false;
        Ok(())
    }

    fn open(&mut self, object: bool) -> Result<(), JsonError> {
        self.before_value()?;
        if self.depth >= MAX_DEPTH {
            return Err(JsonError::Depth);
        }
        self.put(if object { b"{" } else { b"[" })?;
        if object {
            self.stack |= 1 << self.depth;
        } else {
            self.stack &= !(1 << self.depth);
        }
        self.depth += 1;
        self.first = true;
        Ok(())
    }

    fn close(&mut self, object: bool) -> Result<(), JsonError> {
        if self.depth == 0 || self.in_object() != object || self.after_name {
            return Err(JsonError::InvalidValue);
        }
        self.put(if object { b"}" } else { b"]" })?;
        self.depth -= 1;
        self.first = false;
        Ok(())
    }

    /// `{`
    pub fn begin_object(&mut self) -> Result<(), JsonError> {
        self.open(true)
    }

    /// `}`
    pub fn end_object(&mut self) -> Result<(), JsonError> {
        self.close(true)
    }

    /// `[`
    pub fn begin_array(&mut self) -> Result<(), JsonError> {
        self.open(false)
    }

    /// `]`
    pub fn end_array(&mut self) -> Result<(), JsonError> {
        self.close(false)
    }

    /// Writes an object name: a-z, A-Z, 0-9 only, not only digits, at most `max_len`
    /// characters (REQ-PE-JSON-002, -003; 20 for Header Data, REQ-PE-HDR-003).
    pub fn name(&mut self, name: &str, max_len: usize) -> Result<(), JsonError> {
        let ok = !name.is_empty()
            && name.len() <= max_len
            && name.bytes().all(|b| b.is_ascii_alphanumeric())
            && !name.bytes().all(|b| b.is_ascii_digit());
        if !ok {
            return Err(JsonError::InvalidName);
        }
        self.name_unchecked(name)
    }

    /// Writes an object name with only the JSON escapes, for names that are not Property
    /// names (JSON Pointer keys of a Partial SET).
    pub fn key(&mut self, key: &str) -> Result<(), JsonError> {
        self.name_unchecked(key)
    }

    fn name_unchecked(&mut self, name: &str) -> Result<(), JsonError> {
        if !self.in_object() || self.after_name {
            return Err(JsonError::InvalidName);
        }
        if !self.first {
            self.put(b",")?;
        }
        self.first = false;
        self.escaped(name, usize::MAX)?;
        self.put(b":")?;
        self.after_name = true;
        Ok(())
    }

    /// Writes a quoted string with the escapes of REQ-PE-JSON-006 and -007. At most `limit`
    /// bytes of escaped text are written, cut at a character boundary; gives true when cut.
    fn escaped(&mut self, s: &str, limit: usize) -> Result<bool, JsonError> {
        self.put(b"\"")?;
        let cut = self.escaped_text(s, limit)?;
        self.put(b"\"")?;
        Ok(cut)
    }

    /// The text of [`JsonWriter::escaped`] with no quotes.
    fn escaped_text(&mut self, s: &str, limit: usize) -> Result<bool, JsonError> {
        let mut used = 0usize;
        let mut cut = false;
        for c in s.chars() {
            let mut tmp = [0u8; 12];
            let n = match c {
                '"' => {
                    tmp[..2].copy_from_slice(b"\\\"");
                    2
                }
                '\\' => {
                    tmp[..2].copy_from_slice(b"\\\\");
                    2
                }
                '\u{8}' => {
                    tmp[..2].copy_from_slice(b"\\b");
                    2
                }
                '\u{C}' => {
                    tmp[..2].copy_from_slice(b"\\f");
                    2
                }
                '\n' => {
                    tmp[..2].copy_from_slice(b"\\n");
                    2
                }
                '\r' => {
                    tmp[..2].copy_from_slice(b"\\r");
                    2
                }
                '\t' => {
                    tmp[..2].copy_from_slice(b"\\t");
                    2
                }
                c if (c as u32) < 0x20 || (c as u32) > 0x7E => {
                    let mut units = [0u16; 2];
                    let mut n = 0;
                    for u in c.encode_utf16(&mut units).iter() {
                        tmp[n..n + 2].copy_from_slice(b"\\u");
                        for k in 0..4 {
                            tmp[n + 2 + k] = HEX[((u >> (12 - 4 * k)) & 0xF) as usize];
                        }
                        n += 6;
                    }
                    n
                }
                c => {
                    tmp[0] = c as u8;
                    1
                }
            };
            if used + n > limit {
                cut = true;
                break;
            }
            self.put(&tmp[..n])?;
            used += n;
        }
        Ok(cut)
    }

    /// Writes a string value (REQ-PE-JSON-006, -007).
    pub fn string(&mut self, s: &str) -> Result<(), JsonError> {
        self.before_value()?;
        self.escaped(s, usize::MAX).map(|_| ())
    }

    /// Writes a string value from characters, with the escapes of [`JsonWriter::string`]. A
    /// received string is written again this way, with no length limit of its own.
    pub fn string_chars(&mut self, chars: impl Iterator<Item = char>) -> Result<(), JsonError> {
        self.before_value()?;
        self.put(b"\"")?;
        for c in chars {
            let mut tmp = [0u8; 4];
            self.escaped_body(c.encode_utf8(&mut tmp))?;
        }
        self.put(b"\"")
    }

    fn escaped_body(&mut self, s: &str) -> Result<(), JsonError> {
        // No quotes and no `copy_within`: a move links `memmove` (1.4 KB of flash).
        self.escaped_text(s, usize::MAX).map(|_| ())
    }

    /// Writes a string value of at most `limit` escaped bytes, cut at a character boundary;
    /// gives true when the text was cut (DEC-010 D-4).
    pub fn string_limited(&mut self, s: &str, limit: usize) -> Result<bool, JsonError> {
        self.before_value()?;
        self.escaped(s, limit)
    }

    /// Writes an unsigned integer in decimal (REQ-PE-JSON-005).
    pub fn number_u64(&mut self, v: u64) -> Result<(), JsonError> {
        self.before_value()?;
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        let mut v = v;
        loop {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        self.put(&tmp[i..])
    }

    /// Writes an unsigned 32-bit integer in decimal (REQ-PE-JSON-005), with 32-bit division
    /// only: the Cortex-M4 has no 64-bit division instruction (DEC-018).
    pub fn number_u32(&mut self, v: u32) -> Result<(), JsonError> {
        self.before_value()?;
        let mut tmp = [0u8; 10];
        let mut i = tmp.len();
        let mut v = v;
        loop {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        self.put(&tmp[i..])
    }

    /// Writes a signed integer in decimal (REQ-PE-JSON-005).
    pub fn number_i64(&mut self, v: i64) -> Result<(), JsonError> {
        if v < 0 {
            self.before_value()?;
            self.put(b"-")?;
            // Write the magnitude without a second comma.
            let mut tmp = [0u8; 20];
            let mut i = tmp.len();
            let mut m = v.unsigned_abs();
            loop {
                i -= 1;
                tmp[i] = b'0' + (m % 10) as u8;
                m /= 10;
                if m == 0 {
                    break;
                }
            }
            self.put(&tmp[i..])
        } else {
            self.number_u64(v as u64)
        }
    }

    /// Writes `true` or `false`.
    pub fn boolean(&mut self, v: bool) -> Result<(), JsonError> {
        self.before_value()?;
        self.put(if v { b"true" } else { b"false" })
    }

    /// Writes `null`.
    pub fn null(&mut self) -> Result<(), JsonError> {
        self.before_value()?;
        self.put(b"null")
    }

    /// Writes caller JSON text unchanged ("schema", "propData": DEC-007). The text must be
    /// 7-bit and not empty; the caller is responsible for its JSON form.
    pub fn raw(&mut self, text: &[u8]) -> Result<(), JsonError> {
        if text.is_empty() || text.iter().any(|b| *b > 0x7F) {
            return Err(JsonError::InvalidValue);
        }
        self.before_value()?;
        self.put(text)
    }

    /// Ends the value; gives the length. An error when a container is open.
    pub fn finish(self) -> Result<usize, JsonError> {
        if self.depth != 0 || self.after_name || self.pos == 0 {
            return Err(JsonError::InvalidValue);
        }
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::string::{String, ToString};
    use std::vec::Vec;

    /// Owned form of a token for comparison.
    #[derive(Debug, Clone, PartialEq)]
    enum T {
        Bo,
        Eo,
        Ba,
        Ea,
        N(String, bool),
        S(String, bool),
        Num(String),
        B(bool),
        Null,
        Raw(Vec<u8>),
    }

    fn own(t: Token<'_>) -> T {
        match t {
            Token::BeginObject => T::Bo,
            Token::EndObject => T::Eo,
            Token::BeginArray => T::Ba,
            Token::EndArray => T::Ea,
            Token::Name { text, truncated } => T::N(text.to_string(), truncated),
            Token::String { text, truncated } => T::S(text.to_string(), truncated),
            Token::Number(n) => T::Num(n.text().to_string()),
            Token::Bool(b) => T::B(b),
            Token::Null => T::Null,
            Token::Raw(r) => T::Raw(r.to_vec()),
        }
    }

    /// Reads `pieces` and gives all tokens, joined Raw pieces, or the first error.
    fn read<const S: usize>(r: &mut JsonReader<S>, pieces: &[&[u8]]) -> Result<Vec<T>, JsonError> {
        let mut out: Vec<T> = Vec::new();
        for p in pieces {
            let mut t = r.push(p);
            while let Some(x) = t.next_token() {
                let x = own(x?);
                match (out.last_mut(), &x) {
                    (Some(T::Raw(a)), T::Raw(b)) => a.extend_from_slice(b),
                    _ => out.push(x),
                }
            }
        }
        if let Some(t) = r.finish()? {
            out.push(own(t));
        }
        Ok(out)
    }

    fn s(x: &str) -> String {
        x.to_string()
    }

    #[test]
    fn reads_header_object() {
        // REQ-PE-JSON-001, REQ-PE-HDR-005 (whitespace between tokens is accepted).
        let mut r = JsonReader::<64>::new();
        let t = read(
            &mut r,
            &[b"{ \"resource\" : \"DeviceInfo\",\n\"status\":200,\"x\":[true,null,-1.5e3]}"],
        )
        .unwrap();
        assert_eq!(
            t,
            [
                T::Bo,
                T::N(s("resource"), false),
                T::S(s("DeviceInfo"), false),
                T::N(s("status"), false),
                T::Num(s("200")),
                T::N(s("x"), false),
                T::Ba,
                T::B(true),
                T::Null,
                T::Num(s("-1.5e3")),
                T::Ea,
                T::Eo
            ]
        );
    }

    #[test]
    fn tokens_cross_piece_boundaries() {
        // A string, a number and a literal split over pieces (one Chunk at a time).
        let whole: &[u8] = b"{\"name\":\"Beats\\u266a\",\"n\":12345,\"t\":true}";
        let mut r = JsonReader::<64>::new();
        let expected = read(&mut r, &[whole]).unwrap();
        for cut in 1..whole.len() {
            let mut r = JsonReader::<64>::new();
            let got = read(&mut r, &[&whole[..cut], &whole[cut..]]).unwrap();
            assert_eq!(got, expected, "cut at {cut}");
        }
        assert_eq!(expected[2], T::S(s("Beats\u{266A}"), false));
    }

    #[test]
    fn escapes_and_surrogates() {
        // REQ-PE-JSON-006: all ECMA-404 escapes, either hex case, "/" and "\/", pairs.
        let mut r = JsonReader::<64>::new();
        let t = read(
            &mut r,
            &[br#"["a\"b\\c\/d/e\b\f\n\r\t","\u00E9\u00e9","\ud834\udd1e","\ud834x"]"#],
        )
        .unwrap();
        assert_eq!(t[1], T::S(s("a\"b\\c/d/e\u{8}\u{C}\n\r\t"), false));
        assert_eq!(t[2], T::S(s("éé"), false));
        assert_eq!(t[3], T::S(s("𝄞"), false));
        assert_eq!(
            t[4],
            T::S(s("\u{FFFD}x"), false),
            "unpaired surrogate gives U+FFFD"
        );
    }

    #[test]
    fn names_are_case_sensitive_and_truncation_is_marked() {
        // REQ-PE-JSON-004; fixed buffer (DEC-007).
        let mut r = JsonReader::<8>::new();
        let t = read(&mut r, &[r#"{"Resource":"abcdefghijé"}"#.as_bytes()]).unwrap();
        assert_eq!(t[1], T::N(s("Resource"), false));
        assert_eq!(t[2], T::S(s("abcdefgh"), true));
        let mut r = JsonReader::<10>::new();
        let t = read(&mut r, &[r#"["abcdefghijé"]"#.as_bytes()]).unwrap();
        assert_eq!(
            t[1],
            T::S(s("abcdefghij"), true),
            "cut at a character boundary"
        );
    }

    #[test]
    fn syntax_errors() {
        for bad in [
            &b"{\"a\" 1}"[..],
            b"{\"a\":}",
            b"[1,]",
            b"{,}",
            b"[01]",
            b"tru",
            b"\"a\x01\"",
            b"[1}",
            b"{} {}",
            b"\"\\x\"",
        ] {
            let mut r = JsonReader::<16>::new();
            assert!(
                read(&mut r, &[bad]).is_err(),
                "{:?}",
                core::str::from_utf8(bad)
            );
        }
        let mut r = JsonReader::<16>::new();
        let deep = [b'['; 65];
        assert_eq!(read(&mut r, &[&deep]).err(), Some(JsonError::Depth));
        let mut r = JsonReader::<16>::new();
        assert_eq!(read(&mut r, &[b"{\"a\":1"]).err(), Some(JsonError::Syntax));
    }

    #[test]
    fn top_level_number_at_finish() {
        let mut r = JsonReader::<16>::new();
        assert_eq!(read(&mut r, &[b"4", b"2"]).unwrap(), [T::Num(s("42"))]);
    }

    #[test]
    fn number_helpers() {
        // REQ-PE-HDR-011, -013 use these.
        let n = |t| Number { text: t };
        assert_eq!(n("4294967295").as_u32(), Ok(u32::MAX));
        assert_eq!(n("4294967296").as_u32(), Err(JsonError::InvalidValue));
        assert_eq!(n("1.0").as_u64(), Err(JsonError::InvalidValue));
        assert_eq!(n("1e3").as_u64(), Err(JsonError::InvalidValue));
        assert_eq!(n("-7").as_i64(), Ok(-7));
        assert_eq!(n("-7").as_u64(), Err(JsonError::InvalidValue));
    }

    #[test]
    fn capture_value_object_and_scalar() {
        let mut r = JsonReader::<16>::new();
        let mut out = Vec::new();
        {
            let mut t = r.push(b"{\"schema\":");
            while let Some(x) = t.next_token() {
                out.push(own(x.unwrap()));
            }
        }
        r.capture_value();
        for p in [&b" {\"t\":\"}\","[..], b"\"x\":[1,{}]}", b",\"b\":7}"] {
            let mut t = r.push(p);
            while let Some(x) = t.next_token() {
                let x = own(x.unwrap());
                match (out.last_mut(), &x) {
                    (Some(T::Raw(a)), T::Raw(b)) => a.extend_from_slice(b),
                    _ => out.push(x),
                }
            }
        }
        r.finish().unwrap();
        assert_eq!(
            out,
            [
                T::Bo,
                T::N(s("schema"), false),
                T::Raw(b"{\"t\":\"}\",\"x\":[1,{}]}".to_vec()),
                T::N(s("b"), false),
                T::Num(s("7")),
                T::Eo
            ]
        );
        // A captured number ends at its delimiter.
        let mut r = JsonReader::<16>::new();
        let mut out = Vec::new();
        {
            let mut t = r.push(b"[");
            while let Some(x) = t.next_token() {
                out.push(own(x.unwrap()));
            }
        }
        r.capture_value();
        let mut t = r.push(b"123,4]");
        while let Some(x) = t.next_token() {
            out.push(own(x.unwrap()));
        }
        assert_eq!(out, [T::Ba, T::Raw(b"123".to_vec()), T::Num(s("4")), T::Ea]);
    }

    fn write(
        f: impl FnOnce(&mut JsonWriter<'_>) -> Result<(), JsonError>,
    ) -> Result<String, JsonError> {
        let mut buf = [0u8; 256];
        let mut w = JsonWriter::new(&mut buf);
        f(&mut w)?;
        let n = w.finish()?;
        Ok(String::from_utf8(buf[..n].to_vec()).unwrap())
    }

    #[test]
    fn writer_output() {
        // REQ-PE-JSON-005 to -007, REQ-PE-HDR-005: one line, no whitespace.
        let out = write(|w| {
            w.begin_object()?;
            w.name("resource", 20)?;
            w.string("Beats♪ a/b \"q\" \\")?;
            w.name("n", 20)?;
            w.number_u64(0x42)?;
            w.name("neg", 20)?;
            w.number_i64(-12)?;
            w.name("list", 20)?;
            w.begin_array()?;
            w.boolean(true)?;
            w.null()?;
            w.begin_object()?;
            w.end_object()?;
            w.raw(b"{\"x\":1}")?;
            w.end_array()?;
            w.end_object()
        })
        .unwrap();
        assert_eq!(
            out,
            r#"{"resource":"Beats\u266a a/b \"q\" \\","n":66,"neg":-12,"list":[true,null,{},{"x":1}]}"#
        );
        assert_eq!(
            write(|w| w.string("𝄞\u{1}")).unwrap(),
            "\"\\ud834\\udd1e\\u0001\""
        );
    }

    #[test]
    fn writer_rejects_bad_names_and_structure() {
        // REQ-PE-JSON-002, -003, REQ-PE-HDR-003.
        for bad in ["", "res-id", "0", "120", "a b", "abcdefghijklmnopqrstu"] {
            let r = write(|w| {
                w.begin_object()?;
                w.name(bad, 20)?;
                w.number_u64(1)?;
                w.end_object()
            });
            assert_eq!(r, Err(JsonError::InvalidName), "{bad}");
        }
        assert!(write(|w| {
            w.begin_object()?;
            w.number_u64(1)?;
            w.end_object()
        })
        .is_err());
        assert!(write(|w| w.begin_array()).is_err(), "open array at finish");
        let mut small = [0u8; 4];
        let mut w = JsonWriter::new(&mut small);
        assert_eq!(w.string("abcd"), Err(JsonError::Overflow));
    }

    #[test]
    fn string_limit_cuts_at_character_boundary() {
        // DEC-010 D-4: at most 512 bytes of escaped text.
        let mut buf = [0u8; 64];
        let mut w = JsonWriter::new(&mut buf);
        assert_eq!(w.string_limited("abé", 7), Ok(true));
        let n = w.finish().unwrap();
        assert_eq!(&buf[..n], b"\"ab\"");
    }

    #[test]
    fn prop_writer_reader_round_trip() {
        proptest!(|(text in "\\PC{0,24}", n in any::<u64>(), b in any::<bool>())| {
            let out = write(|w| {
                w.begin_object()?;
                w.name("text", 20)?;
                w.string(&text)?;
                w.name("n", 20)?;
                w.number_u64(n)?;
                w.name("b", 20)?;
                w.boolean(b)?;
                w.end_object()
            }).unwrap();
            prop_assert!(out.is_ascii());
            let mut r = JsonReader::<256>::new();
            let t = read(&mut r, &[out.as_bytes()]).unwrap();
            prop_assert_eq!(&t[2], &T::S(text.clone(), false));
            prop_assert_eq!(&t[4], &T::Num(n.to_string()));
            prop_assert_eq!(&t[6], &T::B(b));
        });
    }

    #[test]
    fn prop_reader_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..64), cut in 0usize..64)| {
            let mut r = JsonReader::<8>::new();
            let c = cut.min(bytes.len());
            let _ = read(&mut r, &[&bytes[..c], &bytes[c..]]);
        });
    }

    #[test]
    fn prop_number_u32_writes_what_number_u64_writes() {
        // REQ-PE-JSON-005, DEC-018: the 32-bit writer gives the same text as the 64-bit one.
        proptest::proptest!(|(v in proptest::prelude::any::<u32>())| {
            let (mut a, mut b) = ([0u8; 16], [0u8; 16]);
            let mut wa = JsonWriter::new(&mut a);
            wa.number_u32(v).unwrap();
            let na = wa.len();
            let mut wb = JsonWriter::new(&mut b);
            wb.number_u64(u64::from(v)).unwrap();
            let nb = wb.len();
            proptest::prop_assert_eq!(&a[..na], &b[..nb]);
        });
    }

    #[test]
    fn prop_slice_walker_gives_the_members_of_the_streaming_walker() {
        // REQ-PE-HDR-014, OPT-020: on any ASCII bytes, both walkers give the same members and
        // the same error.
        type Member = (Option<std::string::String>, Vec<u8>);
        fn members(bytes: &[u8], slice: bool) -> Result<Vec<Member>, JsonError> {
            let mut v = Vec::new();
            if slice {
                for_each_member_slice(
                    bytes,
                    |n: Option<&[u8]>, raw: &[u8]| -> Result<(), JsonError> {
                        let n = n.map(|n| std::string::String::from_utf8(n.to_vec()).unwrap());
                        v.push((n, raw.to_vec()));
                        Ok(())
                    },
                )?;
            } else {
                for_each_member(
                    bytes,
                    |n: Option<&str>, raw: &[u8]| -> Result<(), JsonError> {
                        v.push((n.map(std::string::String::from), raw.to_vec()));
                        Ok(())
                    },
                )?;
            }
            Ok(v)
        }
        proptest!(ProptestConfig::with_cases(30_000), |(m in proptest::collection::vec(0u8..0x80, 0..40))| {
            prop_assert_eq!(members(&m, true), members(&m, false));
        });
        assert_eq!(
            members(b"{ \"a\" : [1, {\"b\": null}], \"c\":\"\\u0041\" }", true),
            members(b"{ \"a\" : [1, {\"b\": null}], \"c\":\"\\u0041\" }", false)
        );
    }
}
