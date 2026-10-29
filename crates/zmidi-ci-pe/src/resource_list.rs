//! The ResourceList Resource (M2-103-UM v1.2 sections 9 and 14; Tables 79 and 80).
//!
//! - [`ResourceCaps`] holds the capability Properties of one Resource. [`ResourceCaps::check`]
//!   gives the reply status of a request that the Resource does not permit; the responder
//!   replies with it, and the initiator refuses to send such a request (REQ-PE-TXN-003,
//!   REQ-PE-RES-001, REQ-PE-HDR-010, REQ-PE-ENC-007, REQ-PE-FLOW-007, -008, REQ-PE-STAT-012).
//! - [`ResourceListWriter`] streams the ResourceList from a constant table of [`ResourceEntry`],
//!   one entry at a time, into the Chunks of the reply, with a buffer for one entry only
//!   (REQ-PE-LIST-001, DEC-007). It writes no entry for ResourceList (REQ-PE-RES-004) and no
//!   Property with its default value (REQ-PE-LIST-009).
//! - [`ResourceListReader`] reads a ResourceList reply Chunk by Chunk and gives events: the
//!   "mediaTypes" strings, the Columns and the raw "schema" text of the object in progress, then
//!   one `Resource` event at the end of each object, or `Invalid` for an object with an error;
//!   the reader then continues with the next object (REQ-PE-LIST-003).

use crate::header::{EncodingName, Header, HeaderStr, Status};
use crate::json::{JsonError, JsonReader, JsonWriter, Token, Tokens};

const MAX_RESOURCE: usize = 36;
const MAX_MEDIA_TYPE: usize = 75;
/// Buffer for a "resource" value of up to 36 characters in UTF-8.
const NAME_BUF: usize = MAX_RESOURCE * 4;

/// Error of the ResourceList reader and writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ListError {
    /// Not valid JSON.
    #[error("JSON: {0}")]
    Json(JsonError),
    /// The Property Data is not a JSON array (REQ-PE-LIST-003).
    #[error("ResourceList is not a JSON array")]
    NotArray,
    /// An array element is not a JSON object.
    #[error("ResourceList element is not an object")]
    NotObject,
    /// A Property is missing.
    #[error("missing \"{0}\"")]
    Missing(&'static str),
    /// A Property has a wrong type, length or form.
    #[error("invalid \"{0}\"")]
    Invalid(&'static str),
    /// A value is longer than the caller buffer, or an output buffer is full.
    #[error("buffer overflow")]
    Overflow,
}

impl From<JsonError> for ListError {
    fn from(e: JsonError) -> Self {
        match e {
            JsonError::Overflow => ListError::Overflow,
            e => ListError::Json(e),
        }
    }
}

/// "canSet" (Table 79).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CanSet {
    /// "none" (default).
    #[default]
    None,
    /// "full".
    Full,
    /// "partial": "full" is also supported.
    Partial,
}

impl CanSet {
    /// The spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            CanSet::None => "none",
            CanSet::Full => "full",
            CanSet::Partial => "partial",
        }
    }
}

/// A set of Table 12 encodings ("encodings", default ["ASCII"]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encodings {
    /// "ASCII".
    pub ascii: bool,
    /// "Mcoded7".
    pub mcoded7: bool,
    /// "zlib+Mcoded7".
    pub zlib_mcoded7: bool,
}

impl Default for Encodings {
    fn default() -> Self {
        Encodings {
            ascii: true,
            mcoded7: false,
            zlib_mcoded7: false,
        }
    }
}

impl Encodings {
    /// True when the set lists `e`.
    pub const fn contains(&self, e: EncodingName) -> bool {
        match e {
            EncodingName::Ascii => self.ascii,
            EncodingName::Mcoded7 => self.mcoded7,
            EncodingName::ZlibMcoded7 => self.zlib_mcoded7,
        }
    }

    fn is_default(&self) -> bool {
        *self == Encodings::default()
    }
}

/// The operation of a request, for [`ResourceCaps::check`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Inquiry: Get Property Data.
    Get,
    /// Inquiry: Set Property Data.
    Set,
    /// Subscription "start".
    Subscribe,
}

/// The capability Properties of one Resource (Tables 79 and 80).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceCaps {
    /// "canGet" (default true).
    pub can_get: bool,
    /// "canSet" (default "none").
    pub can_set: CanSet,
    /// "canSubscribe" (default false).
    pub can_subscribe: bool,
    /// "requireResId" (default false).
    pub require_res_id: bool,
    /// "fcOnGet" (default false).
    pub fc_on_get: bool,
    /// "fcOnSet" (default false).
    pub fc_on_set: bool,
    /// "encodings" (default ["ASCII"]).
    pub encodings: Encodings,
    /// "canPaginate" (default false).
    pub can_paginate: bool,
}

impl Default for ResourceCaps {
    fn default() -> Self {
        DEFAULT_CAPS
    }
}

impl ResourceCaps {
    /// The reply status for a request that these capabilities do not permit, or `Ok`. The
    /// initiator refuses to send a request that gives an error here.
    /// - 405: Get with canGet false; Set with canSet "none", or a Partial SET without
    ///   canSet "partial" (REQ-PE-TXN-003); a "start" without canSubscribe.
    /// - 400: no "resId" with requireResId (REQ-PE-RES-001).
    /// - 406: "flowControl":true without fcOnGet / fcOnSet (REQ-PE-HDR-010, REQ-PE-FLOW-007).
    /// - 407: a Set with no "flowControl":true with fcOnSet (REQ-PE-FLOW-008).
    /// - 415: a "mutualEncoding" that "encodings" does not list, also no "mutualEncoding" (that
    ///   is "ASCII") in a Get or a Set where "encodings" does not list "ASCII" (REQ-PE-ENC-007,
    ///   REQ-PE-STAT-012).
    pub fn check(&self, op: Operation, header: &Header<'_>) -> Result<(), Status> {
        match op {
            Operation::Get if !self.can_get => return Err(Status::NOT_ALLOWED),
            Operation::Set if self.can_set == CanSet::None => return Err(Status::NOT_ALLOWED),
            Operation::Set if header.set_partial && self.can_set != CanSet::Partial => {
                return Err(Status::NOT_ALLOWED)
            }
            Operation::Subscribe if !self.can_subscribe => return Err(Status::NOT_ALLOWED),
            _ => {}
        }
        if self.require_res_id && header.res_id.is_none() {
            return Err(Status::BAD_REQUEST);
        }
        match op {
            Operation::Get if header.flow_control && !self.fc_on_get => {
                return Err(Status::FLOW_CONTROL_NOT_SUPPORTED)
            }
            Operation::Set if header.flow_control && !self.fc_on_set => {
                return Err(Status::FLOW_CONTROL_NOT_SUPPORTED)
            }
            Operation::Set if !header.flow_control && self.fc_on_set => {
                return Err(Status::FLOW_CONTROL_REQUIRED)
            }
            _ => {}
        }
        // No "mutualEncoding" in a Get or a Set is "ASCII" (REQ-PE-ENC-003).
        let encoding = match op {
            Operation::Get | Operation::Set => {
                Some(header.mutual_encoding.unwrap_or(EncodingName::Ascii))
            }
            _ => header.mutual_encoding,
        };
        if let Some(e) = encoding {
            if !self.encodings.contains(e) {
                return Err(Status::UNSUPPORTED_MEDIA);
            }
        }
        Ok(())
    }
}

