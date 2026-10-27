//! DNS-SD record data of a Host instance (M2-124-UM v1.0.1 section 4; RFC 6763, RFC 6762,
//! RFC 1035, RFC 2782).
//!
//! - The module builds and reads the PTR, SRV and TXT records of the service type
//!   "_midi2._udp" in the domain "local." (REQ-NET-DISC-002, -004). It is record data only: it
//!   has no mDNS responder, it does not send or receive mDNS messages, and it does not build A
//!   or AAAA records (DEC-012).
//! - [`HostRecords`] writes the PTR, SRV and TXT records together, as record names and RDATA in
//!   DNS wire format into a caller buffer ([`HostRecords::write`]), and as the text fields of
//!   platform DNS-SD interfaces ([`HostRecords::text_fields`]) (REQ-NET-DISC-002, -003).
//! - The readers [`read_ptr`] and [`read_srv`] take the DNS message and the offset of the
//!   RDATA, so that they follow name compression pointers (RFC 1035 4.1.4, N-57). Read names
//!   are label iterators ([`Name`]); labels compare with no ASCII case difference
//!   (RFC 1035 2.3.3). [`read_txt`] is lenient (N-58).
//! - No alloc (DEC-002). Nothing panics on received bytes.

use crate::identity::Identity;

/// Largest DNS label, in bytes (RFC 1035 2.3.4).
pub const MAX_LABEL: usize = 63;
/// Largest DNS name in wire format, in bytes (RFC 1035 2.3.4, 3.1).
pub const MAX_NAME: usize = 255;
/// The service type for platform interfaces (4.2 Table 4, REQ-NET-DISC-004).
pub const SERVICE_TYPE: &str = "_midi2._udp";
/// The domain for platform interfaces (4.3 Table 5, REQ-NET-DISC-004).
pub const DOMAIN: &str = "local.";
/// "_midi2._udp.local." in wire format, 19 bytes: the PTR record name (REQ-NET-DISC-004).
pub const SERVICE_NAME_WIRE: [u8; 19] = *b"\x06_midi2\x04_udp\x05local\x00";
/// DNS class IN (RFC 1035 3.2.4).
pub const CLASS_IN: u16 = 1;
/// Largest record set of [`HostRecords::write`], in bytes: the instance name (83), the SRV
/// RDATA (261) and the TXT RDATA (2 + 16 + 98 + 18 + 42 = 176).
pub const MAX_RECORD_SET: usize = 1
    + MAX_LABEL
    + SERVICE_NAME_WIRE.len()
    + 6
    + MAX_NAME
    + 2
    + TXT_KEY_ENDPOINT_NAME.len()
    + 1
    + crate::identity::MAX_ENDPOINT_NAME
    + TXT_KEY_PRODUCT_INSTANCE_ID.len()
    + 1
    + crate::identity::MAX_PRODUCT_INSTANCE_ID;

/// TXT key of the UMP Endpoint Name (4.4 Table 6).
pub const TXT_KEY_ENDPOINT_NAME: &str = "UMPEndpointName";
/// TXT key of the Product Instance Id (4.4 Table 6).
pub const TXT_KEY_PRODUCT_INSTANCE_ID: &str = "ProductInstanceId";

const SERVICE_LABELS: [&[u8]; 3] = [b"_midi2", b"_udp", b"local"];

/// Error of the DNS record writer and readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DnsError {
    /// An empty Service Instance Name or hostname (REQ-NET-DISC-006, -009).
    #[error("empty name")]
    Empty,
    /// A Service Instance Name or a label longer than 63 bytes (REQ-NET-DISC-006, -009).
    #[error("label too long")]
    LabelTooLong,
    /// An empty label in a hostname, for example "a..local", or no label before "local"
    /// (REQ-NET-DISC-009).
    #[error("empty label")]
    EmptyLabel,
    /// A name longer than 255 bytes in wire format (REQ-NET-DISC-008).
    #[error("name too long")]
    NameTooLong,
    /// A control character in a Service Instance Name, or a hostname byte outside 0x21 to
    /// 0x7E (REQ-NET-DISC-006, -009).
    #[error("invalid character")]
    InvalidCharacter,
    /// A hostname whose last label is not "local" (REQ-NET-DISC-009).
    #[error("not in the local domain")]
    NotLocal,
    /// A TTL of 0 or above 60 seconds (REQ-NET-DISC-014).
    #[error("invalid TTL")]
    InvalidTtl,
    /// The caller buffer is too small.
    #[error("buffer too small")]
    Overflow,
    /// The data ends inside a field or a name.
    #[error("truncated")]
    Truncated,
    /// A compression pointer that does not point to an earlier byte (RFC 1035 4.1.4).
    #[error("invalid compression pointer")]
    BadPointer,
    /// A label type 0x40 or 0x80 (RFC 6891 / reserved).
    #[error("invalid label type")]
    BadLabelType,
    /// The name is not "<instance>._midi2._udp.local." (REQ-NET-DISC-005).
    #[error("wrong service type")]
    WrongServiceType,
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// True for the text "_midi2._udp.local." or "_midi2._udp.local", with no ASCII case difference
/// (REQ-NET-DISC-004).
pub fn is_service_type(text: &str) -> bool {
    let t = text.strip_suffix('.').unwrap_or(text);
    t.eq_ignore_ascii_case("_midi2._udp.local")
}

/// The TTL of the PTR, SRV and TXT records: 1 to 60 seconds, default 60 (4.6, REQ-NET-DISC-014,
/// N-59).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ttl(u8);

