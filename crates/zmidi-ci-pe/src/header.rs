//! Property Exchange Header Data (M2-103-UM v1.2 sections 7, 10, 11; Tables 14 to 20).
//!
//! - Header Data is one flat JSON object in the first Chunk (7.1). [`Header::parse`] reads it
//!   from the received bytes with no copy: each string is kept as its escaped JSON text
//!   ([`HeaderStr`]) and decoded on demand. Unknown Properties of any type are skipped
//!   (REQ-PE-HDR-002, -003).
//! - [`Header::write`] writes the Properties that are set, the first one by the rule of
//!   REQ-PE-HDR-004, and checks the writer limits ("resource" and "resId" 36, "mediaType" 75,
//!   "subscribeId" 8, "message" and "msg" 512 bytes after escaping).
//! - A received "message" or "msg" longer than 512 escaped bytes is cut at a character boundary
//!   and marked as truncated (DEC-010 D-4, REQ-PE-STAT-005, REQ-PE-LINK-008).

use crate::json::{JsonError, JsonWriter};

/// Header Property names are at most 20 characters (REQ-PE-HDR-003).
pub const MAX_KEY: usize = 20;
/// Limit of "message" and "msg" after escaping (Table 14, DEC-010 D-4).
pub const MAX_MESSAGE: usize = 512;
const MAX_RESOURCE: usize = 36;
const MAX_MEDIA_TYPE: usize = 75;
const MAX_SUBSCRIBE_ID_WRITE: usize = 8;
const MAX_SUBSCRIBE_ID_READ: usize = 16;

/// Error of the header parser and writer; it names the Property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    /// The Header Data is not a valid JSON object.
    #[error("header JSON: {0}")]
    Json(JsonError),
    /// A required Property is missing.
    #[error("missing \"{0}\"")]
    Missing(&'static str),
    /// A known Property has a value of the wrong JSON type.
    #[error("wrong type of \"{0}\"")]
    WrongType(&'static str),
    /// A value is out of range, too long, or breaks the character rules.
    #[error("invalid \"{0}\"")]
    Invalid(&'static str),
}

impl From<JsonError> for HeaderError {
    fn from(e: JsonError) -> Self {
        HeaderError::Json(e)
    }
}

/// A header string: escaped JSON text from a received header, or caller text to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderStr<'a> {
    /// The escaped text between the quotes, as received.
    Raw(&'a [u8]),
    /// Caller text, not escaped.
    Text(&'a str),
}

impl<'a> HeaderStr<'a> {
    /// The decoded characters.
    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    pub fn chars(&self) -> HeaderChars<'a> {
        match *self {
            HeaderStr::Raw(r) => HeaderChars { raw: r, text: "" },
            HeaderStr::Text(t) => HeaderChars { raw: &[], text: t },
        }
    }

    /// Number of decoded characters.
    pub fn char_count(&self) -> usize {
        self.chars().count()
    }

    /// True when the decoded text equals `s` (case-sensitive).
    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    pub fn eq_str(&self, s: &str) -> bool {
        self.chars().eq(s.chars())
    }

    /// True when the decoded text equals `s` with no case for ASCII letters.
    pub fn eq_ignore_case(&self, s: &str) -> bool {
        let mut a = self.chars();
        let mut b = s.chars();
        loop {
            match (a.next(), b.next()) {
                (None, None) => return true,
                (Some(x), Some(y)) if x.eq_ignore_ascii_case(&y) => {}
                _ => return false,
            }
        }
    }

    /// Writes the decoded text as UTF-8 into `out`; gives the used part, cut at a character
    /// boundary when `out` is too small, and true when it was cut.
    pub fn decode_into<'b>(&self, out: &'b mut [u8]) -> (&'b str, bool) {
        let mut n = 0;
        let mut cut = false;
        for c in self.chars() {
            let mut tmp = [0u8; 4];
            let s = c.encode_utf8(&mut tmp).as_bytes();
            if n + s.len() > out.len() {
                cut = true;
                break;
            }
            out[n..n + s.len()].copy_from_slice(s);
            n += s.len();
        }
        (core::str::from_utf8(&out[..n]).unwrap_or(""), cut)
    }

    /// Length of the text after the escape step of section 6.1.1.
    fn escaped_len(&self) -> usize {
        match *self {
            HeaderStr::Raw(r) => r.len(),
            HeaderStr::Text(t) => t
                .chars()
                .map(|c| match c {
                    '"' | '\\' | '\u{8}' | '\u{C}' | '\n' | '\r' | '\t' => 2,
                    c if (c as u32) < 0x20 || (c as u32) > 0x7E => 6 * c.len_utf16(),
                    _ => 1,
                })
                .sum(),
        }
    }

    fn write(&self, w: &mut JsonWriter<'_>) -> Result<(), JsonError> {
        match *self {
            HeaderStr::Text(t) => w.string(t),
            // Received text: decode it and write it with the normal escapes.
            HeaderStr::Raw(_) => w.string_chars(self.chars()),
        }
    }
}

/// Decoder of a [`HeaderStr`]: ECMA-404 escapes, surrogate pairs (U+FFFD when unpaired).
#[derive(Debug, Clone)]
pub struct HeaderChars<'a> {
    raw: &'a [u8],
    text: &'a str,
}

impl HeaderChars<'_> {
    fn hex4(&self, at: usize) -> Option<u16> {
        let s = self.raw.get(at..at + 4)?;
        let mut v = 0u16;
        for b in s {
            v = (v << 4) | (*b as char).to_digit(16)? as u16;
        }
        Some(v)
    }
}

impl Iterator for HeaderChars<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        if !self.text.is_empty() {
            // `as_str` and not a `str` slice: a slice links its panic path.
            let mut it = self.text.chars();
            let c = it.next()?;
            self.text = it.as_str();
            return Some(c);
        }
        let (&b, rest) = self.raw.split_first()?;
        if b != b'\\' {
            if b < 0x80 {
                self.raw = rest;
                return Some(b as char);
            }
            // UTF-8 bytes in a received string.
            let len = match b {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                _ => 1,
            };
            let take = len.min(self.raw.len());
            let c = core::str::from_utf8(&self.raw[..take])
                .ok()
                .and_then(|s| s.chars().next());
            self.raw = &self.raw[take..];
            return Some(c.unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        let e = rest.first().copied();
        let simple = match e {
            Some(b'"') => Some('"'),
            Some(b'\\') => Some('\\'),
            Some(b'/') => Some('/'),
            Some(b'b') => Some('\u{8}'),
            Some(b'f') => Some('\u{C}'),
            Some(b'n') => Some('\n'),
            Some(b'r') => Some('\r'),
            Some(b't') => Some('\t'),
            _ => None,
        };
        if let Some(c) = simple {
            self.raw = &self.raw[2..];
            return Some(c);
        }
        let Some(hi) = self.hex4(2) else {
            self.raw = &self.raw[self.raw.len().min(2)..];
            return Some(char::REPLACEMENT_CHARACTER);
        };
        self.raw = &self.raw[6..];
        if (0xD800..0xDC00).contains(&hi) {
            if self.raw.get(..2) == Some(b"\\u") {
                if let Some(lo) = self.hex4(2).filter(|lo| (0xDC00..0xE000).contains(lo)) {
                    self.raw = &self.raw[6..];
                    let c = 0x10000 + (((hi as u32) - 0xD800) << 10) + ((lo as u32) - 0xDC00);
                    return Some(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
                }
            }
            return Some(char::REPLACEMENT_CHARACTER);
        }
        Some(char::from_u32(hi as u32).unwrap_or(char::REPLACEMENT_CHARACTER))
    }
}

/// Reply status code (Table 15, REQ-PE-STAT-001 to -003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Status(pub u16);

/// Class of a status code by its hundreds digit (REQ-PE-STAT-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    /// 200-299: do not retry.
    Success,
    /// 300-399: retry.
    Redirection,
    /// 400-499: fail, do not retry.
    ClientError,
    /// 500-599: major error.
    ServerError,
    /// Below 200 or above 599.
    Unknown,
}