/// A Column of a List Resource (REQ-PE-LIST-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind<'a> {
    /// "property": a Property name.
    Property(&'a str),
    /// "link": a Resource name of 1 to 36 characters.
    Link(&'a str),
}

/// A Column object of a "columns" array, to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column<'a> {
    /// "property" or "link".
    pub kind: ColumnKind<'a>,
    /// "title", required by the writer.
    pub title: &'a str,
}

/// One entry of the constant Resource table of a responder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEntry<'a> {
    /// "resource", 1-36 characters, A-Z a-z 0-9, optional "X-" prefix.
    pub resource: &'a str,
    /// Capabilities.
    pub caps: ResourceCaps,
    /// "mediaTypes"; empty means the default ["application/json"].
    pub media_types: &'a [&'a str],
    /// "schema", raw JSON text.
    pub schema: Option<&'a [u8]>,
    /// "columns"; empty means none.
    pub columns: &'a [Column<'a>],
    /// Write every Property, also the ones with their default value (REQ-PE-LIST-009).
    pub write_defaults: bool,
}

impl<'a> ResourceEntry<'a> {
    /// An entry with the default capabilities.
    pub const fn new(resource: &'a str) -> Self {
        ResourceEntry {
            resource,
            caps: DEFAULT_CAPS,
            media_types: &[],
            schema: None,
            columns: &[],
            write_defaults: false,
        }
    }

    /// Writes the entry as one JSON object into `out` (REQ-PE-LIST-004 to -009).
    pub fn write(&self, out: &mut [u8]) -> Result<usize, ListError> {
        self.write_with(out, true)
    }

    /// [`Self::write`]; with `subscriptions` false, "canSubscribe" is false whatever the caps say
    /// (REQ-PE-SUB-022).
    fn write_with(&self, out: &mut [u8], subscriptions: bool) -> Result<usize, ListError> {
        let r = self.resource;
        let body = r.strip_prefix("X-").unwrap_or(r);
        if r.is_empty()
            || r.len() > MAX_RESOURCE
            || body.is_empty()
            || !body.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(ListError::Invalid("resource"));
        }
        let c = &self.caps;
        let all = self.write_defaults;
        let mut w = JsonWriter::new(out);
        w.begin_object()?;
        w.name("resource", 20)?;
        w.string(r)?;
        let flags = [
            ("canGet", c.can_get, true),
            ("canSubscribe", c.can_subscribe && subscriptions, false),
            ("requireResId", c.require_res_id, false),
            ("fcOnGet", c.fc_on_get, false),
            ("fcOnSet", c.fc_on_set, false),
            ("canPaginate", c.can_paginate, false),
        ];
        for (name, v, default) in flags {
            if all || v != default {
                w.name(name, 20)?;
                w.boolean(v)?;
            }
        }
        if all || c.can_set != CanSet::None {
            w.name("canSet", 20)?;
            w.string(c.can_set.as_str())?;
        }
        if all || !self.media_types.is_empty() {
            w.name("mediaTypes", 20)?;
            w.begin_array()?;
            if self.media_types.is_empty() {
                w.string("application/json")?;
            }
            for m in self.media_types {
                if m.len() > MAX_MEDIA_TYPE || !crate::header::media_type_ok(m) {
                    return Err(ListError::Invalid("mediaTypes"));
                }
                w.string(m)?;
            }
            w.end_array()?;
        }
        if all || !c.encodings.is_default() {
            w.name("encodings", 20)?;
            w.begin_array()?;
            for e in [
                EncodingName::Ascii,
                EncodingName::Mcoded7,
                EncodingName::ZlibMcoded7,
            ] {
                if c.encodings.contains(e) {
                    w.string(e.as_str())?;
                }
            }
            w.end_array()?;
        }
        if let Some(s) = self.schema {
            if s.first() != Some(&b'{') {
                return Err(ListError::Invalid("schema"));
            }
            w.name("schema", 20)?;
            w.raw(s)?;
        }
        if !self.columns.is_empty() {
            w.name("columns", 20)?;
            w.begin_array()?;
            for col in self.columns {
                if col.title.is_empty() {
                    return Err(ListError::Missing("title"));
                }
                w.begin_object()?;
                match col.kind {
                    ColumnKind::Property(p) => {
                        w.name("property", 20)?;
                        w.string(p)?;
                    }
                    ColumnKind::Link(l) => {
                        if l.is_empty() || l.chars().count() > MAX_RESOURCE {
                            return Err(ListError::Invalid("link"));
                        }
                        w.name("link", 20)?;
                        w.string(l)?;
                    }
                }
                w.name("title", 20)?;
                w.string(col.title)?;
                w.end_object()?;
            }
            w.end_array()?;
        }
        w.end_object()?;
        Ok(w.finish()?)
    }
}

/// Finds the entry of `resource` in a Resource table.
pub fn find_resource<'t>(
    table: &'t [ResourceEntry<'t>],
    resource: &HeaderStr<'_>,
) -> Option<&'t ResourceEntry<'t>> {
    table.iter().find(|e| resource.eq_str(e.resource))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WStage {
    Open,
    Entries,
    Done,
}

/// Streaming ResourceList writer with a buffer of `E` bytes for one entry (REQ-PE-LIST-001).
#[derive(Debug)]
pub struct ResourceListWriter<'t, const E: usize> {
    table: &'t [ResourceEntry<'t>],
    next: usize,
    written: usize,
    buf: [u8; E],
    len: usize,
    pos: usize,
    stage: WStage,
    /// False: every entry has "canSubscribe" false (REQ-PE-SUB-022).
    subscriptions: bool,
}

