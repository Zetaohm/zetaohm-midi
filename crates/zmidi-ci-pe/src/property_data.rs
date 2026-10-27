//! Property Data formats that the crate reads and writes (M2-103-UM v1.2 sections 8.7 and 10).
//!
//! - Partial SET (10, Method 2): one JSON object; each key is an RFC 6901 JSON Pointer that
//!   starts with "/", each value a string, a number or a boolean (REQ-PE-TXN-004, -005). The
//!   reader streams the keys and values Chunk by Chunk with a key buffer of `K` bytes
//!   (DEC-007); a longer key gives status 413 (REQ-PE-STAT-011).
//! - Simple Property Resource (8.7): one JSON value, a string, a number or a boolean
//!   (REQ-PE-RES-002).
//! - [`PropertyDataError::status`] gives the reply status of each error.

use crate::header::Status;
use crate::json::{JsonError, JsonReader, JsonWriter, Number, Token, Tokens};

/// Error of the Property Data readers and writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PropertyDataError {
    /// The data is not valid JSON.
    #[error("JSON: {0}")]
    Json(JsonError),
    /// The data is not a JSON object (Partial SET) or not one scalar value (Simple Property).
    #[error("wrong JSON structure")]
    Structure,
    /// A Partial SET key that is not a JSON Pointer starting with "/".
    #[error("key is not a JSON Pointer")]
    InvalidKey,
    /// A key longer than the key buffer.
    #[error("key too long")]
    KeyTooLong,
    /// An object, an array or null where a scalar value is necessary.
    #[error("value is not a string, number or boolean")]
    InvalidValue,
    /// A string longer than the caller buffer, or a full output buffer.
    #[error("buffer overflow")]
    Overflow,
}

impl From<JsonError> for PropertyDataError {
    fn from(e: JsonError) -> Self {
        match e {
            JsonError::Overflow => PropertyDataError::Overflow,
            e => PropertyDataError::Json(e),
        }
    }
}

impl PropertyDataError {
    /// The reply status for this error: 413 for a key or a value larger than the buffers
    /// (REQ-PE-STAT-011), else 400 (REQ-PE-TXN-004, -005).
    pub fn status(self) -> Status {
        match self {
            PropertyDataError::KeyTooLong | PropertyDataError::Overflow => {
                Status::PAYLOAD_TOO_LARGE
            }
            _ => Status::BAD_REQUEST,
        }
    }
}

/// A scalar JSON value: a string, a number or a boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarValue<'a> {
    /// A string, decoded; `truncated` when longer than the reader buffer.
    String { text: &'a str, truncated: bool },
    /// A number, as its text.
    Number(Number<'a>),
    /// A boolean.
    Bool(bool),
}

/// One change of a Partial SET.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialSetItem<'a> {
    /// The JSON Pointer key, decoded.
    pub key: &'a str,
    /// The new value.
    pub value: ScalarValue<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PsState {
    Start,
    Key,
    Value,
    Done,
}

/// Streaming reader of Partial SET Property Data with a key and string buffer of `K` bytes.
#[derive(Debug, Clone)]
pub struct PartialSetReader<const K: usize> {
    json: JsonReader<K>,
    key: [u8; K],
    key_len: usize,
    state: PsState,
}

#[allow(clippy::new_without_default)]
impl<const K: usize> PartialSetReader<K> {
    /// A reader for one Partial SET.
    pub const fn new() -> Self {
        PartialSetReader {
            json: JsonReader::new(),
            key: [0; K],
            key_len: 0,
            state: PsState::Start,
        }
    }

    /// Takes the Property Data of the next Chunk and gives its complete items.
    pub fn push<'r, 'i>(&'r mut self, input: &'i [u8]) -> PartialSetItems<'r, 'i, K> {
        PartialSetItems {
            tokens: self.json.push(input),
            key: &mut self.key,
            key_len: &mut self.key_len,
            state: &mut self.state,
        }
    }

    /// The end of the Property Data: an error when the object is not complete.
    pub fn finish(&mut self) -> Result<(), PropertyDataError> {
        self.json.finish()?;
        if self.state != PsState::Done {
            return Err(PropertyDataError::Structure);
        }
        Ok(())
    }
}

/// The items of one Chunk of a Partial SET. Call [`PartialSetItems::next_item`] until it gives
/// `None`.
#[derive(Debug)]
pub struct PartialSetItems<'r, 'i, const K: usize> {
    tokens: Tokens<'r, 'i, K>,
    key: &'r mut [u8; K],
    key_len: &'r mut usize,
    state: &'r mut PsState,
}

impl<const K: usize> PartialSetItems<'_, '_, K> {
    /// The next key and value, or `None` at the end of the Chunk. An item is valid until the
    /// next call.
    pub fn next_item(&mut self) -> Option<Result<PartialSetItem<'_>, PropertyDataError>> {
        // Names and structure first; a value is read below, so that no borrow crosses the loop.
        while *self.state != PsState::Value {
            let t = match self.tokens.next_token()? {
                Ok(t) => t,
                Err(e) => return Some(Err(e.into())),
            };
            match (*self.state, t) {
                (PsState::Start, Token::BeginObject) => *self.state = PsState::Key,
                (PsState::Key, Token::Name { text, truncated }) => {
                    if truncated || text.len() > K {
                        return Some(Err(PropertyDataError::KeyTooLong));
                    }
                    if !text.starts_with('/') {
                        return Some(Err(PropertyDataError::InvalidKey));
                    }
                    self.key[..text.len()].copy_from_slice(text.as_bytes());
                    *self.key_len = text.len();
                    *self.state = PsState::Value;
                }
                (PsState::Key, Token::EndObject) => *self.state = PsState::Done,
                _ => return Some(Err(PropertyDataError::Structure)),
            }
        }
        let t = match self.tokens.next_token()? {
            Ok(t) => t,
            Err(e) => return Some(Err(e.into())),
        };
        *self.state = PsState::Key;
        let value = match t {
            Token::String { text, truncated } => ScalarValue::String { text, truncated },
            Token::Number(n) => ScalarValue::Number(n),
            Token::Bool(b) => ScalarValue::Bool(b),
            _ => return Some(Err(PropertyDataError::InvalidValue)),
        };
        let key = core::str::from_utf8(&self.key[..*self.key_len]).unwrap_or("");
        Some(Ok(PartialSetItem { key, value }))
    }
}