impl Ttl {
    /// 60 seconds, the default and the maximum.
    pub const DEFAULT: Ttl = Ttl(60);

    /// A checked TTL: an error for 0 (the mDNS goodbye, sent by the responder) and above 60.
    pub const fn new(seconds: u32) -> Result<Ttl, DnsError> {
        if seconds == 0 || seconds > 60 {
            Err(DnsError::InvalidTtl)
        } else {
            Ok(Ttl(seconds as u8))
        }
    }

    /// The TTL in seconds.
    pub const fn seconds(self) -> u32 {
        self.0 as u32
    }
}

impl Default for Ttl {
    fn default() -> Self {
        Ttl::DEFAULT
    }
}

/// A Service Instance Name for the writer: 1 to 63 bytes of UTF-8 with no ASCII control
/// character (4.2, RFC 6763 4.1.1, REQ-NET-DISC-006). It is one DNS label; a dot stays in the
/// label (RFC 6763 4.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServiceInstanceName {
    buf: [u8; MAX_LABEL],
    len: u8,
}

impl core::fmt::Debug for ServiceInstanceName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ServiceInstanceName")
            .field(&self.as_str())
            .finish()
    }
}

impl ServiceInstanceName {
    /// A checked name.
    pub fn new(name: &str) -> Result<Self, DnsError> {
        if name.is_empty() {
            return Err(DnsError::Empty);
        }
        if name.len() > MAX_LABEL {
            return Err(DnsError::LabelTooLong);
        }
        if name.bytes().any(|b| b < 0x20 || b == 0x7F) {
            return Err(DnsError::InvalidCharacter);
        }
        let mut buf = [0; MAX_LABEL];
        buf[..name.len()].copy_from_slice(name.as_bytes());
        Ok(ServiceInstanceName {
            buf,
            len: name.len() as u8,
        })
    }

    /// The text, for platform interfaces.
    pub fn as_str(&self) -> &str {
        // The constructor took a `&str`, so the bytes are UTF-8.
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }

    /// The label bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..usize::from(self.len)]
    }
}

/// A hostname for the SRV writer, for example "desktop.local." (4.3 Table 5, REQ-NET-DISC-008,
/// -009). Each byte is in 0x21 to 0x7E, each label has 1 to 63 bytes, the last label is
/// "local", and the wire format is at most 255 bytes. The value keeps the text with the final
/// dot.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hostname {
    // Text with the final dot; wire length = text length + 1.
    buf: [u8; MAX_NAME - 1],
    len: u8,
}

impl core::fmt::Debug for Hostname {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Hostname").field(&self.as_str()).finish()
    }
}

impl Hostname {
    /// A checked hostname; the text may end with or without the final dot (AMB-M2-124-22).
    pub fn new(text: &str) -> Result<Self, DnsError> {
        let t = text.strip_suffix('.').unwrap_or(text);
        if t.is_empty() {
            return Err(DnsError::Empty);
        }
        if t.bytes().any(|b| !(0x21..=0x7E).contains(&b)) {
            return Err(DnsError::InvalidCharacter);
        }
        let mut last: &str = "";
        let mut labels = 0;
        for label in t.split('.') {
            labels += 1;
            if label.is_empty() {
                return Err(DnsError::EmptyLabel);
            }
            if label.len() > MAX_LABEL {
                return Err(DnsError::LabelTooLong);
            }
            last = label;
        }
        // Wire format: the text with the final dot, plus the first length byte.
        if t.len() + 2 > MAX_NAME {
            return Err(DnsError::NameTooLong);
        }
        if !last.eq_ignore_ascii_case("local") {
            return Err(DnsError::NotLocal);
        }
        // "local." alone names the domain, not a host.
        if labels < 2 {
            return Err(DnsError::EmptyLabel);
        }
        let mut buf = [0; MAX_NAME - 1];
        buf[..t.len()].copy_from_slice(t.as_bytes());
        buf[t.len()] = b'.';
        Ok(Hostname {
            buf,
            len: (t.len() + 1) as u8,
        })
    }

    /// The text with the final dot, for platform interfaces.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
    }

    /// The length in wire format.
    pub fn wire_len(&self) -> usize {
        usize::from(self.len) + 1
    }

    /// Writes the name in wire format, with the final root label 0x00; gives the length.
    pub fn write_wire(&self, buf: &mut [u8]) -> Result<usize, DnsError> {
        let mut w = Cursor { buf, pos: 0 };
        let text = self.as_str();
        for label in text.strip_suffix('.').unwrap_or(text).split('.') {
            w.label(label.as_bytes())?;
        }
        w.put(&[0])?;
        Ok(w.pos)
    }
}

struct Cursor<'b> {
    buf: &'b mut [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn put(&mut self, data: &[u8]) -> Result<(), DnsError> {
        let dst = self
            .buf
            .get_mut(self.pos..self.pos + data.len())
            .ok_or(DnsError::Overflow)?;
        dst.copy_from_slice(data);
        self.pos += data.len();
        Ok(())
    }

    fn label(&mut self, label: &[u8]) -> Result<(), DnsError> {
        self.put(&[label.len() as u8])?;
        self.put(label)
    }

    fn txt(&mut self, key: &str, value: &str) -> Result<(), DnsError> {
        // Key and value are checked types, so the string is at most 114 bytes.
        self.put(&[(key.len() + 1 + value.len()) as u8])?;
        self.put(key.as_bytes())?;
        self.put(b"=")?;
        self.put(value.as_bytes())
    }
}