impl Status {
    /// 200 Success/Ok.
    pub const OK: Status = Status(200);
    /// 201 Accepted.
    pub const ACCEPTED: Status = Status(201);
    /// 341 Resource Currently Unavailable or an Error Occurred.
    pub const UNAVAILABLE: Status = Status(341);
    /// 342 Bad Data/Unexpected End of Data Set.
    pub const BAD_DATA: Status = Status(342);
    /// 343 Too Many Requests.
    pub const TOO_MANY_REQUESTS: Status = Status(343);
    /// 400 Bad Request.
    pub const BAD_REQUEST: Status = Status(400);
    /// 403 Reply not available based on Authorization.
    pub const FORBIDDEN: Status = Status(403);
    /// 404 Resource Not Supported/Found.
    pub const NOT_FOUND: Status = Status(404);
    /// 405 Resource Not Allowed.
    pub const NOT_ALLOWED: Status = Status(405);
    /// 406 Flow Control not supported.
    pub const FLOW_CONTROL_NOT_SUPPORTED: Status = Status(406);
    /// 407 Flow Control is required.
    pub const FLOW_CONTROL_REQUIRED: Status = Status(407);
    /// 413 Payload Too Large.
    pub const PAYLOAD_TOO_LARGE: Status = Status(413);
    /// 415 Unsupported Media Type or Encoding.
    pub const UNSUPPORTED_MEDIA: Status = Status(415);
    /// 445 Invalid Version of Data.
    pub const INVALID_VERSION: Status = Status(445);
    /// 500 Internal Device Error.
    pub const INTERNAL_ERROR: Status = Status(500);

    const TABLE_15: [u16; 15] = [
        200, 201, 341, 342, 343, 400, 403, 404, 405, 406, 407, 413, 415, 445, 500,
    ];

    /// True for a code of Table 15.
    pub fn is_known(self) -> bool {
        Self::TABLE_15.contains(&self.0)
    }

    /// The class of the code.
    pub fn class(self) -> StatusClass {
        match self.0 {
            200..=299 => StatusClass::Success,
            300..=399 => StatusClass::Redirection,
            400..=499 => StatusClass::ClientError,
            500..=599 => StatusClass::ServerError,
            _ => StatusClass::Unknown,
        }
    }
}

/// Encoding names of Table 12 (REQ-PE-ENC-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncodingName {
    /// "ASCII": not compressed and not encoded.
    Ascii,
    /// "Mcoded7".
    Mcoded7,
    /// "zlib+Mcoded7".
    ZlibMcoded7,
}

impl EncodingName {
    /// The Table 12 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            EncodingName::Ascii => "ASCII",
            EncodingName::Mcoded7 => "Mcoded7",
            EncodingName::ZlibMcoded7 => "zlib+Mcoded7",
        }
    }

    /// Reads a value with no case ("MCoded7" is "Mcoded7"); `None` for any other value.
    pub fn parse(s: &HeaderStr<'_>) -> Option<Self> {
        [
            EncodingName::Ascii,
            EncodingName::Mcoded7,
            EncodingName::ZlibMcoded7,
        ]
        .into_iter()
        .find(|e| s.eq_ignore_case(e.as_str()))
    }
}

/// "command" of a Subscription message (REQ-PE-SUB-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Start,
    Partial,
    Full,
    Notify,
    End,
}

impl Command {
    const ALL: [Command; 5] = [
        Command::Start,
        Command::Partial,
        Command::Full,
        Command::Notify,
        Command::End,
    ];

    /// The exact spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Command::Start => "start",
            Command::Partial => "partial",
            Command::Full => "full",
            Command::Notify => "notify",
            Command::End => "end",
        }
    }
}

/// "endedBy" of an "end" Subscription message (REQ-PE-SUB-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndedBy {
    Initiator,
    Responder,
}

impl EndedBy {
    /// The spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            EndedBy::Initiator => "initiator",
            EndedBy::Responder => "responder",
        }
    }
}

/// Which message a header belongs to: it sets the required and the first Property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderKind {
    /// Inquiry: Get or Set Property Data: "resource" first and required.
    Inquiry,
    /// Reply to Get, Set or Subscription, and Notify: "status" first and required.
    Reply,
    /// Subscription message: "command" first and required.
    Subscription,
}

/// The Properties of one header. A Property is `None` (or false) when absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Header<'a> {
    /// "resource", 1-36 characters.
    pub resource: Option<HeaderStr<'a>>,
    /// "resId", up to 36 characters.
    pub res_id: Option<HeaderStr<'a>>,
    /// "mutualEncoding"; absent means ASCII (REQ-PE-ENC-003).
    pub mutual_encoding: Option<EncodingName>,
    /// "mediaType", up to 75 characters.
    pub media_type: Option<HeaderStr<'a>>,
    /// "offset" and "limit", both or none (REQ-PE-TXN-006).
    pub offset: Option<u32>,
    /// "limit", 1 and up.
    pub limit: Option<u32>,
    /// "setPartial".
    pub set_partial: bool,
    /// "flowControl".
    pub flow_control: bool,
    /// "command".
    pub command: Option<Command>,
    /// "subscribeId".
    pub subscribe_id: Option<HeaderStr<'a>>,
    /// "endedBy".
    pub ended_by: Option<EndedBy>,
    /// "status".
    pub status: Option<Status>,
    /// "message"; `message_truncated` when cut.
    pub message: Option<HeaderStr<'a>>,
    /// The received "message" was cut.
    pub message_truncated: bool,
    /// "msg" of a Reply to Set Property Data.
    pub msg: Option<HeaderStr<'a>>,
    /// The received "msg" was cut.
    pub msg_truncated: bool,
    /// "cacheTime", seconds.
    pub cache_time: Option<u32>,
    /// "totalCount".
    pub total_count: Option<u32>,
}

/// A raw value from the reader.
enum Raw<'a> {
    Str(&'a [u8]),
    Num(&'a str),
    Bool(bool),
    Other,
}

fn classify(raw: &[u8]) -> Raw<'_> {
    match raw.first() {
        Some(b'"') => Raw::Str(&raw[1..raw.len().saturating_sub(1).max(1)]),
        Some(b't') => Raw::Bool(true),
        Some(b'f') => Raw::Bool(false),
        Some(b'-' | b'0'..=b'9') => core::str::from_utf8(raw)
            .map(Raw::Num)
            .unwrap_or(Raw::Other),
        _ => Raw::Other,
    }
}