impl<'t, const E: usize> ResourceListWriter<'t, E> {
    /// A writer over the Resource table of the responder.
    pub fn new(table: &'t [ResourceEntry<'t>]) -> Self {
        ResourceListWriter {
            table,
            next: 0,
            written: 0,
            buf: [0; E],
            len: 0,
            pos: 0,
            stage: WStage::Open,
            subscriptions: true,
        }
    }

    /// The writer for a Device with no subscriptions: every entry has "canSubscribe" false,
    /// whatever its caps say (REQ-PE-SUB-022, M2-103 11).
    pub fn without_subscriptions(mut self) -> Self {
        self.subscriptions = false;
        self
    }

    /// True after the last byte is written.
    pub fn is_done(&self) -> bool {
        self.stage == WStage::Done && self.pos == self.len
    }

    /// Prepares the next text in the buffer: '[' and the first entry, ',' and an entry, or ']'.
    fn refill(&mut self) -> Result<(), ListError> {
        self.len = 0;
        self.pos = 0;
        if self.stage == WStage::Open {
            self.buf[0] = b'[';
            self.len = 1;
            self.stage = WStage::Entries;
        }
        // Skip the ResourceList entry (REQ-PE-RES-004).
        while self.next < self.table.len() && self.table[self.next].resource == "ResourceList" {
            self.next += 1;
        }
        if self.next < self.table.len() {
            if self.written > 0 {
                *self.buf.get_mut(self.len).ok_or(ListError::Overflow)? = b',';
                self.len += 1;
            }
            let n =
                self.table[self.next].write_with(&mut self.buf[self.len..], self.subscriptions)?;
            self.len += n;
            self.next += 1;
            self.written += 1;
        } else {
            *self.buf.get_mut(self.len).ok_or(ListError::Overflow)? = b']';
            self.len += 1;
            self.stage = WStage::Done;
        }
        Ok(())
    }

    /// Writes the next bytes of the ResourceList into `out` (the Property Data room of the next
    /// Chunk); gives the number of bytes written, 0 when the list is complete.
    pub fn write_next(&mut self, out: &mut [u8]) -> Result<usize, ListError> {
        let mut n = 0;
        while n < out.len() {
            if self.pos == self.len {
                if self.stage == WStage::Done {
                    break;
                }
                self.refill()?;
            }
            let k = (self.len - self.pos).min(out.len() - n);
            out[n..n + k].copy_from_slice(&self.buf[self.pos..self.pos + k]);
            self.pos += k;
            n += k;
        }
        Ok(n)
    }
}

/// Which Properties an object of a received ResourceList had (the others have their default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Present {
    pub can_get: bool,
    pub can_set: bool,
    pub can_subscribe: bool,
    pub require_res_id: bool,
    pub fc_on_get: bool,
    pub fc_on_set: bool,
    pub media_types: bool,
    pub encodings: bool,
    pub schema: bool,
    pub can_paginate: bool,
    pub columns: bool,
}

/// One Resource declaration of a received ResourceList (REQ-PE-LIST-003 to -007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceDecl<'a> {
    /// "resource".
    pub resource: &'a str,
    /// Capabilities, with defaults for the absent Properties.
    pub caps: ResourceCaps,
    /// The Properties that the object had.
    pub present: Present,
    /// "canSet" had a value other than "none", "full" and "partial"; it is read as "none".
    pub can_set_invalid: bool,
    /// "encodings" had values other than the Table 12 values; they are skipped.
    pub encodings_skipped: bool,
}

/// A received Column object (REQ-PE-LIST-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnDecl<'a> {
    /// "property" or "link".
    pub kind: ColumnKind<'a>,
    /// "title", if any; `title_truncated` when it was cut at the buffer size.
    pub title: Option<&'a str>,
    /// The title was cut.
    pub title_truncated: bool,
}