/// DNS record type (RFC 1035 3.2.2, RFC 2782).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordType {
    /// PTR, 12.
    Ptr,
    /// TXT, 16.
    Txt,
    /// SRV, 33.
    Srv,
}

impl RecordType {
    /// The TYPE value.
    pub const fn value(self) -> u16 {
        match self {
            RecordType::Ptr => 12,
            RecordType::Txt => 16,
            RecordType::Srv => 33,
        }
    }
}

/// One written record: its name and RDATA in wire format, with no compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'b> {
    /// Record type.
    pub rtype: RecordType,
    /// TTL (REQ-NET-DISC-014).
    pub ttl: Ttl,
    /// Record name.
    pub name: &'b [u8],
    /// RDATA.
    pub rdata: &'b [u8],
}

/// The three records of one Host instance; the TXT record is always there (REQ-NET-DISC-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSet<'b> {
    /// "_midi2._udp.local." PTR "<instance>._midi2._udp.local." (REQ-NET-DISC-005).
    pub ptr: Record<'b>,
    /// "<instance>._midi2._udp.local." SRV 0 0 port hostname (REQ-NET-DISC-007).
    pub srv: Record<'b>,
    /// "<instance>._midi2._udp.local." TXT (REQ-NET-DISC-010, -015).
    pub txt: Record<'b>,
}

/// One TXT key/value pair for platform interfaces; `Display` gives "key=value".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxtEntry<'a> {
    /// The key.
    pub key: &'static str,
    /// The value.
    pub value: &'a str,
}

impl core::fmt::Display for TxtEntry<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}={}", self.key, self.value)
    }
}

/// The text fields of a Host instance for platform DNS-SD interfaces (for example
/// DNSServiceRegister, Avahi) (REQ-NET-DISC-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFields<'a> {
    /// Service Instance Name, with no escapes.
    pub instance_name: &'a str,
    /// "_midi2._udp".
    pub service_type: &'static str,
    /// "local.".
    pub domain: &'static str,
    /// Host UDP port.
    pub port: u16,
    /// Hostname with the final dot.
    pub hostname: &'a str,
    /// TTL.
    pub ttl: Ttl,
    /// TXT strings, in the order of the TXT RDATA (REQ-NET-DISC-015).
    pub txt: [TxtEntry<'a>; 2],
}

/// The PTR, SRV and TXT record set of one Host instance (REQ-NET-DISC-002, -003). Give one
/// value for each Host instance (REQ-NET-DISC-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostRecords {
    instance: ServiceInstanceName,
    identity: Identity,
    port: u16,
    hostname: Hostname,
    ttl: Ttl,
}

impl HostRecords {
    /// A record set with the TTL 60 seconds (REQ-NET-DISC-014). The identity gives the TXT
    /// data, so a set with no TXT data can not be made (REQ-NET-DISC-003).
    pub fn new(
        instance: ServiceInstanceName,
        identity: Identity,
        port: u16,
        hostname: Hostname,
    ) -> Self {
        HostRecords {
            instance,
            identity,
            port,
            hostname,
            ttl: Ttl::DEFAULT,
        }
    }

    /// Sets the TTL: an error for 0 or above 60 seconds (REQ-NET-DISC-014).
    pub fn with_ttl(mut self, seconds: u32) -> Result<Self, DnsError> {
        self.ttl = Ttl::new(seconds)?;
        Ok(self)
    }

    /// The TTL.
    pub fn ttl(&self) -> Ttl {
        self.ttl
    }

    /// Writes the three records into `buf` (at most [`MAX_RECORD_SET`] bytes) and gives their
    /// names and RDATA. The record names share the bytes of the PTR RDATA. The SRV RDATA is
    /// Priority 0, Weight 0, Port and Target, big-endian (REQ-NET-DISC-007).
    pub fn write<'b>(&self, buf: &'b mut [u8]) -> Result<RecordSet<'b>, DnsError> {
        let mut w = Cursor { buf, pos: 0 };
        // PTR RDATA = SRV and TXT record name.
        w.label(self.instance.as_bytes())?;
        w.put(&SERVICE_NAME_WIRE)?;
        let inst_end = w.pos;
        // SRV RDATA.
        w.put(&[0, 0, 0, 0])?;
        w.put(&self.port.to_be_bytes())?;
        let n = self
            .hostname
            .write_wire(w.buf.get_mut(w.pos..).ok_or(DnsError::Overflow)?)?;
        w.pos += n;
        let srv_end = w.pos;
        // TXT RDATA (REQ-NET-DISC-015).
        w.txt(TXT_KEY_ENDPOINT_NAME, self.identity.name.as_str())?;
        w.txt(
            TXT_KEY_PRODUCT_INSTANCE_ID,
            self.identity.product_instance_id.as_str(),
        )?;
        let txt_end = w.pos;
        let buf: &'b [u8] = w.buf;
        let instance_name = &buf[..inst_end];
        let ttl = self.ttl;
        Ok(RecordSet {
            ptr: Record {
                rtype: RecordType::Ptr,
                ttl,
                name: &buf[inst_end - SERVICE_NAME_WIRE.len()..inst_end],
                rdata: instance_name,
            },
            srv: Record {
                rtype: RecordType::Srv,
                ttl,
                name: instance_name,
                rdata: &buf[inst_end..srv_end],
            },
            txt: Record {
                rtype: RecordType::Txt,
                ttl,
                name: instance_name,
                rdata: &buf[srv_end..txt_end],
            },
        })
    }

    /// The text fields for platform DNS-SD interfaces (REQ-NET-DISC-002).
    pub fn text_fields(&self) -> TextFields<'_> {
        TextFields {
            instance_name: self.instance.as_str(),
            service_type: SERVICE_TYPE,
            domain: DOMAIN,
            port: self.port,
            hostname: self.hostname.as_str(),
            ttl: self.ttl,
            txt: [
                TxtEntry {
                    key: TXT_KEY_ENDPOINT_NAME,
                    value: self.identity.name.as_str(),
                },
                TxtEntry {
                    key: TXT_KEY_PRODUCT_INSTANCE_ID,
                    value: self.identity.product_instance_id.as_str(),
                },
            ],
        }
    }
}