/// Writer of Partial SET Property Data (REQ-PE-TXN-004, -005).
#[derive(Debug)]
pub struct PartialSetWriter<'b> {
    w: JsonWriter<'b>,
}

impl<'b> PartialSetWriter<'b> {
    /// Starts the object in `out`.
    pub fn new(out: &'b mut [u8]) -> Result<Self, PropertyDataError> {
        let mut w = JsonWriter::new(out);
        w.begin_object()?;
        Ok(PartialSetWriter { w })
    }

    /// Writes one change; the key must be a JSON Pointer that starts with "/".
    pub fn item(&mut self, key: &str, value: ScalarValue<'_>) -> Result<(), PropertyDataError> {
        if !key.starts_with('/') {
            return Err(PropertyDataError::InvalidKey);
        }
        self.w.key(key)?;
        write_scalar(&mut self.w, value)
    }

    /// Ends the object and gives the length.
    pub fn finish(mut self) -> Result<usize, PropertyDataError> {
        self.w.end_object()?;
        Ok(self.w.finish()?)
    }
}

fn write_scalar(w: &mut JsonWriter<'_>, value: ScalarValue<'_>) -> Result<(), PropertyDataError> {
    match value {
        ScalarValue::String {
            truncated: true, ..
        } => return Err(PropertyDataError::InvalidValue),
        ScalarValue::String { text, .. } => w.string(text)?,
        ScalarValue::Number(n) => {
            // The number text was checked by the reader or comes from Number::text.
            if n.text()
                .bytes()
                .any(|b| !matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'))
            {
                return Err(PropertyDataError::InvalidValue);
            }
            w.raw(n.text().as_bytes())?
        }
        ScalarValue::Bool(b) => w.boolean(b)?,
    }
    Ok(())
}

/// Writes the Property Data of a Simple Property Resource (REQ-PE-RES-002).
pub fn write_simple(value: ScalarValue<'_>, out: &mut [u8]) -> Result<usize, PropertyDataError> {
    let mut w = JsonWriter::new(out);
    write_scalar(&mut w, value)?;
    Ok(w.finish()?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimpleKind {
    None,
    String,
    Number,
    Bool(bool),
}

/// Reader of the Property Data of a Simple Property Resource, with a value buffer of `S` bytes
/// (REQ-PE-RES-002).
#[derive(Debug, Clone)]
pub struct SimpleValueReader<const S: usize> {
    json: JsonReader<S>,
    buf: [u8; S],
    len: usize,
    kind: SimpleKind,
    error: Option<PropertyDataError>,
}

#[allow(clippy::new_without_default)]
impl<const S: usize> SimpleValueReader<S> {
    /// A reader for one value.
    pub const fn new() -> Self {
        SimpleValueReader {
            json: JsonReader::new(),
            buf: [0; S],
            len: 0,
            kind: SimpleKind::None,
            error: None,
        }
    }

    /// Takes the Property Data of the next Chunk.
    pub fn push(&mut self, input: &[u8]) -> Result<(), PropertyDataError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let mut tokens = self.json.push(input);
        let mut kept: Option<Result<(SimpleKind, usize), PropertyDataError>> = None;
        let mut tmp = [0u8; S];
        while let Some(t) = tokens.next_token() {
            let r = match t {
                Err(e) => Err(e.into()),
                Ok(Token::String {
                    truncated: true, ..
                }) => Err(PropertyDataError::Overflow),
                Ok(Token::String { text, .. }) => {
                    tmp[..text.len()].copy_from_slice(text.as_bytes());
                    Ok((SimpleKind::String, text.len()))
                }
                Ok(Token::Number(n)) => match tmp.get_mut(..n.text().len()) {
                    Some(d) => {
                        d.copy_from_slice(n.text().as_bytes());
                        Ok((SimpleKind::Number, n.text().len()))
                    }
                    None => Err(PropertyDataError::Overflow),
                },
                Ok(Token::Bool(b)) => Ok((SimpleKind::Bool(b), 0)),
                Ok(_) => Err(PropertyDataError::InvalidValue),
            };
            if r.is_err() || kept.is_some() {
                kept = Some(if r.is_err() {
                    r
                } else {
                    Err(PropertyDataError::Structure)
                });
                break;
            }
            kept = Some(r);
        }
        match kept {
            Some(Ok((kind, n))) => {
                if self.kind != SimpleKind::None {
                    return self.fail(PropertyDataError::Structure);
                }
                self.buf[..n].copy_from_slice(&tmp[..n]);
                self.len = n;
                self.kind = kind;
                Ok(())
            }
            Some(Err(e)) => self.fail(e),
            None => Ok(()),
        }
    }

    fn fail(&mut self, e: PropertyDataError) -> Result<(), PropertyDataError> {
        self.error = Some(e);
        Err(e)
    }

    /// The end of the Property Data: gives the value.
    pub fn finish(&mut self) -> Result<ScalarValue<'_>, PropertyDataError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        // A top-level number ends at the end of the data.
        let mut tmp = [0u8; 32];
        let mut num = None;
        match self.json.finish() {
            Err(e) => return Err(e.into()),
            Ok(Some(Token::Number(n))) => {
                let t = n.text().as_bytes();
                tmp.get_mut(..t.len())
                    .ok_or(PropertyDataError::Overflow)?
                    .copy_from_slice(t);
                num = Some(t.len());
            }
            Ok(Some(_)) => return Err(PropertyDataError::Structure),
            Ok(None) => {}
        }
        if let Some(n) = num {
            if self.kind != SimpleKind::None {
                return Err(PropertyDataError::Structure);
            }
            self.buf
                .get_mut(..n)
                .ok_or(PropertyDataError::Overflow)?
                .copy_from_slice(&tmp[..n]);
            self.len = n;
            self.kind = SimpleKind::Number;
        }
        let text = core::str::from_utf8(&self.buf[..self.len])
            .map_err(|_| PropertyDataError::Structure)?;
        match self.kind {
            SimpleKind::None => Err(PropertyDataError::Structure),
            SimpleKind::String => Ok(ScalarValue::String {
                text,
                truncated: false,
            }),
            SimpleKind::Number => Ok(ScalarValue::Number(crate::json::number(text))),
            SimpleKind::Bool(b) => Ok(ScalarValue::Bool(b)),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::string::{String, ToString};
    use std::vec::Vec;

    #[derive(Debug, PartialEq)]
    enum V {
        S(String),
        N(String),
        B(bool),
    }

    fn own(v: ScalarValue<'_>) -> V {
        match v {
            ScalarValue::String { text, .. } => V::S(text.to_string()),
            ScalarValue::Number(n) => V::N(n.text().to_string()),
            ScalarValue::Bool(b) => V::B(b),
        }
    }

    fn read_partial<const K: usize>(
        pieces: &[&[u8]],
    ) -> Result<Vec<(String, V)>, PropertyDataError> {
        let mut r = PartialSetReader::<K>::new();
        let mut out = Vec::new();
        for p in pieces {
            let mut items = r.push(p);
            while let Some(i) = items.next_item() {
                let i = i?;
                out.push((i.key.to_string(), own(i.value)));
            }
        }
        r.finish()?;
        Ok(out)
    }

    #[test]
    fn partial_set_vectors() {
        // M2-103 Tables 33 and 35; REQ-PE-TXN-004.
        assert_eq!(
            read_partial::<32>(&[br#"{"/lfoSpeed":10}"#]).unwrap(),
            [(String::from("/lfoSpeed"), V::N(String::from("10")))]
        );
        let text: &[u8] = br#"{"/pitchEnvelope/rates/0":80,"/pitchEnvelope/levels/0":60,"/name":"Bright","/on":true}"#;
        let whole = read_partial::<32>(&[text]).unwrap();
        assert_eq!(whole.len(), 4);
        assert_eq!(
            whole[2],
            (String::from("/name"), V::S(String::from("Bright")))
        );
        for cut in 1..text.len() {
            assert_eq!(
                read_partial::<32>(&[&text[..cut], &text[cut..]]).unwrap(),
                whole,
                "cut {cut}"
            );
        }
    }

    #[test]
    fn partial_set_errors_and_status() {
        // REQ-PE-TXN-004, -005, REQ-PE-STAT-011.
        let e = read_partial::<32>(&[br#"{"lfoSpeed":10}"#]).unwrap_err();
        assert_eq!(
            (e, e.status()),
            (PropertyDataError::InvalidKey, Status::BAD_REQUEST)
        );
        for v in [
            &br#"{"/a":{"b":1}}"#[..],
            br#"{"/a":[1]}"#,
            br#"{"/a":null}"#,
        ] {
            let e = read_partial::<32>(&[v]).unwrap_err();
            assert_eq!(
                (e, e.status()),
                (PropertyDataError::InvalidValue, Status::BAD_REQUEST)
            );
        }
        let e = read_partial::<8>(&[br#"{"/averyverylongkey":1}"#]).unwrap_err();
        assert_eq!(
            (e, e.status()),
            (PropertyDataError::KeyTooLong, Status::PAYLOAD_TOO_LARGE)
        );
        assert_eq!(
            read_partial::<32>(&[br#"[1]"#]).unwrap_err(),
            PropertyDataError::Structure
        );
        assert!(
            read_partial::<32>(&[br#"{"/a":1"#]).is_err(),
            "not complete"
        );
    }

    #[test]
    fn partial_set_writer() {
        // REQ-PE-TXN-004, -005.
        let mut buf = [0u8; 64];
        let mut w = PartialSetWriter::new(&mut buf).unwrap();
        w.item("/lfoSpeed", ScalarValue::Number(crate::json::number("10")))
            .unwrap();
        w.item(
            "/name",
            ScalarValue::String {
                text: "a~b",
                truncated: false,
            },
        )
        .unwrap();
        assert_eq!(
            w.item("x", ScalarValue::Bool(true)),
            Err(PropertyDataError::InvalidKey)
        );
        let n = w.finish().unwrap();
        assert_eq!(&buf[..n], br#"{"/lfoSpeed":10,"/name":"a~b"}"#);
    }

    fn read_simple(pieces: &[&[u8]]) -> Result<V, PropertyDataError> {
        let mut r = SimpleValueReader::<16>::new();
        for p in pieces {
            r.push(p)?;
        }
        r.finish().map(own)
    }

    #[test]
    fn simple_property_vectors() {
        // M2-103 Tables 23 to 26; REQ-PE-RES-002.
        assert_eq!(read_simple(&[b"true"]), Ok(V::B(true)));
        assert_eq!(
            read_simple(&[b"\"perfor", b"mance\""]),
            Ok(V::S(String::from("performance")))
        );
        assert_eq!(read_simple(&[b"12", b"7"]), Ok(V::N(String::from("127"))));
        assert_eq!(read_simple(&[b"{}"]), Err(PropertyDataError::InvalidValue));
        assert_eq!(
            read_simple(&[b"null"]),
            Err(PropertyDataError::InvalidValue)
        );
        assert!(read_simple(&[b"1 2"]).is_err(), "more than one value");
        assert_eq!(
            read_simple(&[b"\"0123456789abcdefg\""]),
            Err(PropertyDataError::Overflow)
        );
        let mut buf = [0u8; 16];
        let n = write_simple(
            ScalarValue::String {
                text: "performance",
                truncated: false,
            },
            &mut buf,
        )
        .unwrap();
        assert_eq!(&buf[..n], b"\"performance\"");
        let n = write_simple(ScalarValue::Bool(false), &mut buf).unwrap();
        assert_eq!(&buf[..n], b"false");
    }

    #[test]
    fn prop_readers_never_panic() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..48), cut in 0usize..48)| {
            let c = cut.min(bytes.len());
            let _ = read_partial::<8>(&[&bytes[..c], &bytes[c..]]);
            let _ = read_simple(&[&bytes[..c], &bytes[c..]]);
        });
    }
}