/// An event of the ResourceList reader. MediaType, Column and SchemaPart belong to the object
/// whose Resource (or Invalid) event comes next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListEvent<'a> {
    /// One "mediaTypes" string, in array order.
    MediaType(&'a str),
    /// One Column, in array order.
    Column(ColumnDecl<'a>),
    /// A piece of the raw "schema" text.
    SchemaPart(&'a [u8]),
    /// The end of a valid object.
    Resource(ResourceDecl<'a>),
    /// The end of an object with an error; the reader continues with the next object.
    Invalid(ListError),
}

/// Where the reader is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RState {
    Start,
    /// In the array, between objects.
    Array,
    /// In an object, waiting for a name.
    Object,
    /// After a name, waiting for its value.
    Value(Prop),
    /// In a "mediaTypes" or "encodings" array.
    StrArray(Prop),
    /// In "columns", between Column objects.
    Columns,
    /// In a Column object, waiting for a name.
    Column,
    ColumnValue(ColProp),
    /// Skip a container until the depth comes back to the value given.
    Skip(u8),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prop {
    Resource,
    CanGet,
    CanSet,
    CanSubscribe,
    RequireResId,
    FcOnGet,
    FcOnSet,
    MediaTypes,
    Encodings,
    Schema,
    CanPaginate,
    Columns,
    Unknown,
}

impl Prop {
    fn of(name: &str) -> Prop {
        match name {
            "resource" => Prop::Resource,
            "canGet" => Prop::CanGet,
            "canSet" => Prop::CanSet,
            "canSubscribe" => Prop::CanSubscribe,
            "requireResId" => Prop::RequireResId,
            "fcOnGet" => Prop::FcOnGet,
            "fcOnSet" => Prop::FcOnSet,
            "mediaTypes" => Prop::MediaTypes,
            "encodings" => Prop::Encodings,
            "schema" => Prop::Schema,
            "canPaginate" => Prop::CanPaginate,
            "columns" => Prop::Columns,
            _ => Prop::Unknown,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Prop::Resource => "resource",
            Prop::CanGet => "canGet",
            Prop::CanSet => "canSet",
            Prop::CanSubscribe => "canSubscribe",
            Prop::RequireResId => "requireResId",
            Prop::FcOnGet => "fcOnGet",
            Prop::FcOnSet => "fcOnSet",
            Prop::MediaTypes => "mediaTypes",
            Prop::Encodings => "encodings",
            Prop::Schema => "schema",
            Prop::CanPaginate => "canPaginate",
            Prop::Columns => "columns",
            Prop::Unknown => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColProp {
    Property,
    Link,
    Title,
    Unknown,
}

/// What one token gives, with no borrow.
enum Out {
    None,
    /// Start the capture of the "schema" value.
    Capture,
    MediaType,
    Column,
    Resource,
    Invalid(ListError),
}

const DEFAULT_CAPS: ResourceCaps = ResourceCaps {
    can_get: true,
    can_set: CanSet::None,
    can_subscribe: false,
    require_res_id: false,
    fc_on_get: false,
    fc_on_set: false,
    encodings: Encodings {
        ascii: true,
        mcoded7: false,
        zlib_mcoded7: false,
    },
    can_paginate: false,
};

const NO_PRESENT: Present = Present {
    can_get: false,
    can_set: false,
    can_subscribe: false,
    require_res_id: false,
    fc_on_get: false,
    fc_on_set: false,
    media_types: false,
    encodings: false,
    schema: false,
    can_paginate: false,
    columns: false,
};

/// Reader state apart from the JSON reader, so that both can be borrowed at the same time.
#[derive(Debug, Clone)]
struct ListState<const S: usize> {
    state: RState,
    /// State after a Skip.
    after_skip: RState,
    depth: u8,
    name: [u8; NAME_BUF],
    name_len: usize,
    caps: ResourceCaps,
    present: Present,
    have_resource: bool,
    can_set_invalid: bool,
    encodings_skipped: bool,
    /// The first error of the object in progress.
    error: Option<ListError>,
    /// A fatal error: the reader gives nothing more.
    fatal: Option<ListError>,
    text: [u8; S],
    text_len: usize,
    /// Column: true for "link", and the length in `col_a`.
    col_kind: Option<(bool, usize)>,
    col_count: u8,
    col_a: [u8; S],
    col_title: [u8; S],
    col_title_len: Option<(usize, bool)>,
    schema_first: bool,
}

impl<const S: usize> ListState<S> {
    const fn new() -> Self {
        ListState {
            state: RState::Start,
            after_skip: RState::Array,
            depth: 0,
            name: [0; NAME_BUF],
            name_len: 0,
            caps: DEFAULT_CAPS,
            present: NO_PRESENT,
            have_resource: false,
            can_set_invalid: false,
            encodings_skipped: false,
            error: None,
            fatal: None,
            text: [0; S],
            text_len: 0,
            col_kind: None,
            col_count: 0,
            col_a: [0; S],
            col_title: [0; S],
            col_title_len: None,
            schema_first: false,
        }
    }

    fn fail(&mut self, e: ListError) {
        if self.error.is_none() {
            self.error = Some(e);
        }
    }

    /// Skips the container that a Begin token opened, then goes to `next`.
    fn skip(&mut self, next: RState) {
        self.state = RState::Skip(self.depth - 1);
        self.after_skip = next;
    }

    fn step(&mut self, t: Token<'_>) -> Out {
        use RState as R;
        match t {
            Token::BeginObject | Token::BeginArray => self.depth += 1,
            Token::EndObject | Token::EndArray => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        let begin = matches!(t, Token::BeginObject | Token::BeginArray);
        match (self.state, t) {
            (R::Skip(d), Token::EndObject | Token::EndArray) if self.depth == d => {
                self.state = self.after_skip
            }
            (R::Skip(_), _) => {}
            (R::Start, Token::BeginArray) => self.state = R::Array,
            (R::Start, _) => {
                self.state = R::Done;
                self.fatal = Some(ListError::NotArray);
                return Out::Invalid(ListError::NotArray);
            }
            (R::Array, Token::BeginObject) => {
                self.name_len = 0;
                self.caps = DEFAULT_CAPS;
                self.present = NO_PRESENT;
                self.have_resource = false;
                self.can_set_invalid = false;
                self.encodings_skipped = false;
                self.error = None;
                self.state = R::Object;
            }
            (R::Array, Token::EndArray) => self.state = R::Done,
            (R::Array, _) => {
                if begin {
                    self.skip(R::Array);
                }
                return Out::Invalid(ListError::NotObject);
            }
            (R::Object, Token::Name { text, .. }) => {
                let p = Prop::of(text);
                self.state = R::Value(p);
                if p == Prop::Schema {
                    self.schema_first = true;
                    return Out::Capture;
                }
            }
            (R::Object, Token::EndObject) => {
                self.state = R::Array;
                if !self.have_resource {
                    self.fail(ListError::Missing("resource"));
                }
                return match self.error {
                    Some(e) => Out::Invalid(e),
                    None => Out::Resource,
                };
            }
            (R::Value(p), t) => return self.value(p, t, begin),
            (R::StrArray(p), Token::EndArray) => {
                let _ = p;
                self.state = R::Object;
            }
            (R::StrArray(p), Token::String { text, truncated }) => {
                if p == Prop::MediaTypes {
                    if truncated || text.chars().count() > MAX_MEDIA_TYPE || text.len() > S {
                        self.fail(ListError::Invalid("mediaTypes"));
                    } else if self.error.is_none() {
                        self.text[..text.len()].copy_from_slice(text.as_bytes());
                        self.text_len = text.len();
                        return Out::MediaType;
                    }
                } else {
                    match EncodingName::parse(&HeaderStr::Text(text)) {
                        Some(EncodingName::Ascii) => self.caps.encodings.ascii = true,
                        Some(EncodingName::Mcoded7) => self.caps.encodings.mcoded7 = true,
                        Some(EncodingName::ZlibMcoded7) => self.caps.encodings.zlib_mcoded7 = true,
                        None => self.encodings_skipped = true,
                    }
                }
            }
            (R::StrArray(p), _) => {
                self.fail(ListError::Invalid(p.name()));
                if begin {
                    self.skip(R::StrArray(p));
                }
            }
            (R::Columns, Token::BeginObject) => {
                self.col_kind = None;
                self.col_count = 0;
                self.col_title_len = None;
                self.state = R::Column;
            }
            (R::Columns, Token::EndArray) => self.state = R::Object,
            (R::Columns, _) => {
                self.fail(ListError::Invalid("columns"));
                if begin {
                    self.skip(R::Columns);
                }
            }
            (R::Column, Token::Name { text, .. }) => {
                self.state = R::ColumnValue(match text {
                    "property" => ColProp::Property,
                    "link" => ColProp::Link,
                    "title" => ColProp::Title,
                    _ => ColProp::Unknown,
                });
            }
            (R::Column, Token::EndObject) => {
                self.state = R::Columns;
                if self.col_count != 1 || self.col_kind.is_none() {
                    self.fail(ListError::Invalid("columns"));
                } else if self.error.is_none() {
                    return Out::Column;
                }
            }
            (R::ColumnValue(c), t) => {
                self.state = R::Column;
                match (c, t) {
                    (ColProp::Property | ColProp::Link, Token::String { text, truncated }) => {
                        let link = c == ColProp::Link;
                        self.col_count += 1;
                        if truncated || text.len() > S {
                            self.fail(ListError::Overflow);
                        } else if link && !(1..=MAX_RESOURCE).contains(&text.chars().count()) {
                            self.fail(ListError::Invalid("link"));
                        } else {
                            self.col_a[..text.len()].copy_from_slice(text.as_bytes());
                            self.col_kind = Some((link, text.len()));
                        }
                    }
                    (ColProp::Title, Token::String { text, truncated }) => {
                        let n = text.len().min(S);
                        self.col_title[..n].copy_from_slice(&text.as_bytes()[..n]);
                        self.col_title_len = Some((n, truncated));
                    }
                    (ColProp::Unknown, _) => {
                        if begin {
                            self.skip(R::Column);
                        }
                    }
                    _ => {
                        self.fail(ListError::Invalid("columns"));
                        if begin {
                            self.skip(R::Column);
                        }
                    }
                }
            }
            (R::Done, _) => {}
            _ => {
                self.state = R::Done;
                self.fatal = Some(ListError::Json(JsonError::Syntax));
                return Out::Invalid(ListError::Json(JsonError::Syntax));
            }
        }
        Out::None
    }

    fn value(&mut self, p: Prop, t: Token<'_>, begin: bool) -> Out {
        use RState as R;
        self.state = R::Object;
        let b = match t {
            Token::Bool(b) => Some(b),
            _ => None,
        };
        let bad = |s: &mut Self| {
            s.fail(ListError::Invalid(p.name()));
            if begin {
                s.skip(R::Object);
            }
        };
        match (p, b) {
            (Prop::CanGet, Some(v)) => (self.caps.can_get, self.present.can_get) = (v, true),
            (Prop::CanSubscribe, Some(v)) => {
                (self.caps.can_subscribe, self.present.can_subscribe) = (v, true)
            }
            (Prop::RequireResId, Some(v)) => {
                (self.caps.require_res_id, self.present.require_res_id) = (v, true)
            }
            (Prop::FcOnGet, Some(v)) => (self.caps.fc_on_get, self.present.fc_on_get) = (v, true),
            (Prop::FcOnSet, Some(v)) => (self.caps.fc_on_set, self.present.fc_on_set) = (v, true),
            (Prop::CanPaginate, Some(v)) => {
                (self.caps.can_paginate, self.present.can_paginate) = (v, true)
            }
            (
                Prop::CanGet
                | Prop::CanSubscribe
                | Prop::RequireResId
                | Prop::FcOnGet
                | Prop::FcOnSet
                | Prop::CanPaginate,
                None,
            ) => bad(self),
            (Prop::Resource, _) => match t {
                Token::String { text, truncated }
                    if !truncated
                        && (1..=MAX_RESOURCE).contains(&text.chars().count())
                        && text.len() <= NAME_BUF =>
                {
                    self.name[..text.len()].copy_from_slice(text.as_bytes());
                    self.name_len = text.len();
                    self.have_resource = true;
                }
                _ => bad(self),
            },
            (Prop::CanSet, _) => match t {
                Token::String { text, .. } => {
                    self.present.can_set = true;
                    self.caps.can_set = match text {
                        "none" => CanSet::None,
                        "full" => CanSet::Full,
                        "partial" => CanSet::Partial,
                        _ => {
                            // REQ-PE-LIST-004: another string reads as "none", marked.
                            self.can_set_invalid = true;
                            CanSet::None
                        }
                    };
                }
                _ => bad(self),
            },
            (Prop::MediaTypes | Prop::Encodings, _) => match t {
                Token::BeginArray => {
                    if p == Prop::MediaTypes {
                        self.present.media_types = true;
                    } else {
                        self.present.encodings = true;
                        self.caps.encodings = Encodings {
                            ascii: false,
                            mcoded7: false,
                            zlib_mcoded7: false,
                        };
                    }
                    self.state = R::StrArray(p);
                }
                _ => bad(self),
            },
            (Prop::Columns, _) => match t {
                Token::BeginArray => {
                    self.present.columns = true;
                    self.state = R::Columns;
                }
                _ => bad(self),
            },
            // The "schema" value comes as Raw pieces; ListEvents handles them.
            (Prop::Schema, _) => {}
            (Prop::Unknown, _) => {
                if begin {
                    self.skip(R::Object);
                }
            }
        }
        Out::None
    }
}

/// Streaming ResourceList reader with string buffers of `S` bytes (REQ-PE-LIST-003 to -008).
#[derive(Debug, Clone)]
pub struct ResourceListReader<const S: usize> {
    json: JsonReader<S>,
    st: ListState<S>,
}

#[allow(clippy::new_without_default)]
impl<const S: usize> ResourceListReader<S> {
    /// A reader for one ResourceList.
    pub const fn new() -> Self {
        ResourceListReader {
            json: JsonReader::new(),
            st: ListState::new(),
        }
    }

    /// Takes the Property Data of the next Chunk and gives its events.
    pub fn push<'r, 'i>(&'r mut self, input: &'i [u8]) -> ListEvents<'r, 'i, S> {
        ListEvents {
            tokens: self.json.push(input),
            st: &mut self.st,
            input,
        }
    }

    /// The end of the Property Data: an error when the list is not a complete JSON array.
    pub fn finish(&mut self) -> Result<(), ListError> {
        if let Some(e) = self.st.fatal {
            return Err(e);
        }
        self.json.finish()?;
        if self.st.state != RState::Done {
            return Err(ListError::Json(JsonError::Syntax));
        }
        Ok(())
    }
}

/// The events of one Chunk of a ResourceList. Call [`ListEvents::next_event`] until it gives
/// `None`.
#[derive(Debug)]
pub struct ListEvents<'r, 'i, const S: usize> {
    tokens: Tokens<'r, 'i, S>,
    st: &'r mut ListState<S>,
    input: &'i [u8],
}

impl<'i, const S: usize> ListEvents<'_, 'i, S> {
    /// The next event, or `None` at the end of the Chunk. An event is valid until the next call.
    pub fn next_event(&mut self) -> Option<ListEvent<'_>> {
        if self.st.fatal.is_some() {
            return None;
        }
        loop {
            let mut raw = None;
            let out = match self.tokens.next_token()? {
                Err(e) => {
                    self.st.state = RState::Done;
                    self.st.fatal = Some(e.into());
                    Out::Invalid(e.into())
                }
                Ok(Token::Raw(r)) => {
                    let start = r.as_ptr() as usize - self.input.as_ptr() as usize;
                    raw = Some(start..start + r.len());
                    Out::None
                }
                Ok(t) => self.st.step(t),
            };
            if let Some(r) = raw {
                // A piece of the "schema" text (DEC-007, REQ-PE-LIST-006).
                if self.st.schema_first {
                    self.st.schema_first = false;
                    if self.input.get(r.start).copied() != Some(b'{') {
                        self.st.fail(ListError::Invalid("schema"));
                    }
                }
                self.st.present.schema = true;
                if !self.tokens.capturing() {
                    self.st.state = RState::Object;
                }
                if r.is_empty() || self.st.error.is_some() {
                    continue;
                }
                return Some(ListEvent::SchemaPart(&self.input[r]));
            }
            match out {
                Out::None => {}
                Out::Capture => self.tokens.capture_value(),
                Out::MediaType => {
                    let t = core::str::from_utf8(&self.st.text[..self.st.text_len]).unwrap_or("");
                    return Some(ListEvent::MediaType(t));
                }
                Out::Column => {
                    let st = &*self.st;
                    let (link, n) = st.col_kind.unwrap_or((false, 0));
                    let a = core::str::from_utf8(&st.col_a[..n]).unwrap_or("");
                    let kind = if link {
                        ColumnKind::Link(a)
                    } else {
                        ColumnKind::Property(a)
                    };
                    let (title, title_truncated) = match st.col_title_len {
                        // REQ-PE-LIST-008: marked only when the reader cut it; a title of
                        // exactly S bytes is whole.
                        Some((n, cut)) => (Some(utf8_prefix(&st.col_title[..n])), cut),
                        None => (None, false),
                    };
                    return Some(ListEvent::Column(ColumnDecl {
                        kind,
                        title,
                        title_truncated,
                    }));
                }
                Out::Resource => {
                    let st = &*self.st;
                    return Some(ListEvent::Resource(ResourceDecl {
                        resource: core::str::from_utf8(&st.name[..st.name_len]).unwrap_or(""),
                        caps: st.caps,
                        present: st.present,
                        can_set_invalid: st.can_set_invalid,
                        encodings_skipped: st.encodings_skipped,
                    }));
                }
                Out::Invalid(e) => return Some(ListEvent::Invalid(e)),
            }
        }
    }
}

fn utf8_prefix(b: &[u8]) -> &str {
    match core::str::from_utf8(b) {
        Ok(s) => s,
        Err(e) => core::str::from_utf8(&b[..e.valid_up_to()]).unwrap_or(""),
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
    enum E {
        M(String),
        C(String, bool, Option<String>),
        S(Vec<u8>),
        R(String, ResourceCaps, bool, bool),
        I(ListError),
    }

    fn read<const S: usize>(pieces: &[&[u8]]) -> (Vec<E>, Result<(), ListError>) {
        let mut r = ResourceListReader::<S>::new();
        let mut out: Vec<E> = Vec::new();
        for p in pieces {
            let mut ev = r.push(p);
            while let Some(e) = ev.next_event() {
                let e = match e {
                    ListEvent::MediaType(m) => E::M(m.to_string()),
                    ListEvent::Column(c) => {
                        let (link, a) = match c.kind {
                            ColumnKind::Link(a) => (true, a),
                            ColumnKind::Property(a) => (false, a),
                        };
                        E::C(a.to_string(), link, c.title.map(|t| t.to_string()))
                    }
                    ListEvent::SchemaPart(p) => E::S(p.to_vec()),
                    ListEvent::Resource(d) => E::R(
                        d.resource.to_string(),
                        d.caps,
                        d.can_set_invalid,
                        d.encodings_skipped,
                    ),
                    ListEvent::Invalid(e) => E::I(e),
                };
                match (out.last_mut(), &e) {
                    (Some(E::S(a)), E::S(b)) => a.extend_from_slice(b),
                    _ => out.push(e),
                }
            }
        }
        let end = r.finish();
        (out, end)
    }

    const LIST: &[u8] = br#"[{"resource":"DeviceInfo"},{"resource":"ChannelList"},{"resource":"CMList","canSet":"partial","canSubscribe":true,"encodings":["ASCII","MCoded7","Other"],"mediaTypes":["application/json","image/png"],"canPaginate":true,"columns":[{"property":"name","title":"Name"},{"link":"Detail","title":"More"}],"schema":{"type":"object","p":{"x":[1,{"}":2}]}},"extra":{"a":[1]}},{"canGet":true},{"resource":"X","canGet":"yes"},{"resource":"Y","canSet":"maybe"},5,{"resource":"Z"}]"#;

    #[test]
    fn reads_resource_list_example() {
        // M2-103 Table 82 style; REQ-PE-LIST-003 to -008, REQ-PE-ENC-005.
        let (ev, end) = read::<32>(&[LIST]);
        end.unwrap();
        let mut cm = DEFAULT_CAPS;
        cm.can_set = CanSet::Partial;
        cm.can_subscribe = true;
        cm.can_paginate = true;
        cm.encodings = Encodings {
            ascii: true,
            mcoded7: true,
            zlib_mcoded7: false,
        };
        let mut y = DEFAULT_CAPS;
        y.can_set = CanSet::None;
        assert_eq!(
            ev,
            [
                E::R(String::from("DeviceInfo"), DEFAULT_CAPS, false, false),
                E::R(String::from("ChannelList"), DEFAULT_CAPS, false, false),
                E::M(String::from("application/json")),
                E::M(String::from("image/png")),
                E::C(String::from("name"), false, Some(String::from("Name"))),
                E::C(String::from("Detail"), true, Some(String::from("More"))),
                E::S(br#"{"type":"object","p":{"x":[1,{"}":2}]}}"#.to_vec()),
                E::R(String::from("CMList"), cm, false, true),
                E::I(ListError::Missing("resource")),
                E::I(ListError::Invalid("canGet")),
                E::R(String::from("Y"), y, true, false),
                E::I(ListError::NotObject),
                E::R(String::from("Z"), DEFAULT_CAPS, false, false),
            ]
        );
    }

    #[test]
    fn same_events_in_any_chunk_split() {
        // DEC-007: one Chunk at a time.
        let (whole, _) = read::<32>(&[LIST]);
        for cut in (1..LIST.len()).step_by(7) {
            let (split, end) = read::<32>(&[&LIST[..cut], &LIST[cut..]]);
            end.unwrap();
            assert_eq!(split, whole, "cut {cut}");
        }
    }

    #[test]
    fn reader_errors() {
        // REQ-PE-LIST-003 to -008.
        assert_eq!(
            read::<32>(&[br#"{"resource":"A"}"#]).1,
            Err(ListError::NotArray)
        );
        let (ev, _) = read::<32>(&[br#"[{"resource":"A","schema":[1]},{"resource":"B","columns":[{"property":"p","link":"l","title":"t"}]},{"resource":"C","mediaTypes":["0123456789012345678901234567890123456789012345678901234567890123456789012345"]}]"#]);
        assert_eq!(
            ev,
            [
                E::I(ListError::Invalid("schema")),
                E::I(ListError::Invalid("columns")),
                E::I(ListError::Invalid("mediaTypes"))
            ]
        );
        assert!(
            read::<32>(&[b"[{\"resource\":\"A\"}"]).1.is_err(),
            "not complete"
        );
    }

    fn table() -> [ResourceEntry<'static>; 3] {
        let mut cm = ResourceEntry::new("CMList");
        cm.caps.can_set = CanSet::Partial;
        cm.caps.encodings.mcoded7 = true;
        cm.media_types = &["application/json"];
        cm.columns = &[Column {
            kind: ColumnKind::Property("name"),
            title: "Name",
        }];
        cm.schema = Some(br#"{"type":"object"}"#);
        [
            ResourceEntry::new("ResourceList"),
            ResourceEntry::new("DeviceInfo"),
            cm,
        ]
    }

    #[test]
    fn writer_without_subscriptions_writes_can_subscribe_false() {
        // REQ-PE-SUB-022: the same table, "canSubscribe" true only with subscriptions.
        let mut x = ResourceEntry::new("X-State");
        x.caps.can_subscribe = true;
        let t = [x];
        let mut out = [0u8; 128];
        let mut w = ResourceListWriter::<128>::new(&t);
        let n = w.write_next(&mut out).unwrap();
        assert_eq!(
            &out[..n],
            br#"[{"resource":"X-State","canSubscribe":true}]"#
        );
        let mut w = ResourceListWriter::<128>::new(&t).without_subscriptions();
        let n = w.write_next(&mut out).unwrap();
        assert_eq!(&out[..n], br#"[{"resource":"X-State"}]"#);
    }

    fn write_all(t: &[ResourceEntry<'_>], room: usize) -> Vec<u8> {
        let mut w = ResourceListWriter::<256>::new(t);
        let mut out = Vec::new();
        loop {
            let mut buf = std::vec![0u8; room];
            let n = w.write_next(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        assert!(w.is_done());
        out
    }

    #[test]
    fn writer_streams_without_resource_list_and_defaults() {
        // REQ-PE-LIST-001, -004, -005, -007 to -009, REQ-PE-RES-004.
        let t = table();
        let whole = write_all(&t, 1000);
        assert_eq!(
            String::from_utf8(whole.clone()).unwrap(),
            r#"[{"resource":"DeviceInfo"},{"resource":"CMList","canSet":"partial","mediaTypes":["application/json"],"encodings":["ASCII","Mcoded7"],"schema":{"type":"object"},"columns":[{"property":"name","title":"Name"}]}]"#
        );
        for room in [1, 3, 17] {
            assert_eq!(write_all(&t, room), whole, "room {room}");
        }
        assert_eq!(
            write_all(&t[..1], 50),
            b"[]",
            "only ResourceList: empty array"
        );
        let (ev, end) = read::<32>(&[&whole]);
        end.unwrap();
        assert_eq!(ev.iter().filter(|e| matches!(e, E::R(..))).count(), 2);
        let mut all = ResourceEntry::new("A");
        all.write_defaults = true;
        let mut buf = [0u8; 400];
        let n = all.write(&mut buf).unwrap();
        assert!(core::str::from_utf8(&buf[..n])
            .unwrap()
            .contains(r#""canGet":true"#));
        assert!(ResourceEntry::new("").write(&mut buf).is_err());
        assert!(ResourceEntry::new("Bad-Name").write(&mut buf).is_err());
    }

    #[test]
    fn caps_check_statuses() {
        // REQ-PE-TXN-003, REQ-PE-RES-001, REQ-PE-HDR-010, REQ-PE-ENC-007, REQ-PE-FLOW-007, -008,
        // REQ-PE-STAT-012.
        let h = |f: fn(&mut Header<'static>)| {
            let mut h = Header {
                resource: Some(HeaderStr::Text("R")),
                ..Default::default()
            };
            f(&mut h);
            h
        };
        let base = DEFAULT_CAPS;
        assert_eq!(base.check(Operation::Get, &h(|_| {})), Ok(()));
        assert_eq!(
            base.check(Operation::Set, &h(|_| {})),
            Err(Status::NOT_ALLOWED)
        );
        let full = ResourceCaps {
            can_set: CanSet::Full,
            ..base
        };
        assert_eq!(full.check(Operation::Set, &h(|_| {})), Ok(()));
        assert_eq!(
            full.check(Operation::Set, &h(|h| h.set_partial = true)),
            Err(Status::NOT_ALLOWED)
        );
        // "partial" permits a Partial SET and a Full SET (REQ-PE-TXN-003).
        let partial = ResourceCaps {
            can_set: CanSet::Partial,
            ..base
        };
        assert_eq!(partial.check(Operation::Set, &h(|_| {})), Ok(()));
        assert_eq!(
            partial.check(Operation::Set, &h(|h| h.set_partial = true)),
            Ok(())
        );
        // REQ-PE-HDR-010: "flowControl":true in a Set without fcOnSet.
        assert_eq!(
            full.check(Operation::Set, &h(|h| h.flow_control = true)),
            Err(Status::FLOW_CONTROL_NOT_SUPPORTED)
        );
        assert_eq!(
            base.check(Operation::Subscribe, &h(|_| {})),
            Err(Status::NOT_ALLOWED)
        );
        let rid = ResourceCaps {
            require_res_id: true,
            ..base
        };
        assert_eq!(
            rid.check(Operation::Get, &h(|_| {})),
            Err(Status::BAD_REQUEST)
        );
        assert_eq!(
            base.check(Operation::Get, &h(|h| h.flow_control = true)),
            Err(Status::FLOW_CONTROL_NOT_SUPPORTED)
        );
        let fc = ResourceCaps {
            fc_on_set: true,
            can_set: CanSet::Full,
            ..base
        };
        assert_eq!(
            fc.check(Operation::Set, &h(|_| {})),
            Err(Status::FLOW_CONTROL_REQUIRED)
        );
        assert_eq!(
            fc.check(Operation::Set, &h(|h| h.flow_control = true)),
            Ok(())
        );
        assert_eq!(
            base.check(
                Operation::Get,
                &h(|h| h.mutual_encoding = Some(EncodingName::Mcoded7))
            ),
            Err(Status::UNSUPPORTED_MEDIA)
        );
        // An Mcoded7-only Resource: no "mutualEncoding" is "ASCII", so a Get or a Set gets 415;
        // a Subscription message with no "mutualEncoding" is not refused for it.
        let m7 = ResourceCaps {
            encodings: Encodings {
                ascii: false,
                mcoded7: true,
                zlib_mcoded7: false,
            },
            can_set: CanSet::Full,
            can_subscribe: true,
            ..base
        };
        assert_eq!(
            m7.check(Operation::Get, &h(|_| {})),
            Err(Status::UNSUPPORTED_MEDIA)
        );
        assert_eq!(
            m7.check(Operation::Set, &h(|_| {})),
            Err(Status::UNSUPPORTED_MEDIA)
        );
        assert_eq!(
            m7.check(
                Operation::Get,
                &h(|h| h.mutual_encoding = Some(EncodingName::Mcoded7))
            ),
            Ok(())
        );
        assert_eq!(m7.check(Operation::Subscribe, &h(|_| {})), Ok(()));
        let t = table();
        assert_eq!(
            find_resource(&t, &HeaderStr::Raw(b"CMList")).map(|e| e.resource),
            Some("CMList")
        );
        assert!(find_resource(&t, &HeaderStr::Text("Nope")).is_none());
    }

    /// The Resource declarations (name, caps, present) and the Columns (name, link, title,
    /// truncated) of one ResourceList, and the other events.
    #[allow(clippy::type_complexity)]
    fn read_full<const S: usize>(
        text: &[u8],
    ) -> (
        Vec<(String, ResourceCaps, Present)>,
        Vec<(String, bool, Option<String>, bool)>,
        Vec<ListError>,
    ) {
        let mut r = ResourceListReader::<S>::new();
        let (mut res, mut cols, mut errs) = (Vec::new(), Vec::new(), Vec::new());
        let mut ev = r.push(text);
        while let Some(e) = ev.next_event() {
            match e {
                ListEvent::Resource(d) => res.push((d.resource.to_string(), d.caps, d.present)),
                ListEvent::Column(c) => {
                    let (a, link) = match c.kind {
                        ColumnKind::Link(a) => (a, true),
                        ColumnKind::Property(a) => (a, false),
                    };
                    cols.push((
                        a.to_string(),
                        link,
                        c.title.map(|t| t.to_string()),
                        c.title_truncated,
                    ))
                }
                ListEvent::Invalid(e) => errs.push(e),
                _ => {}
            }
        }
        (res, cols, errs)
    }

    #[test]
    fn reader_defaults_and_present_flags() {
        // REQ-PE-LIST-003, -004, -007: an absent Property has its Table 79 / 80 default and is
        // marked absent; a present one is marked present, with its value.
        let (res, _, errs) = read_full::<32>(
            br#"[{"resource":"A"},{"resource":"B","canGet":false,"canSet":"full","canSubscribe":true,"requireResId":true,"fcOnGet":true,"fcOnSet":true,"mediaTypes":["a/b"],"encodings":["Mcoded7"],"schema":{},"canPaginate":true,"columns":[]}]"#,
        );
        assert!(errs.is_empty());
        let defaults = ResourceCaps {
            can_get: true,
            can_set: CanSet::None,
            can_subscribe: false,
            require_res_id: false,
            fc_on_get: false,
            fc_on_set: false,
            encodings: Encodings {
                ascii: true,
                mcoded7: false,
                zlib_mcoded7: false,
            },
            can_paginate: false,
        };
        let all = Present {
            can_get: true,
            can_set: true,
            can_subscribe: true,
            require_res_id: true,
            fc_on_get: true,
            fc_on_set: true,
            media_types: true,
            encodings: true,
            schema: true,
            can_paginate: true,
            columns: true,
        };
        let b = ResourceCaps {
            can_get: false,
            can_set: CanSet::Full,
            can_subscribe: true,
            require_res_id: true,
            fc_on_get: true,
            fc_on_set: true,
            encodings: Encodings {
                ascii: false,
                mcoded7: true,
                zlib_mcoded7: false,
            },
            can_paginate: true,
        };
        assert_eq!(
            res,
            [
                (String::from("A"), defaults, Present::default()),
                (String::from("B"), b, all)
            ]
        );
        assert_eq!(ResourceCaps::default(), defaults);
    }

    #[test]
    fn reader_object_errors() {
        // REQ-PE-LIST-004, -006, -007, -008: the error of each object; the reader continues
        // with the next object ("Z").
        let r37 = "r".repeat(37);
        for (obj, want) in [
            (
                std::format!(r#"{{"resource":"{r37}"}}"#),
                ListError::Invalid("resource"),
            ),
            (
                r#"{"resource":"A","canPaginate":"yes"}"#.into(),
                ListError::Invalid("canPaginate"),
            ),
            (
                r#"{"resource":"A","canPaginate":1}"#.into(),
                ListError::Invalid("canPaginate"),
            ),
            (
                r#"{"resource":"A","columns":{"property":"p"}}"#.into(),
                ListError::Invalid("columns"),
            ),
            (
                r#"{"resource":"A","columns":[{"title":"t"}]}"#.into(),
                ListError::Invalid("columns"),
            ),
            (
                r#"{"resource":"A","columns":[{"property":"p","title":"t"},{"link":"0123456789012345678901234567890123456"}]}"#.into(),
                ListError::Invalid("link"),
            ),
        ] {
            let text = std::format!(r#"[{obj},{{"resource":"Z"}}]"#);
            let (res, _, errs) = read_full::<64>(text.as_bytes());
            assert_eq!(errs, [want], "{obj}");
            assert_eq!(res.len(), 1, "{obj}");
            assert_eq!(res[0].0, "Z");
        }
        // A "property" or "link" longer than the caller buffer (8 bytes).
        for obj in [
            r#"{"resource":"A","columns":[{"property":"abcdefghi","title":"t"}]}"#,
            r#"{"resource":"A","columns":[{"link":"abcdefghi"}]}"#,
        ] {
            let text = std::format!(r#"[{obj},{{"resource":"Z"}}]"#);
            let (res, _, errs) = read_full::<8>(text.as_bytes());
            assert_eq!(
                (errs, res.len()),
                (std::vec![ListError::Overflow], 1),
                "{obj}"
            );
        }
        // A "schema" that is not valid JSON, or deeper than 64 levels: the JSON error ends the
        // list.
        let deep = std::format!("{}{}", "[".repeat(64), "]".repeat(64));
        for (schema, want) in [
            (String::from(r#"{"a":}"#), JsonError::Syntax),
            (std::format!(r#"{{"a":{deep}}}"#), JsonError::Depth),
        ] {
            let text = std::format!(r#"[{{"resource":"A","schema":{schema}}}]"#);
            let mut r = ResourceListReader::<32>::new();
            let mut ev = r.push(text.as_bytes());
            let mut got = Vec::new();
            while let Some(e) = ev.next_event() {
                if let ListEvent::Invalid(e) = e {
                    got.push(e);
                }
            }
            assert_eq!(got, [ListError::Json(want)], "{schema}");
            assert_eq!(r.finish(), Err(ListError::Json(want)));
        }
    }

    #[test]
    fn reader_column_titles() {
        // REQ-PE-LIST-008: a Column with no "title" is accepted; a "title" longer than the
        // caller buffer (8 bytes) is cut at a character boundary and marked.
        let (res, cols, errs) = read_full::<8>(
            "[{\"resource\":\"A\",\"columns\":[{\"property\":\"p\"},{\"link\":\"L\",\"title\":\"abcdefg\u{e9}\"},{\"property\":\"q\",\"title\":\"abcdefgh_\"},{\"property\":\"r\",\"title\":\"abc\"}]}]"
                .as_bytes(),
        );
        assert!(errs.is_empty());
        assert_eq!(res.len(), 1);
        assert_eq!(
            cols,
            [
                (String::from("p"), false, None, false),
                (String::from("L"), true, Some(String::from("abcdefg")), true),
                (
                    String::from("q"),
                    false,
                    Some(String::from("abcdefgh")),
                    true
                ),
                (String::from("r"), false, Some(String::from("abc")), false),
            ]
        );
        let (_, cols, _) = read_full::<8>(
            br#"[{"resource":"A","columns":[{"property":"p","title":"abcdefgh"}]}]"#,
        );
        assert_eq!(
            cols,
            [(
                String::from("p"),
                false,
                Some(String::from("abcdefgh")),
                false
            )]
        );
    }

    #[test]
    fn writer_errors_and_list_properties() {
        // REQ-PE-LIST-004: "resource" of 1 to 36 characters; REQ-PE-LIST-007: "canPaginate";
        // REQ-PE-LIST-008: a Column needs a "title".
        let mut buf = [0u8; 200];
        let a36 = "a".repeat(36);
        let a37 = "a".repeat(37);
        assert_eq!(
            ResourceEntry::new(&a37).write(&mut buf),
            Err(ListError::Invalid("resource"))
        );
        assert_eq!(
            ResourceEntry::new("").write(&mut buf),
            Err(ListError::Invalid("resource"))
        );
        let n = ResourceEntry::new(&a36).write(&mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            std::format!(r#"{{"resource":"{a36}"}}"#).as_bytes()
        );
        let mut p = ResourceEntry::new("P");
        p.caps.can_paginate = true;
        p.columns = &[Column {
            kind: ColumnKind::Link("Detail"),
            title: "More",
        }];
        let n = p.write(&mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            br#"{"resource":"P","canPaginate":true,"columns":[{"link":"Detail","title":"More"}]}"#
        );
        p.columns = &[Column {
            kind: ColumnKind::Property("name"),
            title: "",
        }];
        assert_eq!(p.write(&mut buf), Err(ListError::Missing("title")));
    }

    #[test]
    fn prop_reader_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..64), cut in 0usize..64)| {
            let c = cut.min(bytes.len());
            let _ = read::<8>(&[&bytes[..c], &bytes[c..]]);
        });
    }
}