/// A name read from a DNS message: a checked start offset. Its labels follow compression
/// pointers (RFC 1035 4.1.4); its wire length is at most 255 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Name<'a> {
    msg: &'a [u8],
    start: usize,
    wire_len: usize,
}

/// Reads the name at `offset` of `msg`; gives the name and the offset after it in the message.
/// Each compression pointer must point to an earlier byte than itself, so the read always
/// stops.
pub fn read_name(msg: &[u8], offset: usize) -> Result<(Name<'_>, usize), DnsError> {
    let mut pos = offset;
    let mut end = None;
    let mut len = 0usize;
    loop {
        let b = *msg.get(pos).ok_or(DnsError::Truncated)?;
        match b & 0xC0 {
            0x00 => {
                let l = usize::from(b);
                len += 1 + l;
                if len > MAX_NAME {
                    return Err(DnsError::NameTooLong);
                }
                if l == 0 {
                    let end = end.unwrap_or(pos + 1);
                    return Ok((
                        Name {
                            msg,
                            start: offset,
                            wire_len: len,
                        },
                        end,
                    ));
                }
                if msg.len() < pos + 1 + l {
                    return Err(DnsError::Truncated);
                }
                pos += 1 + l;
            }
            0xC0 => {
                let b2 = *msg.get(pos + 1).ok_or(DnsError::Truncated)?;
                let target = (usize::from(b & 0x3F) << 8) | usize::from(b2);
                if target >= pos {
                    return Err(DnsError::BadPointer);
                }
                end.get_or_insert(pos + 2);
                pos = target;
            }
            _ => return Err(DnsError::BadLabelType),
        }
    }
}

/// The labels of a [`Name`], with no root label.
#[derive(Debug, Clone)]
pub struct Labels<'a> {
    msg: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for Labels<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        // The name was checked by `read_name`; the `get` calls only keep this panic-free.
        loop {
            let b = *self.msg.get(self.pos)?;
            if b & 0xC0 == 0xC0 {
                let b2 = *self.msg.get(self.pos + 1)?;
                self.pos = (usize::from(b & 0x3F) << 8) | usize::from(b2);
                continue;
            }
            if b == 0 {
                return None;
            }
            let l = usize::from(b);
            let label = self.msg.get(self.pos + 1..self.pos + 1 + l)?;
            self.pos += 1 + l;
            return Some(label);
        }
    }
}

impl<'a> Name<'a> {
    /// The labels, first to last, with no root label.
    pub fn labels(&self) -> Labels<'a> {
        Labels {
            msg: self.msg,
            pos: self.start,
        }
    }

    /// The length in wire format with no compression (at most 255).
    pub fn wire_len(&self) -> usize {
        self.wire_len
    }

    /// True when the labels are `labels`, with no ASCII case difference (RFC 1035 2.3.3).
    pub fn eq_labels<'b>(&self, labels: impl IntoIterator<Item = &'b [u8]>) -> bool {
        let mut a = self.labels();
        let mut b = labels.into_iter();
        loop {
            match (a.next(), b.next()) {
                (None, None) => return true,
                (Some(x), Some(y)) if eq_ignore_case(x, y) => {}
                _ => return false,
            }
        }
    }

    /// True for the name "_midi2._udp.local." (REQ-NET-DISC-004).
    pub fn is_service_type(&self) -> bool {
        self.eq_labels(SERVICE_LABELS)
    }

    /// The Service Instance Name of "<instance>._midi2._udp.local." (the PTR RDATA, the SRV and
    /// TXT record names), as bytes; an error for another name (REQ-NET-DISC-005, -006).
    pub fn service_instance(&self) -> Result<&'a [u8], DnsError> {
        let mut l = self.labels();
        let inst = l.next().ok_or(DnsError::WrongServiceType)?;
        let rest = Name {
            msg: self.msg,
            start: l.pos,
            wire_len: 0,
        };
        if rest.is_service_type() {
            Ok(inst)
        } else {
            Err(DnsError::WrongServiceType)
        }
    }

    /// Writes the name in wire format with no compression; gives the length.
    pub fn write_wire(&self, buf: &mut [u8]) -> Result<usize, DnsError> {
        let mut w = Cursor { buf, pos: 0 };
        for label in self.labels() {
            w.label(label)?;
        }
        w.put(&[0])?;
        Ok(w.pos)
    }
}

/// The RDATA of a read PTR record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtrRdata<'a> {
    /// The full name "<instance>._midi2._udp.local.".
    pub name: Name<'a>,
    /// The Service Instance Name, 1 to 63 bytes of any value (REQ-NET-DISC-006).
    pub instance: &'a [u8],
}

/// Reads the PTR RDATA at `offset` of the DNS message `msg` (REQ-NET-DISC-002, -005, N-57); an
/// error when the name does not end with "_midi2._udp.local.".
pub fn read_ptr(msg: &[u8], offset: usize) -> Result<PtrRdata<'_>, DnsError> {
    let (name, _) = read_name(msg, offset)?;
    let instance = name.service_instance()?;
    Ok(PtrRdata { name, instance })
}