/// Cuts escaped text at most `limit` bytes long at an escape and character boundary.
fn cut_escaped(raw: &[u8], limit: usize) -> &[u8] {
    if raw.len() <= limit {
        return raw;
    }
    let mut i = 0;
    let mut last = 0;
    while i < raw.len() {
        let step = match raw[i] {
            b'\\' => match raw.get(i + 1) {
                Some(b'u') => 6,
                _ => 2,
            },
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        if i + step > limit {
            break;
        }
        i += step;
        last = i;
    }
    &raw[..last]
}

fn resource_chars_ok(s: &str) -> bool {
    let body = s.strip_prefix("X-").unwrap_or(s);
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// RFC 6838 "type/subtype" with restricted-name characters.
pub(crate) fn media_type_ok(s: &str) -> bool {
    let name = |p: &str| {
        !p.is_empty()
            && p.len() <= 127
            && p.as_bytes()[0].is_ascii_alphanumeric()
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
    };
    matches!(s.split_once('/'), Some((t, st)) if name(t) && name(st))
}

impl<'a> Header<'a> {
    /// Reads Header Data (REQ-PE-JSON-001, REQ-PE-HDR-002 to -004). Unknown Properties of
    /// any type are skipped; the Properties can come in any order. The Header Data is one whole
    /// JSON object, walked in place (REQ-PE-HDR-014, OPT-020): a JSON error comes before any
    /// Property error, and a name with a character that is not ASCII is never a known Property.
    pub fn parse(kind: HeaderKind, bytes: &'a [u8]) -> Result<Self, HeaderError> {
        let mut h = Header::default();
        let (mut have_offset, mut have_limit) = (false, false);
        crate::json::for_each_member_slice(bytes, |name, raw| match name.and_then(known_bytes) {
            Some(name) => h.set(name, classify(raw), &mut have_offset, &mut have_limit),
            None => Ok(()),
        })?;
        h.check(kind, have_offset, have_limit)?;
        Ok(h)
    }

    /// The parse before OPT-020, with the streaming walker: the reference of the comparison test.
    #[cfg(test)]
    fn parse_streaming(kind: HeaderKind, bytes: &'a [u8]) -> Result<Self, HeaderError> {
        let mut h = Header::default();
        let (mut have_offset, mut have_limit) = (false, false);
        crate::json::for_each_member(bytes, |name, raw| match name.and_then(known) {
            Some(name) => h.set(name, classify(raw), &mut have_offset, &mut have_limit),
            None => Ok(()),
        })?;
        h.check(kind, have_offset, have_limit)?;
        Ok(h)
    }

    fn set(
        &mut self,
        name: &'static str,
        v: Raw<'a>,
        off: &mut bool,
        lim: &mut bool,
    ) -> Result<(), HeaderError> {
        let s = |v: Raw<'a>| match v {
            Raw::Str(s) => Ok(HeaderStr::Raw(s)),
            _ => Err(HeaderError::WrongType(name)),
        };
        let n = |v: Raw<'a>| match v {
            Raw::Num(t) => Ok(t),
            _ => Err(HeaderError::WrongType(name)),
        };
        let b = |v: Raw<'a>| match v {
            Raw::Bool(b) => Ok(b),
            _ => Err(HeaderError::WrongType(name)),
        };
        let u32_of = |t: &str| crate::json::number_u32(t).map_err(|_| HeaderError::Invalid(name));
        match name {
            "resource" => {
                let v = s(v)?;
                if !(1..=MAX_RESOURCE).contains(&v.char_count()) {
                    return Err(HeaderError::Invalid(name));
                }
                self.resource = Some(v);
            }
            "resId" => {
                let v = s(v)?;
                if v.char_count() > MAX_RESOURCE {
                    return Err(HeaderError::Invalid(name));
                }
                self.res_id = Some(v);
            }
            "mutualEncoding" => {
                let v = s(v)?;
                self.mutual_encoding =
                    Some(EncodingName::parse(&v).ok_or(HeaderError::Invalid(name))?);
            }
            "mediaType" => {
                let v = s(v)?;
                if v.char_count() > MAX_MEDIA_TYPE {
                    return Err(HeaderError::Invalid(name));
                }
                self.media_type = Some(v);
            }
            "offset" => {
                self.offset = Some(u32_of(n(v)?)?);
                *off = true;
            }
            "limit" => {
                let l = u32_of(n(v)?)?;
                if l == 0 {
                    return Err(HeaderError::Invalid(name));
                }
                self.limit = Some(l);
                *lim = true;
            }
            "setPartial" => self.set_partial = b(v)?,
            "flowControl" => self.flow_control = b(v)?,
            "command" => {
                let v = s(v)?;
                self.command = Some(
                    Command::ALL
                        .into_iter()
                        .find(|c| v.eq_str(c.as_str()))
                        .ok_or(HeaderError::Invalid(name))?,
                );
            }
            "subscribeId" => {
                let v = s(v)?;
                if !(1..=MAX_SUBSCRIBE_ID_READ).contains(&v.char_count()) {
                    return Err(HeaderError::Invalid(name));
                }
                self.subscribe_id = Some(v);
            }
            "endedBy" => {
                let v = s(v)?;
                self.ended_by = Some(match () {
                    _ if v.eq_str("initiator") => EndedBy::Initiator,
                    _ if v.eq_str("responder") => EndedBy::Responder,
                    _ => return Err(HeaderError::Invalid(name)),
                });
            }
            "status" => {
                let t = n(v)?;
                let code = crate::json::number_u32(t)
                    .ok()
                    .filter(|c| *c <= 0xFFFF)
                    .ok_or(HeaderError::Invalid(name))?;
                self.status = Some(Status(code as u16));
            }
            "message" | "msg" => {
                let Raw::Str(raw) = v else {
                    return Err(HeaderError::WrongType(name));
                };
                let cut = cut_escaped(raw, MAX_MESSAGE);
                let truncated = cut.len() < raw.len();
                if name == "message" {
                    self.message = Some(HeaderStr::Raw(cut));
                    self.message_truncated = truncated;
                } else {
                    self.msg = Some(HeaderStr::Raw(cut));
                    self.msg_truncated = truncated;
                }
            }
            "cacheTime" => self.cache_time = Some(u32_of(n(v)?)?),
            "totalCount" => self.total_count = Some(u32_of(n(v)?)?),
            _ => {}
        }
        Ok(())
    }

    fn check(&self, kind: HeaderKind, off: bool, lim: bool) -> Result<(), HeaderError> {
        if off != lim {
            return Err(HeaderError::Missing(if off { "limit" } else { "offset" }));
        }
        match kind {
            HeaderKind::Inquiry if self.resource.is_none() => Err(HeaderError::Missing("resource")),
            HeaderKind::Reply if self.status.is_none() => Err(HeaderError::Missing("status")),
            HeaderKind::Subscription => match self.command {
                None => Err(HeaderError::Missing("command")),
                Some(Command::Start) if self.resource.is_none() => {
                    Err(HeaderError::Missing("resource"))
                }
                Some(c) if c != Command::Start && self.subscribe_id.is_none() => {
                    Err(HeaderError::Missing("subscribeId"))
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    /// The encoding of the Property Data: "ASCII" when absent (REQ-PE-ENC-003).
    pub fn encoding(&self) -> EncodingName {
        self.mutual_encoding.unwrap_or(EncodingName::Ascii)
    }

    /// Writes the Header Data of a `kind` message into `out` (REQ-PE-HDR-001, -004).
    pub fn write(&self, kind: HeaderKind, out: &mut [u8]) -> Result<usize, HeaderError> {
        self.check_write(kind)?;
        let mut w = JsonWriter::new(out);
        w.begin_object()?;
        // REQ-PE-HDR-004: the first Property.
        match kind {
            HeaderKind::Inquiry => self.put_str(&mut w, "resource", self.resource)?,
            HeaderKind::Reply => {
                w.name("status", MAX_KEY)?;
                w.number_u32(u32::from(self.status.map_or(0, |s| s.0)))?;
            }
            HeaderKind::Subscription => {
                w.name("command", MAX_KEY)?;
                w.string(self.command.map_or("", Command::as_str))?;
            }
        }
        if kind != HeaderKind::Inquiry {
            self.put_str(&mut w, "resource", self.resource)?;
        }
        self.put_str(&mut w, "resId", self.res_id)?;
        if let Some(e) = self.mutual_encoding {
            w.name("mutualEncoding", MAX_KEY)?;
            w.string(e.as_str())?;
        }
        self.put_str(&mut w, "mediaType", self.media_type)?;
        for (name, v) in [
            ("offset", self.offset),
            ("limit", self.limit),
            ("cacheTime", self.cache_time),
            ("totalCount", self.total_count),
        ] {
            if let Some(v) = v {
                w.name(name, MAX_KEY)?;
                w.number_u32(v)?;
            }
        }
        if self.set_partial {
            w.name("setPartial", MAX_KEY)?;
            w.boolean(true)?;
        }
        if self.flow_control {
            w.name("flowControl", MAX_KEY)?;
            w.boolean(true)?;
        }
        self.put_str(&mut w, "subscribeId", self.subscribe_id)?;
        if let Some(e) = self.ended_by {
            w.name("endedBy", MAX_KEY)?;
            w.string(e.as_str())?;
        }
        if kind != HeaderKind::Reply {
            if let Some(s) = self.status {
                w.name("status", MAX_KEY)?;
                w.number_u32(u32::from(s.0))?;
            }
        }
        self.put_str(&mut w, "message", self.message)?;
        self.put_str(&mut w, "msg", self.msg)?;
        w.end_object()?;
        Ok(w.finish()?)
    }

    fn put_str(
        &self,
        w: &mut JsonWriter<'_>,
        name: &str,
        v: Option<HeaderStr<'_>>,
    ) -> Result<(), HeaderError> {
        if let Some(v) = v {
            w.name(name, MAX_KEY)?;
            v.write(w)?;
        }
        Ok(())
    }

    /// The writer rules (REQ-PE-HDR-006, -007, -012, -013, REQ-PE-STAT-001, -005,
    /// REQ-PE-SUB-001, -002, -004, REQ-PE-LINK-008, REQ-PE-TXN-006).
    fn check_write(&self, kind: HeaderKind) -> Result<(), HeaderError> {
        // Caller text is checked; received raw text is written back unchanged.
        let caller = |v: &Option<HeaderStr<'a>>| match v {
            Some(HeaderStr::Text(t)) => Some(*t),
            _ => None,
        };
        if let Some(t) = caller(&self.resource) {
            if t.len() > MAX_RESOURCE || !resource_chars_ok(t) {
                return Err(HeaderError::Invalid("resource"));
            }
        }
        if let Some(t) = caller(&self.res_id) {
            if t.len() > MAX_RESOURCE || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err(HeaderError::Invalid("resId"));
            }
        }
        if let Some(t) = caller(&self.media_type) {
            if t.len() > MAX_MEDIA_TYPE || !media_type_ok(t) {
                return Err(HeaderError::Invalid("mediaType"));
            }
        }
        if let Some(t) = caller(&self.subscribe_id) {
            if t.is_empty()
                || t.len() > MAX_SUBSCRIBE_ID_WRITE
                || !t
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(HeaderError::Invalid("subscribeId"));
            }
        }
        if self.offset.is_some() != self.limit.is_some() {
            return Err(HeaderError::Missing(if self.offset.is_some() {
                "limit"
            } else {
                "offset"
            }));
        }
        if self.limit == Some(0) {
            return Err(HeaderError::Invalid("limit"));
        }
        for (name, v) in [("message", &self.message), ("msg", &self.msg)] {
            if v.is_some_and(|v| v.escaped_len() > MAX_MESSAGE) {
                return Err(HeaderError::Invalid(name));
            }
        }
        if self.status.is_some_and(|s| !s.is_known()) {
            return Err(HeaderError::Invalid("status"));
        }
        if self.ended_by.is_some() && self.command != Some(Command::End) {
            return Err(HeaderError::Invalid("endedBy"));
        }
        match kind {
            HeaderKind::Inquiry if self.resource.is_none() => Err(HeaderError::Missing("resource")),
            HeaderKind::Reply if self.status.is_none() => Err(HeaderError::Missing("status")),
            HeaderKind::Subscription => match self.command {
                None => Err(HeaderError::Missing("command")),
                Some(Command::Start) if self.resource.is_none() => {
                    Err(HeaderError::Missing("resource"))
                }
                Some(Command::End) if self.ended_by.is_none() => {
                    Err(HeaderError::Missing("endedBy"))
                }
                Some(c) if c != Command::Start && self.subscribe_id.is_none() => {
                    Err(HeaderError::Missing("subscribeId"))
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }
}

/// The Header Property names that the parser knows.
/// The known Property name equal to the bytes of `name`.
fn known_bytes(name: &[u8]) -> Option<&'static str> {
    KNOWN.iter().copied().find(|k| k.as_bytes() == name)
}

#[cfg(test)]
fn known(name: &str) -> Option<&'static str> {
    known_bytes(name.as_bytes())
}

const KNOWN: [&str; 16] = [
    "resource",
    "resId",
    "mutualEncoding",
    "mediaType",
    "offset",
    "limit",
    "setPartial",
    "flowControl",
    "command",
    "subscribeId",
    "endedBy",
    "status",
    "message",
    "msg",
    "cacheTime",
    "totalCount",
];

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::string::String;
    use std::vec;

    fn s(v: &Option<HeaderStr<'_>>) -> String {
        v.map(|v| v.chars().collect()).unwrap_or_default()
    }

    #[test]
    fn parse_inquiry_any_order_and_skip_unknown() {
        // M2-103 Table 14 and 19; REQ-PE-HDR-002 to -004, -006, -007, -009, -013,
        // REQ-PE-TXN-001, -002, REQ-PE-ENC-003, REQ-PE-ENC-005.
        let h = Header::parse(
            HeaderKind::Inquiry,
            br#"{"resId":"abc_1","x":[1,{"y":null}],"resource":"ProgramList","offset":0,"limit":10,"mutualEncoding":"MCoded7","flowControl":true,"aVeryLongUnknownPropertyName":1}"#,
        )
        .unwrap();
        assert_eq!(s(&h.resource), "ProgramList");
        assert_eq!(s(&h.res_id), "abc_1");
        assert_eq!((h.offset, h.limit), (Some(0), Some(10)));
        assert_eq!(
            h.mutual_encoding,
            Some(EncodingName::Mcoded7),
            "no case (REQ-PE-ENC-005)"
        );
        assert!(h.flow_control && !h.set_partial);
        let plain = Header::parse(HeaderKind::Inquiry, br#"{"resource":"DeviceInfo"}"#).unwrap();
        assert_eq!(plain.encoding(), EncodingName::Ascii, "REQ-PE-ENC-003");
        assert!(!plain.flow_control, "REQ-PE-HDR-009 default false");
    }

    #[test]
    fn parse_errors() {
        // REQ-PE-HDR-002, -006, -009, -011, -013, REQ-PE-TXN-002, -006, REQ-PE-STAT-001.
        let bad: [(&[u8], HeaderKind); 14] = [
            (br#"{"resId":"a"}"#, HeaderKind::Inquiry),
            (br#"{"resource":""}"#, HeaderKind::Inquiry),
            (
                br#"{"resource":"0123456789012345678901234567890123456"}"#,
                HeaderKind::Inquiry,
            ),
            (br#"{"resource":["a"]}"#, HeaderKind::Inquiry),
            (
                br#"{"resource":"a","flowControl":"yes"}"#,
                HeaderKind::Inquiry,
            ),
            (br#"{"resource":"a","setPartial":1}"#, HeaderKind::Inquiry),
            (br#"{"resource":"a","offset":1}"#, HeaderKind::Inquiry),
            (
                br#"{"resource":"a","offset":1,"limit":0}"#,
                HeaderKind::Inquiry,
            ),
            (
                br#"{"resource":"a","offset":-1,"limit":2}"#,
                HeaderKind::Inquiry,
            ),
            (br#"{"status":200,"cacheTime":1.5}"#, HeaderKind::Reply),
            (
                br#"{"status":200,"totalCount":4294967296}"#,
                HeaderKind::Reply,
            ),
            (br#"{"message":"x"}"#, HeaderKind::Reply),
            (br#"{"status":"200"}"#, HeaderKind::Reply),
            (br#"[1]"#, HeaderKind::Reply),
        ];
        for (b, k) in bad {
            assert!(
                Header::parse(k, b).is_err(),
                "{}",
                core::str::from_utf8(b).unwrap()
            );
        }
    }

    #[test]
    fn reply_status_message_and_classes() {
        // REQ-PE-STAT-001 to -003, -005; REQ-PE-HDR-011.
        let h = Header::parse(
            HeaderKind::Reply,
            br#"{"status":404,"message":"Not \u00e9 found","cacheTime":60}"#,
        )
        .unwrap();
        assert_eq!(h.status, Some(Status::NOT_FOUND));
        assert_eq!(s(&h.message), "Not é found");
        assert_eq!(h.cache_time, Some(60));
        assert_eq!(Status(200).class(), StatusClass::Success);
        assert_eq!(Status(343).class(), StatusClass::Redirection);
        assert_eq!(Status(415).class(), StatusClass::ClientError);
        assert_eq!(Status(500).class(), StatusClass::ServerError);
        assert_eq!(Status(100).class(), StatusClass::Unknown);
        assert!(Status(445).is_known() && !Status(444).is_known());
        let unknown = Header::parse(HeaderKind::Reply, br#"{"status":299}"#).unwrap();
        assert_eq!(
            unknown.status,
            Some(Status(299)),
            "an unknown code is given with its value"
        );
    }

    #[test]
    fn long_message_is_cut_and_marked() {
        // DEC-010 D-4; REQ-PE-STAT-005, REQ-PE-LINK-008.
        let mut text = String::from(r#"{"status":200,"msg":""#);
        for _ in 0..100 {
            text.push_str("ab\\u00e9");
        }
        text.push_str(r#"","x":1}"#);
        let h = Header::parse(HeaderKind::Reply, text.as_bytes()).unwrap();
        assert!(h.msg_truncated);
        // REQ-PE-LINK-008: the cut is at 512 escaped bytes exactly (64 groups of 8).
        assert_eq!(s(&h.msg), "abé".repeat(64));
        let HeaderStr::Raw(raw) = h.msg.unwrap() else {
            panic!()
        };
        assert!(raw.len() <= MAX_MESSAGE);
        assert_eq!(raw.len() % 8, 0, "cut at an escape boundary");
        assert_eq!(h.status, Some(Status::OK), "the other Properties are kept");
    }

    #[test]
    fn subscription_headers() {
        // REQ-PE-SUB-001, -002, -004 (parser part).
        let h = Header::parse(
            HeaderKind::Subscription,
            br#"{"command":"end","subscribeId":"sub138047","endedBy":"responder"}"#,
        )
        .unwrap();
        assert_eq!(h.command, Some(Command::End));
        assert_eq!(
            s(&h.subscribe_id),
            "sub138047",
            "9 characters from the M2-103 examples"
        );
        assert_eq!(h.ended_by, Some(EndedBy::Responder));
        for b in [
            &br#"{"command":"partial"}"#[..],
            br#"{"command":"stop","subscribeId":"a"}"#,
            br#"{"command":"start"}"#,
            br#"{"command":"End","subscribeId":"a"}"#,
            br#"{"command":"end","subscribeId":"a","endedBy":"me"}"#,
            br#"{"command":"full","subscribeId":"01234567890123456"}"#,
        ] {
            assert!(
                Header::parse(HeaderKind::Subscription, b).is_err(),
                "{}",
                core::str::from_utf8(b).unwrap()
            );
        }
        assert!(
            Header::parse(
                HeaderKind::Subscription,
                br#"{"command":"end","subscribeId":"a"}"#
            )
            .is_ok(),
            "no endedBy"
        );
    }

    fn write(h: &Header<'_>, k: HeaderKind) -> Result<String, HeaderError> {
        let mut buf = [0u8; 700];
        let n = h.write(k, &mut buf)?;
        Ok(String::from_utf8(buf[..n].to_vec()).unwrap())
    }

    #[test]
    fn writer_first_property_and_round_trip() {
        // REQ-PE-HDR-001, -004, REQ-PE-TXN-002, REQ-PE-SUB-002, -004.
        let h = Header {
            res_id: Some(HeaderStr::Text("x1")),
            resource: Some(HeaderStr::Text("X-Custom")),
            set_partial: true,
            mutual_encoding: Some(EncodingName::Mcoded7),
            ..Default::default()
        };
        let out = write(&h, HeaderKind::Inquiry).unwrap();
        assert_eq!(
            out,
            r#"{"resource":"X-Custom","resId":"x1","mutualEncoding":"Mcoded7","setPartial":true}"#
        );
        let back = Header::parse(HeaderKind::Inquiry, out.as_bytes()).unwrap();
        assert_eq!(s(&back.resource), "X-Custom");
        assert!(back.set_partial);
        let r = Header {
            status: Some(Status::OK),
            message: Some(HeaderStr::Text("ok ♪")),
            ..Default::default()
        };
        assert_eq!(
            write(&r, HeaderKind::Reply).unwrap(),
            r#"{"status":200,"message":"ok \u266a"}"#
        );
        let e = Header {
            command: Some(Command::End),
            subscribe_id: Some(HeaderStr::Text("sub1")),
            ended_by: Some(EndedBy::Initiator),
            ..Default::default()
        };
        assert_eq!(
            write(&e, HeaderKind::Subscription).unwrap(),
            r#"{"command":"end","subscribeId":"sub1","endedBy":"initiator"}"#
        );
    }

    #[test]
    fn writer_rules() {
        // REQ-PE-HDR-006, -007, -012, -013, REQ-PE-STAT-001, -005, REQ-PE-SUB-001,
        // REQ-PE-LINK-008, REQ-PE-TXN-006.
        let base = Header {
            resource: Some(HeaderStr::Text("Res")),
            ..Default::default()
        };
        let long: String = core::iter::repeat_n('a', 37).collect();
        let msg: String = core::iter::repeat_n('\n', 257).collect();
        let cases: [Header<'_>; 10] = [
            Header {
                resource: Some(HeaderStr::Text("Re-s")),
                ..base
            },
            Header {
                resource: Some(HeaderStr::Text(&long)),
                ..base
            },
            Header {
                res_id: Some(HeaderStr::Text("a-b")),
                ..base
            },
            Header {
                media_type: Some(HeaderStr::Text("text")),
                ..base
            },
            Header {
                offset: Some(1),
                ..base
            },
            Header {
                offset: Some(1),
                limit: Some(0),
                ..base
            },
            Header {
                subscribe_id: Some(HeaderStr::Text("sub138047")),
                ..base
            },
            Header {
                message: Some(HeaderStr::Text(&msg)),
                ..base
            },
            Header {
                status: Some(Status(299)),
                ..base
            },
            Header {
                resource: None,
                ..base
            },
        ];
        for (i, h) in cases.iter().enumerate() {
            assert!(write(h, HeaderKind::Inquiry).is_err(), "case {i}");
        }
        let ok = Header {
            media_type: Some(HeaderStr::Text("image/png")),
            offset: Some(0),
            limit: Some(5),
            ..base
        };
        assert!(write(&ok, HeaderKind::Inquiry).is_ok());
        // REQ-PE-TXN-006: "limit" with no "offset".
        let no_offset = Header {
            limit: Some(5),
            ..base
        };
        assert!(write(&no_offset, HeaderKind::Inquiry).is_err());
        // REQ-PE-HDR-011: "cacheTime" at its largest value.
        let cached = Header {
            status: Some(Status::OK),
            cache_time: Some(u32::MAX),
            ..Default::default()
        };
        assert_eq!(
            write(&cached, HeaderKind::Reply).as_deref(),
            Ok(r#"{"status":200,"cacheTime":4294967295}"#)
        );
        assert_eq!(
            write(&Header::default(), HeaderKind::Reply),
            Err(HeaderError::Missing("status"))
        );
    }

    #[test]
    fn parse_errors_name_the_property() {
        // REQ-PE-HDR-002 (object or null value), -006, -007, -011, -012, REQ-PE-STAT-001,
        // REQ-PE-SUB-001, -002: the exact error of each refused value, and the largest value
        // that is accepted.
        let r37 = "r".repeat(37);
        let m76 = std::format!("application/{}", "x".repeat(64));
        let m75 = std::format!("application/{}", "x".repeat(63));
        let cases: [(String, HeaderKind, HeaderError); 13] = [
            (
                r#"{"resource":{}}"#.into(),
                HeaderKind::Inquiry,
                HeaderError::WrongType("resource"),
            ),
            (
                r#"{"resource":null}"#.into(),
                HeaderKind::Inquiry,
                HeaderError::WrongType("resource"),
            ),
            (
                r#"{"resource":""}"#.into(),
                HeaderKind::Inquiry,
                HeaderError::Invalid("resource"),
            ),
            (
                std::format!(r#"{{"resource":"{r37}"}}"#),
                HeaderKind::Inquiry,
                HeaderError::Invalid("resource"),
            ),
            (
                std::format!(r#"{{"resource":"a","resId":"{r37}"}}"#),
                HeaderKind::Inquiry,
                HeaderError::Invalid("resId"),
            ),
            (
                std::format!(r#"{{"resource":"a","mediaType":"{m76}"}}"#),
                HeaderKind::Inquiry,
                HeaderError::Invalid("mediaType"),
            ),
            (
                r#"{"status":200,"cacheTime":1e3}"#.into(),
                HeaderKind::Reply,
                HeaderError::Invalid("cacheTime"),
            ),
            (
                r#"{"status":200,"cacheTime":-1}"#.into(),
                HeaderKind::Reply,
                HeaderError::Invalid("cacheTime"),
            ),
            (
                r#"{"status":200,"cacheTime":4294967296}"#.into(),
                HeaderKind::Reply,
                HeaderError::Invalid("cacheTime"),
            ),
            (
                r#"{"status":65536}"#.into(),
                HeaderKind::Reply,
                HeaderError::Invalid("status"),
            ),
            (
                r#"{"command":"full","subscribeId":""}"#.into(),
                HeaderKind::Subscription,
                HeaderError::Invalid("subscribeId"),
            ),
            (
                r#"{"subscribeId":"a"}"#.into(),
                HeaderKind::Subscription,
                HeaderError::Missing("command"),
            ),
            (
                r#"{"command":"notify"}"#.into(),
                HeaderKind::Subscription,
                HeaderError::Missing("subscribeId"),
            ),
        ];
        for (text, kind, want) in &cases {
            assert_eq!(Header::parse(*kind, text.as_bytes()), Err(*want), "{text}");
        }
        // The limits are accepted; the parser accepts any characters (REQ-PE-HDR-006, -007,
        // REQ-PE-SUB-001).
        let r36 = "é".repeat(35) + "-";
        let ok = std::format!(
            r#"{{"resource":"Re-s é","resId":"{r36}","mediaType":"{m75}","setPartial":false}}"#
        );
        let h = Header::parse(HeaderKind::Inquiry, ok.as_bytes()).unwrap();
        assert_eq!(
            (s(&h.resource), s(&h.res_id), s(&h.media_type)),
            (String::from("Re-s é"), r36.clone(), m75.clone())
        );
        // REQ-PE-TXN-001: "setPartial":false is a Full SET.
        assert!(!h.set_partial);
        let h = Header::parse(
            HeaderKind::Reply,
            br#"{"status":65535,"cacheTime":4294967295}"#,
        )
        .unwrap();
        assert_eq!(
            (h.status, h.cache_time),
            (Some(Status(65535)), Some(u32::MAX))
        );
        let h = Header::parse(
            HeaderKind::Subscription,
            "{\"command\":\"full\",\"subscribeId\":\"Ab-\u{e9} 0123456789x\"}".as_bytes(),
        )
        .unwrap();
        assert_eq!(s(&h.subscribe_id), "Ab-\u{e9} 0123456789x", "16 characters");
        // REQ-PE-HDR-013, REQ-PE-TXN-006: "offset", "limit" and "totalCount" are integers in
        // their ranges, and "offset" and "limit" come together.
        for (text, kind, want) in [
            (
                r#"{"resource":"a","offset":1.5,"limit":2}"#,
                HeaderKind::Inquiry,
                HeaderError::Invalid("offset"),
            ),
            (
                r#"{"resource":"a","offset":1e2,"limit":2}"#,
                HeaderKind::Inquiry,
                HeaderError::Invalid("offset"),
            ),
            (
                r#"{"resource":"a","offset":4294967296,"limit":2}"#,
                HeaderKind::Inquiry,
                HeaderError::Invalid("offset"),
            ),
            (
                r#"{"resource":"a","offset":0,"limit":2.5}"#,
                HeaderKind::Inquiry,
                HeaderError::Invalid("limit"),
            ),
            (
                r#"{"resource":"a","offset":0,"limit":-2}"#,
                HeaderKind::Inquiry,
                HeaderError::Invalid("limit"),
            ),
            (
                r#"{"resource":"a","offset":0,"limit":4294967296}"#,
                HeaderKind::Inquiry,
                HeaderError::Invalid("limit"),
            ),
            (
                r#"{"status":200,"totalCount":1.5}"#,
                HeaderKind::Reply,
                HeaderError::Invalid("totalCount"),
            ),
            (
                r#"{"status":200,"totalCount":-1}"#,
                HeaderKind::Reply,
                HeaderError::Invalid("totalCount"),
            ),
            (
                r#"{"resource":"a","limit":2}"#,
                HeaderKind::Inquiry,
                HeaderError::Missing("offset"),
            ),
        ] {
            assert_eq!(Header::parse(kind, text.as_bytes()), Err(want), "{text}");
        }
        let h = Header::parse(
            HeaderKind::Inquiry,
            br#"{"resource":"a","offset":4294967295,"limit":4294967295}"#,
        )
        .unwrap();
        assert_eq!((h.offset, h.limit), (Some(u32::MAX), Some(u32::MAX)));
        // REQ-PE-HDR-005: JSON whitespace around names, values and separators.
        let h = Header::parse(
            HeaderKind::Inquiry,
            b"{ \"resource\" :\n\"A\"\t,\r\"resId\": \"b\" }",
        )
        .unwrap();
        assert_eq!(
            (s(&h.resource), s(&h.res_id)),
            (String::from("A"), String::from("b"))
        );
        // REQ-PE-JSON-001: JSON that is not valid is a parse error of the header parser.
        for bad in [
            &br#"{"a" 1}"#[..],
            br#"{} {}"#,
            br#"{"resource":"a""#,
            br#"{"resource":}"#,
        ] {
            assert!(
                matches!(
                    Header::parse(HeaderKind::Inquiry, bad),
                    Err(HeaderError::Json(_))
                ),
                "{}",
                core::str::from_utf8(bad).unwrap()
            );
        }
    }

    #[test]
    fn writer_errors_name_the_property() {
        // REQ-PE-HDR-006, -007, -012, REQ-PE-STAT-001, -005, REQ-PE-SUB-001, REQ-PE-LINK-008:
        // the exact error of each refused caller value, and the largest value that is written.
        let base = Header {
            resource: Some(HeaderStr::Text("Res")),
            ..Default::default()
        };
        let a36 = "a".repeat(36);
        let a37 = "a".repeat(37);
        let m75 = std::format!("application/{}", "x".repeat(63));
        let m76 = std::format!("application/{}", "x".repeat(64));
        // 512 and 513 bytes after the escape step: "\n" is 2 bytes, "é" 6 ("é").
        let nl256 = "\n".repeat(256);
        let e85 = std::format!("a{}a", "é".repeat(85));
        let e85b = std::format!("ab{}a", "é".repeat(85));
        let refused: [(Header<'_>, HeaderError); 13] = [
            (
                Header {
                    resource: Some(HeaderStr::Text("")),
                    ..base
                },
                HeaderError::Invalid("resource"),
            ),
            (
                Header {
                    resource: Some(HeaderStr::Text(&a37)),
                    ..base
                },
                HeaderError::Invalid("resource"),
            ),
            (
                Header {
                    res_id: Some(HeaderStr::Text(&a37)),
                    ..base
                },
                HeaderError::Invalid("resId"),
            ),
            (
                Header {
                    media_type: Some(HeaderStr::Text("")),
                    ..base
                },
                HeaderError::Invalid("mediaType"),
            ),
            (
                Header {
                    media_type: Some(HeaderStr::Text(&m76)),
                    ..base
                },
                HeaderError::Invalid("mediaType"),
            ),
            (
                Header {
                    subscribe_id: Some(HeaderStr::Text("")),
                    ..base
                },
                HeaderError::Invalid("subscribeId"),
            ),
            (
                Header {
                    subscribe_id: Some(HeaderStr::Text("Ab")),
                    ..base
                },
                HeaderError::Invalid("subscribeId"),
            ),
            (
                Header {
                    subscribe_id: Some(HeaderStr::Text("a-b")),
                    ..base
                },
                HeaderError::Invalid("subscribeId"),
            ),
            (
                Header {
                    message: Some(HeaderStr::Text(&e85b)),
                    ..base
                },
                HeaderError::Invalid("message"),
            ),
            (
                Header {
                    msg: Some(HeaderStr::Text(&(nl256.clone() + "x"))),
                    ..base
                },
                HeaderError::Invalid("msg"),
            ),
            (
                Header {
                    msg: Some(HeaderStr::Text(&e85b)),
                    ..base
                },
                HeaderError::Invalid("msg"),
            ),
            (
                Header {
                    status: Some(Status(202)),
                    ..base
                },
                HeaderError::Invalid("status"),
            ),
            (
                Header {
                    status: Some(Status(444)),
                    ..base
                },
                HeaderError::Invalid("status"),
            ),
        ];
        for (i, (h, want)) in refused.iter().enumerate() {
            assert_eq!(write(h, HeaderKind::Inquiry), Err(*want), "case {i}");
        }
        let ok = Header {
            resource: Some(HeaderStr::Text(&a36)),
            res_id: Some(HeaderStr::Text("Ab_9")),
            media_type: Some(HeaderStr::Text(&m75)),
            subscribe_id: Some(HeaderStr::Text("abc_1234")),
            message: Some(HeaderStr::Text("aé")),
            ..Default::default()
        };
        // "message" and "msg" of 512 bytes after the escape step are written.
        for (h, empty) in [
            (
                Header {
                    message: Some(HeaderStr::Text(&e85)),
                    ..base
                },
                r#"{"resource":"Res","message":""}"#,
            ),
            (
                Header {
                    msg: Some(HeaderStr::Text(&nl256)),
                    ..base
                },
                r#"{"resource":"Res","msg":""}"#,
            ),
        ] {
            assert_eq!(
                write(&h, HeaderKind::Inquiry).unwrap().len(),
                empty.len() + 512
            );
        }
        let out = write(&ok, HeaderKind::Inquiry).unwrap();
        assert_eq!(
            out,
            std::format!(
                r#"{{"resource":"{a36}","resId":"Ab_9","mediaType":"{m75}","subscribeId":"abc_1234","message":"a\u00e9"}}"#
            )
        );
        // REQ-PE-TXN-001: a Full SET has no "setPartial".
        assert_eq!(
            write(&base, HeaderKind::Inquiry).unwrap(),
            r#"{"resource":"Res"}"#
        );
    }

    #[test]
    fn status_table_and_classes() {
        // REQ-PE-STAT-002: the 15 codes of Table 15 are known, every other code is not;
        // REQ-PE-STAT-003: the class boundaries.
        let table = [
            200u16, 201, 341, 342, 343, 400, 403, 404, 405, 406, 407, 413, 415, 445, 500,
        ];
        for c in 0..=1000u16 {
            assert_eq!(Status(c).is_known(), table.contains(&c), "{c}");
        }
        assert!(!Status(u16::MAX).is_known());
        let consts = [
            Status::OK,
            Status::ACCEPTED,
            Status::UNAVAILABLE,
            Status::BAD_DATA,
            Status::TOO_MANY_REQUESTS,
            Status::BAD_REQUEST,
            Status::FORBIDDEN,
            Status::NOT_FOUND,
            Status::NOT_ALLOWED,
            Status::FLOW_CONTROL_NOT_SUPPORTED,
            Status::FLOW_CONTROL_REQUIRED,
            Status::PAYLOAD_TOO_LARGE,
            Status::UNSUPPORTED_MEDIA,
            Status::INVALID_VERSION,
            Status::INTERNAL_ERROR,
        ];
        assert_eq!(consts.map(|s| s.0), table);
        for (c, class) in [
            (0, StatusClass::Unknown),
            (199, StatusClass::Unknown),
            (200, StatusClass::Success),
            (299, StatusClass::Success),
            (300, StatusClass::Redirection),
            (399, StatusClass::Redirection),
            (400, StatusClass::ClientError),
            (499, StatusClass::ClientError),
            (500, StatusClass::ServerError),
            (599, StatusClass::ServerError),
            (600, StatusClass::Unknown),
            (65535, StatusClass::Unknown),
        ] {
            assert_eq!(Status(c).class(), class, "{c}");
        }
    }

    #[test]
    fn encoding_names_spelling_and_case() {
        // REQ-PE-ENC-005: the writer spelling of Table 12; the reader compares with no case.
        assert_eq!(
            [
                EncodingName::Ascii,
                EncodingName::Mcoded7,
                EncodingName::ZlibMcoded7
            ]
            .map(EncodingName::as_str),
            ["ASCII", "Mcoded7", "zlib+Mcoded7"]
        );
        for (text, want) in [
            ("ascii", Some(EncodingName::Ascii)),
            ("ASCII", Some(EncodingName::Ascii)),
            ("MCODED7", Some(EncodingName::Mcoded7)),
            ("ZLIB+MCODED7", Some(EncodingName::ZlibMcoded7)),
            ("Zlib+mcoded7", Some(EncodingName::ZlibMcoded7)),
            ("zlib", None),
            ("Mcoded", None),
            ("ASCII ", None),
        ] {
            assert_eq!(EncodingName::parse(&HeaderStr::Text(text)), want, "{text}");
        }
        let h = Header::parse(
            HeaderKind::Inquiry,
            br#"{"resource":"a","mutualEncoding":"ZLIB+mcoded7"}"#,
        )
        .unwrap();
        assert_eq!(h.mutual_encoding, Some(EncodingName::ZlibMcoded7));
        assert_eq!(
            write(&h, HeaderKind::Inquiry).unwrap(),
            r#"{"resource":"a","mutualEncoding":"zlib+Mcoded7"}"#
        );
    }

    #[test]
    fn long_message_is_cut_at_512_bytes() {
        // DEC-010 D-4, REQ-PE-STAT-005: a "message" of more than 512 escaped bytes is cut at the
        // last escape or character boundary at or before 512, marked, and the other Properties
        // are kept; a "message" of 512 bytes is not cut.
        let reply =
            |body: &str| std::format!(r#"{{"status":404,"message":"{body}","cacheTime":7}}"#);
        // 8-byte groups: the cut is at 512 exactly.
        let t = reply(&"ab\\u00e9".repeat(100));
        let h = Header::parse(HeaderKind::Reply, t.as_bytes()).unwrap();
        assert_eq!(s(&h.message), "abé".repeat(64));
        assert!(h.message_truncated && !h.msg_truncated);
        assert_eq!((h.status, h.cache_time), (Some(Status::NOT_FOUND), Some(7)));
        // An escape across the 512 boundary: cut before it (1 + 85 x 6 = 511).
        let t = reply(&std::format!("a{}", "\\u00e9".repeat(90)));
        let h = Header::parse(HeaderKind::Reply, t.as_bytes()).unwrap();
        assert_eq!(s(&h.message), std::format!("a{}", "é".repeat(85)));
        assert!(h.message_truncated);
        // A UTF-8 character across the boundary: cut before it (1 + 255 x 2 = 511).
        let t = reply(&std::format!("a{}", "é".repeat(256)));
        let h = Header::parse(HeaderKind::Reply, t.as_bytes()).unwrap();
        assert_eq!(s(&h.message), std::format!("a{}", "é".repeat(255)));
        assert!(h.message_truncated);
        // 512 bytes: not cut.
        let t = reply(&"x".repeat(512));
        let h = Header::parse(HeaderKind::Reply, t.as_bytes()).unwrap();
        assert_eq!((s(&h.message).len(), h.message_truncated), (512, false));
    }

    #[test]
    fn prop_parse_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..80))| {
            let _ = Header::parse(HeaderKind::Reply, &bytes);
            let _ = Header::parse(HeaderKind::Inquiry, &bytes);
        });
    }

    /// Headers from known and unknown names, values of every JSON type, escapes and white space.
    fn header_bytes() -> impl Strategy<Value = std::vec::Vec<u8>> {
        let key = prop_oneof![
            Just("resource"),
            Just("resId"),
            Just("offset"),
            Just("limit"),
            Just("status"),
            Just("mutualEncoding"),
            Just("setPartial"),
            Just("command"),
            Just("subscribeId"),
            Just("x"),
            Just("res\\u0049d"),
            Just("aaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        ];
        let val = prop_oneof![
            Just("\"X-State\""),
            Just("\"ASCII\""),
            Just("\"start\""),
            Just("12"),
            Just("-0.5e3"),
            Just("true"),
            Just("null"),
            Just("{\"a\":[1,2,{\"b\":\"c\\\"d\"}]}"),
            Just("\"\\ud800x\""),
            Just("\"caf\u{e9}\""),
            Just("[]"),
        ];
        let space = prop_oneof![Just(""), Just(" "), Just("\n\t")];
        proptest::collection::vec((key, val, space), 0..6).prop_map(|m| {
            let body: std::vec::Vec<std::string::String> = m
                .iter()
                .map(|(k, v, w)| std::format!("{w}\"{k}\"{w}:{w}{v}{w}"))
                .collect();
            std::format!("{{{}}}", body.join(",")).into_bytes()
        })
    }

    #[test]
    fn prop_header_parse_equals_streaming_parse() {
        // REQ-PE-HDR-014, OPT-020: for headers with ASCII names, cut or with one byte changed, the
        // walk in place gives the header of the streaming walk, or an error with the same reply
        // status. The one allowed difference: a JSON error that the streaming walk found only
        // after a Property error of a value cut at the end.
        let unsupported =
            |e: &HeaderError| matches!(e, HeaderError::Invalid("mutualEncoding" | "mediaType"));
        proptest!(ProptestConfig::with_cases(30_000), |(
            h in header_bytes(),
            cut in 0usize..200,
            b in 0u8..0x80,
            mode in 0u8..3,
        )| {
            let mut m = h.clone();
            if !m.is_empty() {
                let i = cut % m.len();
                match mode {
                    0 => {}
                    1 => m[i] = b,
                    _ => m.truncate(i),
                }
            }
            for kind in [HeaderKind::Inquiry, HeaderKind::Reply, HeaderKind::Subscription] {
                let (new, old) = (Header::parse(kind, &m), Header::parse_streaming(kind, &m));
                match (&new, &old) {
                    (Err(HeaderError::Json(_)), Err(_)) => {}
                    (Err(x), Err(y)) => prop_assert_eq!(unsupported(x), unsupported(y)),
                    _ => prop_assert_eq!(&new, &old),
                }
            }
        });
    }

    #[test]
    fn damaged_names_and_cut_headers() {
        // REQ-PE-HDR-014: a name with an invalid UTF-8 byte is not a known Property (the
        // streaming walk took "resource\xFF" as "resource"), and a header cut inside a string is
        // a JSON error, so it gets 400, not 415 (the streaming walk gave Invalid("mutualEncoding")).
        let damaged = b"{\"resource\xFF\":\"A\"}";
        assert_eq!(
            Header::parse(HeaderKind::Inquiry, damaged),
            Err(HeaderError::Missing("resource"))
        );
        assert!(Header::parse_streaming(HeaderKind::Inquiry, damaged).is_ok());
        let cut = b"{\"resource\":\"A\",\"mutualEncoding\":\"";
        assert_eq!(
            Header::parse(HeaderKind::Inquiry, cut),
            Err(HeaderError::Json(JsonError::Syntax))
        );
        assert_eq!(
            Header::parse_streaming(HeaderKind::Inquiry, cut),
            Err(HeaderError::Invalid("mutualEncoding"))
        );
    }

    // ----- limits found by mutation testing -----

    fn raw_chars(raw: &[u8]) -> String {
        HeaderStr::Raw(raw).chars().collect()
    }

    #[test]
    fn received_text_decodes_each_escape_and_utf8_length() {
        // REQ-PE-JSON-006: ECMA-404 escapes, UTF-8 of 2, 3 and 4 bytes, surrogate pairs.
        assert_eq!(raw_chars(br#"\"\\\/\b\f\n\r\t"#), "\"\\/\u{8}\u{C}\n\r\t");
        assert_eq!(raw_chars("é€😀".as_bytes()), "é€😀");
        assert_eq!(raw_chars(br#"\ud834\udd1e\u00e9"#), "𝄞é");
        assert_eq!(raw_chars(br#"\ud834x"#), "\u{FFFD}x");
        // 0x7F is ASCII; 0x80 alone is no character.
        assert_eq!(raw_chars(b"\x7f"), "\u{7F}");
        assert_eq!(raw_chars(&[0x80]), "\u{FFFD}");
    }

    #[test]
    fn escaped_length_of_each_class() {
        // Section 6.1.1: 2 for a short escape, 6 for each UTF-16 unit of a \u escape.
        for (t, n) in [
            ("a", 1),
            (" ", 1),
            ("~", 1),
            ("\u{1F}", 6),
            ("\u{7F}", 6),
            ("é", 6),
            ("😀", 12),
            ("\"", 2),
        ] {
            assert_eq!(HeaderStr::Text(t).escaped_len(), n, "{t:?}");
        }
        // REQ-PE-HDR-012: a "message" of MAX_MESSAGE escaped bytes is written.
        let text = "m".repeat(MAX_MESSAGE);
        let mut h = Header {
            status: Some(Status::OK),
            message: Some(HeaderStr::Text(&text)),
            ..Default::default()
        };
        let mut buf = [0u8; 600];
        assert!(h.write(HeaderKind::Reply, &mut buf).is_ok());
        let long = "m".repeat(MAX_MESSAGE + 1);
        h.message = Some(HeaderStr::Text(&long));
        assert_eq!(
            h.write(HeaderKind::Reply, &mut buf),
            Err(HeaderError::Invalid("message"))
        );
    }

    #[test]
    fn cut_keeps_escapes_and_characters_whole() {
        let raw = "\\u00e9\\u00e9".as_bytes();
        assert_eq!(cut_escaped(raw, 8), &raw[..6]);
        assert_eq!(cut_escaped(raw, 12), raw);
        let raw = "aé€😀".as_bytes();
        for (limit, n) in [(2, 1), (3, 3), (5, 3), (6, 6), (9, 6), (10, 10)] {
            assert_eq!(cut_escaped(raw, limit), &raw[..n], "{limit}");
        }
        assert_eq!(cut_escaped(br#"a\nb"#, 2), b"a");
    }

    #[test]
    fn media_type_rules() {
        // RFC 6838 4.2: restricted names of 1 to 127 characters, the first one alphanumeric.
        let long = "a".repeat(128);
        for (t, ok) in [
            ("text/plain", true),
            ("application/vnd.a+json", true),
            ("/plain", false),
            ("text/", false),
            ("-text/plain", false),
            ("text/-plain", false),
            ("te xt/plain", false),
            ("textplain", false),
        ] {
            assert_eq!(media_type_ok(t), ok, "{t}");
        }
        assert!(!media_type_ok(&std::format!("{long}/x")));
        assert!(media_type_ok(&std::format!("{}/x", &long[..127])));
    }

    #[test]
    fn subscription_writer_needs_its_properties() {
        // REQ-PE-SUB-003, -005, -008: "command"; "resource" for a start; "endedBy" for an
        // end; "subscribeId" for the other commands.
        let mut buf = [0u8; 200];
        let w = |h: Header<'_>, buf: &mut [u8]| h.write(HeaderKind::Subscription, buf).map(drop);
        let base = Header::default();
        assert_eq!(w(base, &mut buf), Err(HeaderError::Missing("command")));
        let start = Header {
            command: Some(Command::Start),
            ..base
        };
        assert_eq!(w(start, &mut buf), Err(HeaderError::Missing("resource")));
        let end = Header {
            command: Some(Command::End),
            subscribe_id: Some(HeaderStr::Text("s1")),
            ..base
        };
        assert_eq!(w(end, &mut buf), Err(HeaderError::Missing("endedBy")));
        let full = Header {
            command: Some(Command::Full),
            ..base
        };
        assert_eq!(w(full, &mut buf), Err(HeaderError::Missing("subscribeId")));
        let full = Header {
            subscribe_id: Some(HeaderStr::Text("s1")),
            ..full
        };
        assert_eq!(w(full, &mut buf), Ok(()));
    }
}