/// The RDATA of a read SRV record (RFC 2782).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SrvRdata<'a> {
    /// Priority, any value.
    pub priority: u16,
    /// Weight, any value.
    pub weight: u16,
    /// Host UDP port.
    pub port: u16,
    /// Target hostname, any name of at most 255 bytes in wire format (REQ-NET-DISC-008, -009).
    pub target: Name<'a>,
}

/// Reads the SRV RDATA at `offset` of the DNS message `msg` (REQ-NET-DISC-002, -007, N-57).
pub fn read_srv(msg: &[u8], offset: usize) -> Result<SrvRdata<'_>, DnsError> {
    let n = msg
        .get(offset..)
        .and_then(|r| r.first_chunk::<6>())
        .ok_or(DnsError::Truncated)?;
    let (target, _) = read_name(msg, offset + 6)?;
    Ok(SrvRdata {
        priority: u16::from_be_bytes([n[0], n[1]]),
        weight: u16::from_be_bytes([n[2], n[3]]),
        port: u16::from_be_bytes([n[4], n[5]]),
        target,
    })
}

/// The strings of a TXT RDATA (RFC 6763 6.1). A string whose length goes past the end stops
/// the iteration and is not given.
#[derive(Debug, Clone)]
pub struct TxtStrings<'a> {
    rest: &'a [u8],
}

/// The strings of the TXT RDATA `rdata`.
pub fn txt_strings(rdata: &[u8]) -> TxtStrings<'_> {
    TxtStrings { rest: rdata }
}

impl<'a> Iterator for TxtStrings<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let (&len, body) = self.rest.split_first()?;
        match body.split_at_checked(usize::from(len)) {
            Some((s, rest)) => {
                self.rest = rest;
                Some(s)
            }
            None => {
                self.rest = &[];
                None
            }
        }
    }
}

/// The values of a read TXT record (REQ-NET-DISC-016). An absent or empty value is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxtInfo<'a> {
    /// UMP Endpoint Name bytes, not checked.
    pub endpoint_name: Option<&'a [u8]>,
    /// Product Instance Id bytes, not checked.
    pub product_instance_id: Option<&'a [u8]>,
}

/// Reads the TXT RDATA (REQ-NET-DISC-010, -016, N-58). Each string splits at the first "=";
/// keys compare with no ASCII case difference; unknown keys and strings with no "=" are
/// ignored; the first string of a key wins. It never rejects the record.
pub fn read_txt(rdata: &[u8]) -> TxtInfo<'_> {
    let mut name_seen = false;
    let mut id_seen = false;
    let mut info = TxtInfo::default();
    for s in txt_strings(rdata) {
        let Some(eq) = s.iter().position(|b| *b == b'=') else {
            continue;
        };
        let (key, value) = (&s[..eq], &s[eq + 1..]);
        let value = (!value.is_empty()).then_some(value);
        if eq_ignore_case(key, TXT_KEY_ENDPOINT_NAME.as_bytes()) {
            if !name_seen {
                name_seen = true;
                info.endpoint_name = value;
            }
        } else if eq_ignore_case(key, TXT_KEY_PRODUCT_INSTANCE_ID.as_bytes()) && !id_seen {
            id_seen = true;
            info.product_instance_id = value;
        }
    }
    info
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    const INSTANCE: &str = "173ACE-MIDIWorkbench";

    fn records(port: u16, host: &str) -> HostRecords {
        HostRecords::new(
            ServiceInstanceName::new(INSTANCE).unwrap(),
            Identity::new("MIDI Workbench", "173ACEHDI3nVAUiFH").unwrap(),
            port,
            Hostname::new(host).unwrap(),
        )
    }

    fn ptr_rdata_vector() -> Vec<u8> {
        let mut v = Vec::from([0x14]);
        v.extend_from_slice(INSTANCE.as_bytes());
        v.extend_from_slice(b"\x06_midi2\x04_udp\x05local\x00");
        v
    }

    #[test]
    fn service_type_wire_bytes() {
        // REQ-NET-DISC-004: 19 bytes, checked against the requirement vector.
        assert_eq!(
            SERVICE_NAME_WIRE,
            [
                0x06, 0x5f, 0x6d, 0x69, 0x64, 0x69, 0x32, 0x04, 0x5f, 0x75, 0x64, 0x70, 0x05, 0x6c,
                0x6f, 0x63, 0x61, 0x6c, 0x00
            ]
        );
        let mut buf = [0; MAX_RECORD_SET];
        let set = records(5673, "desktop.local.").write(&mut buf).unwrap();
        assert_eq!(set.ptr.name, &SERVICE_NAME_WIRE);
        assert_eq!(set.ptr.rtype.value(), 12);
    }

    #[test]
    fn service_type_text_with_and_without_final_dot() {
        // REQ-NET-DISC-004.
        assert!(is_service_type("_midi2._udp.local."));
        assert!(is_service_type("_midi2._udp.local"));
        assert!(is_service_type("_MIDI2._UDP.Local."));
        assert!(!is_service_type("_midi2._tcp.local."));
        assert!(!is_service_type("_midi2._udp"));
    }

    #[test]
    fn service_type_labels_case_insensitive() {
        // REQ-NET-DISC-004: the reader compares labels with no ASCII case difference.
        let msg = b"\x06_MIDI2\x04_Udp\x05LOCAL\x00";
        let (n, end) = read_name(msg, 0).unwrap();
        assert_eq!(end, 19);
        assert!(n.is_service_type());
    }

    #[test]
    fn ptr_record_spec_vector() {
        // REQ-NET-DISC-005, 4.2 example.
        let mut buf = [0; MAX_RECORD_SET];
        let set = records(5673, "desktop.local.").write(&mut buf).unwrap();
        let v = ptr_rdata_vector();
        assert_eq!(v.len(), 40);
        assert_eq!(set.ptr.rdata, &v[..]);
        let p = read_ptr(&v, 0).unwrap();
        assert_eq!(p.instance, INSTANCE.as_bytes());
        assert_eq!(p.name.wire_len(), 40);
    }

    #[test]
    fn ptr_reader_rejects_other_service_type() {
        // REQ-NET-DISC-005.
        assert_eq!(
            read_ptr(b"\x01a\x06_midi2\x04_tcp\x05local\x00", 0),
            Err(DnsError::WrongServiceType)
        );
        assert_eq!(
            read_ptr(&SERVICE_NAME_WIRE, 0),
            Err(DnsError::WrongServiceType)
        );
        assert_eq!(
            read_ptr(b"\x01a\x01b\x06_midi2\x04_udp\x05local\x00", 0),
            Err(DnsError::WrongServiceType)
        );
    }

    #[test]
    fn ptr_reader_follows_compression_pointer() {
        // REQ-NET-DISC-002, N-57: the service type at offset 12, the RDATA points to it.
        let mut msg = Vec::from([0u8; 12]);
        msg.extend_from_slice(&SERVICE_NAME_WIRE);
        let rdata_at = msg.len();
        msg.push(0x14);
        msg.extend_from_slice(INSTANCE.as_bytes());
        msg.extend_from_slice(&[0xC0, 12]);
        let p = read_ptr(&msg, rdata_at).unwrap();
        assert_eq!(p.instance, INSTANCE.as_bytes());
        let mut out = [0; 255];
        let n = p.name.write_wire(&mut out).unwrap();
        assert_eq!(&out[..n], &ptr_rdata_vector()[..]);
        assert_eq!(read_name(&msg, rdata_at).unwrap().1, msg.len());
    }

    #[test]
    fn name_reader_rejects_bad_pointers_and_labels() {
        // Nothing panics on received bytes; loops stop.
        assert_eq!(read_name(&[0xC0, 0x00], 0), Err(DnsError::BadPointer));
        assert_eq!(
            read_name(&[0x01, b'a', 0xC0, 0x00], 0),
            Err(DnsError::NameTooLong)
        );
        assert_eq!(read_name(&[0x40], 0), Err(DnsError::BadLabelType));
        assert_eq!(read_name(&[0x03, b'a'], 0), Err(DnsError::Truncated));
        assert_eq!(read_name(&[0xC0], 0), Err(DnsError::Truncated));
        assert_eq!(read_name(&[], 0), Err(DnsError::Truncated));
    }

    #[test]
    fn instance_name_writer_rules() {
        // REQ-NET-DISC-006.
        assert_eq!(ServiceInstanceName::new(""), Err(DnsError::Empty));
        assert_eq!(
            ServiceInstanceName::new(&"x".repeat(64)),
            Err(DnsError::LabelTooLong)
        );
        assert!(ServiceInstanceName::new(&"x".repeat(63)).is_ok());
        assert_eq!(
            ServiceInstanceName::new("a\tb"),
            Err(DnsError::InvalidCharacter)
        );
        assert_eq!(
            ServiceInstanceName::new("a\x7Fb"),
            Err(DnsError::InvalidCharacter)
        );
        let n = ServiceInstanceName::new("Studio A. Synthé").unwrap();
        assert_eq!(n.as_str(), "Studio A. Synthé");
    }

    #[test]
    fn instance_name_reader_accepts_any_bytes() {
        // REQ-NET-DISC-006: the dot stays in the label; control bytes are accepted.
        let mut msg = Vec::from([0x05, b'a', b'.', 0x00, 0xFF, 0x7F]);
        msg.extend_from_slice(&SERVICE_NAME_WIRE);
        let p = read_ptr(&msg, 0).unwrap();
        assert_eq!(p.instance, &[b'a', b'.', 0x00, 0xFF, 0x7F]);
    }

    #[test]
    fn srv_record_spec_vector() {
        // REQ-NET-DISC-007, 4.3 example.
        let mut buf = [0; MAX_RECORD_SET];
        let set = records(5673, "desktop.local.").write(&mut buf).unwrap();
        let rdata = b"\x00\x00\x00\x00\x16\x29\x07desktop\x05local\x00";
        assert_eq!(rdata.len(), 21);
        assert_eq!(set.srv.rdata, rdata);
        assert_eq!(set.srv.ttl.seconds(), 60);
        assert_eq!(set.srv.rtype.value(), 33);
        assert_eq!(set.srv.name, &ptr_rdata_vector()[..]);
        let s = read_srv(rdata, 0).unwrap();
        assert_eq!((s.priority, s.weight, s.port), (0, 0, 5673));
        assert!(s
            .target
            .eq_labels([b"desktop".as_slice(), b"local".as_slice()]));
    }

    #[test]
    fn srv_reader_accepts_any_priority_weight_and_target() {
        // REQ-NET-DISC-007, -009.
        let rdata = b"\x12\x34\xAB\xCD\x00\x01\x03a b\x03com\x00";
        let s = read_srv(rdata, 0).unwrap();
        assert_eq!((s.priority, s.weight, s.port), (0x1234, 0xABCD, 1));
        assert!(s.target.eq_labels([b"a b".as_slice(), b"COM".as_slice()]));
        assert_eq!(read_srv(&rdata[..5], 0), Err(DnsError::Truncated));
    }

    #[test]
    fn srv_hostname_length_limit() {
        // REQ-NET-DISC-008: 255 bytes wire format is the limit for writer and reader.
        let l = "a".repeat(63);
        // 3 * 64 + 56 + 6 ("local") + 1 = 255.
        let ok = std::format!("{l}.{l}.{l}.{}.local.", "b".repeat(55));
        let h = Hostname::new(&ok).unwrap();
        assert_eq!(h.wire_len(), 255);
        let long = std::format!("{l}.{l}.{l}.{}.local.", "b".repeat(56));
        assert_eq!(Hostname::new(&long), Err(DnsError::NameTooLong));
        let set = HostRecords::new(
            ServiceInstanceName::new(INSTANCE).unwrap(),
            Identity::new("a", "b").unwrap(),
            1,
            h,
        );
        let mut buf = [0; MAX_RECORD_SET];
        let srv = Vec::from(set.write(&mut buf).unwrap().srv.rdata);
        assert_eq!(srv.len(), 261);
        assert_eq!(read_srv(&srv, 0).unwrap().target.wire_len(), 255);
        // Reader: 256 bytes is refused.
        let mut msg = Vec::from([0u8; 6]);
        for _ in 0..4 {
            msg.push(63);
            msg.extend_from_slice(l.as_bytes());
        }
        msg.push(0);
        assert_eq!(read_srv(&msg, 0), Err(DnsError::NameTooLong));
    }

    #[test]
    fn hostname_writer_rules() {
        // REQ-NET-DISC-009.
        assert_eq!(
            Hostname::new("desktop.local").unwrap().as_str(),
            "desktop.local."
        );
        assert_eq!(
            Hostname::new("desktop.LOCAL.").unwrap().as_str(),
            "desktop.LOCAL."
        );
        assert_eq!(Hostname::new(""), Err(DnsError::Empty));
        assert_eq!(Hostname::new("."), Err(DnsError::Empty));
        assert_eq!(Hostname::new("a..local"), Err(DnsError::EmptyLabel));
        assert_eq!(Hostname::new(".local"), Err(DnsError::EmptyLabel));
        assert_eq!(Hostname::new("local."), Err(DnsError::EmptyLabel));
        assert_eq!(Hostname::new("LOCAL"), Err(DnsError::EmptyLabel));
        assert_eq!(Hostname::new("a b.local"), Err(DnsError::InvalidCharacter));
        assert_eq!(Hostname::new("é.local"), Err(DnsError::InvalidCharacter));
        assert_eq!(
            Hostname::new("a\x7F.local"),
            Err(DnsError::InvalidCharacter)
        );
        assert_eq!(
            Hostname::new(&std::format!("{}.local", "x".repeat(64))),
            Err(DnsError::LabelTooLong)
        );
        assert_eq!(Hostname::new("desktop.com."), Err(DnsError::NotLocal));
        let mut out = [0; 16];
        let n = Hostname::new("desktop.local")
            .unwrap()
            .write_wire(&mut out)
            .unwrap();
        assert_eq!(&out[..n], b"\x07desktop\x05local\x00");
        assert_eq!(
            Hostname::new("desktop.local")
                .unwrap()
                .write_wire(&mut out[..5]),
            Err(DnsError::Overflow)
        );
    }

    #[test]
    fn txt_record_spec_vector() {
        // REQ-NET-DISC-010, -015, 4.4 example.
        let mut buf = [0; MAX_RECORD_SET];
        let set = records(5673, "desktop.local.").write(&mut buf).unwrap();
        let mut v = Vec::from([0x1E]);
        v.extend_from_slice(b"UMPEndpointName=MIDI Workbench");
        v.push(0x23);
        v.extend_from_slice(b"ProductInstanceId=173ACEHDI3nVAUiFH");
        assert_eq!(v.len(), 67);
        assert_eq!(set.txt.rdata, &v[..]);
        assert_eq!(set.txt.rtype.value(), 16);
        assert_eq!(set.txt.name, &ptr_rdata_vector()[..]);
        let i = read_txt(&v);
        assert_eq!(i.endpoint_name, Some(b"MIDI Workbench".as_slice()));
        assert_eq!(i.product_instance_id, Some(b"173ACEHDI3nVAUiFH".as_slice()));
    }

    #[test]
    fn record_set_always_has_txt() {
        // REQ-NET-DISC-003, -002: one write gives all three records and the text fields.
        let mut buf = [0; MAX_RECORD_SET];
        let r = records(5673, "desktop.local");
        let set = r.write(&mut buf).unwrap();
        assert!(!set.txt.rdata.is_empty());
        assert_eq!(set.txt.ttl, set.ptr.ttl);
        let t = r.text_fields();
        assert_eq!(t.instance_name, INSTANCE);
        assert_eq!((t.service_type, t.domain), ("_midi2._udp", "local."));
        assert_eq!((t.port, t.hostname), (5673, "desktop.local."));
        assert_eq!(
            std::format!("{}", t.txt[0]),
            "UMPEndpointName=MIDI Workbench"
        );
        assert_eq!(
            std::format!("{}", t.txt[1]),
            "ProductInstanceId=173ACEHDI3nVAUiFH"
        );
        // The buffer is too small: an error, no partial set.
        let mut small = [0; 40 + 21 + 66];
        assert_eq!(r.write(&mut small), Err(DnsError::Overflow));
        let mut exact = [0; 40 + 21 + 67];
        assert!(r.write(&mut exact).is_ok());
    }

    #[test]
    fn record_set_fits_max_size() {
        // REQ-NET-DISC-002: the largest set fits in MAX_RECORD_SET.
        let l = "a".repeat(63);
        let host = std::format!("{l}.{l}.{l}.{}.local", "b".repeat(55));
        let r = HostRecords::new(
            ServiceInstanceName::new(&"i".repeat(63)).unwrap(),
            Identity::new(&"n".repeat(98), &"p".repeat(42)).unwrap(),
            1,
            Hostname::new(&host).unwrap(),
        );
        let mut buf = [0; MAX_RECORD_SET];
        let set = r.write(&mut buf).unwrap();
        assert_eq!(
            set.ptr.rdata.len() + set.srv.rdata.len() + set.txt.rdata.len(),
            MAX_RECORD_SET
        );
    }

    #[test]
    fn ttl_rules() {
        // REQ-NET-DISC-014, N-59.
        assert_eq!(Ttl::default().seconds(), 60);
        assert_eq!(records(1, "a.local").ttl().seconds(), 60);
        assert_eq!(Ttl::new(0), Err(DnsError::InvalidTtl));
        assert_eq!(Ttl::new(61), Err(DnsError::InvalidTtl));
        assert_eq!(Ttl::new(1).unwrap().seconds(), 1);
        let r = records(1, "a.local").with_ttl(30).unwrap();
        let mut buf = [0; MAX_RECORD_SET];
        let set = r.write(&mut buf).unwrap();
        assert_eq!(
            [set.ptr.ttl, set.srv.ttl, set.txt.ttl].map(Ttl::seconds),
            [30; 3]
        );
        assert_eq!(r.text_fields().ttl.seconds(), 30);
        assert_eq!(records(1, "a.local").with_ttl(0), Err(DnsError::InvalidTtl));
        assert_eq!(
            records(1, "a.local").with_ttl(3600),
            Err(DnsError::InvalidTtl)
        );
    }

    #[test]
    fn txt_reader_is_lenient() {
        // REQ-NET-DISC-016, N-58.
        let mut v = Vec::new();
        for s in [
            b"flag".as_slice(),
            b"other=1",
            b"umpendpointname=First",
            b"UMPEndpointName=Second",
            b"PRODUCTINSTANCEID=",
            b"ProductInstanceId=late",
            b"UMPEndpointNamex=no",
        ] {
            v.push(s.len() as u8);
            v.extend_from_slice(s);
        }
        let i = read_txt(&v);
        assert_eq!(i.endpoint_name, Some(b"First".as_slice()));
        // The first string of the key is empty: absent, and the later one is not used.
        assert_eq!(i.product_instance_id, None);
        // Value split at the first "=", and no length limit.
        let long = std::format!("UMPEndpointName=a=b{}", "c".repeat(200));
        let mut v = Vec::from([long.len() as u8]);
        v.extend_from_slice(long.as_bytes());
        assert_eq!(read_txt(&v).endpoint_name, Some(&long.as_bytes()[16..]));
        // Empty TXT (the Zephyr sample) and a single zero byte: all absent.
        assert_eq!(read_txt(&[]), TxtInfo::default());
        assert_eq!(read_txt(&[0]), TxtInfo::default());
        // A truncated string is ignored.
        assert_eq!(read_txt(b"\x20UMPEndpointName=x"), TxtInfo::default());
    }

    #[test]
    fn prop_record_set_round_trip() {
        // REQ-NET-DISC-005, -007, -010: what the writer writes, the readers read back.
        proptest!(|(inst in "[ -~]{1,63}", host in "[a-z0-9-]{1,40}", port in any::<u16>(),
                    name in "[ -~]{1,98}", id in "[ -~]{1,42}")| {
            let r = HostRecords::new(
                ServiceInstanceName::new(&inst).unwrap(),
                Identity::new(&name, &id).unwrap(),
                port,
                Hostname::new(&std::format!("{host}.local")).unwrap(),
            );
            let mut buf = [0; MAX_RECORD_SET];
            let set = r.write(&mut buf).unwrap();
            prop_assert!(read_name(set.ptr.name, 0).unwrap().0.is_service_type());
            prop_assert_eq!(read_ptr(set.ptr.rdata, 0).unwrap().instance, inst.as_bytes());
            let s = read_srv(set.srv.rdata, 0).unwrap();
            prop_assert_eq!(s.port, port);
            prop_assert!(s.target.eq_labels([host.as_bytes(), b"local".as_slice()]));
            let t = read_txt(set.txt.rdata);
            prop_assert_eq!(t.endpoint_name, Some(name.as_bytes()));
            prop_assert_eq!(t.product_instance_id, Some(id.as_bytes()));
        });
    }

    #[test]
    fn prop_readers_never_panic() {
        // Nothing panics on received bytes.
        proptest!(|(msg in proptest::collection::vec(any::<u8>(), 0..300), off in 0usize..310)| {
            if let Ok((n, _)) = read_name(&msg, off) {
                prop_assert!(n.labels().count() <= 127);
                let mut out = [0; MAX_NAME];
                prop_assert_eq!(n.write_wire(&mut out).unwrap(), n.wire_len());
                let _ = n.service_instance();
            }
            let _ = read_ptr(&msg, off);
            let _ = read_srv(&msg, off);
            let _ = read_txt(msg.get(off..).unwrap_or(&[]));
        });
    }
}
