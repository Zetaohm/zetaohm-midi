//! A small mDNS responder for one Network MIDI 2.0 Host (M2-124-UM v1.0.1 section 4,
//! RFC 6762). `zmidi-net` gives the PTR, SRV and TXT record data (DEC-012); this module adds the A
//! record, the DNS message and the rules of RFC 6762 that one fixed service needs:
//!
//! - It answers a question for "_midi2._udp.local.", for the Service Instance Name or for the
//!   hostname, of any type and class IN, with the PTR, SRV, TXT and A records (RFC 6762 6).
//! - A query from a port that is not 5353 (a one-shot query, RFC 6762 5.1) gets a unicast reply
//!   with the query ID and the question, no cache-flush bit and a TTL of at most 10 s
//!   (RFC 6762 6.7). Other queries get a multicast reply.
//! - [`Responder::announcement`] is the unsolicited reply of RFC 6762 8.3: send it at start-up,
//!   then again after 1 s.
//!
//! No probing (RFC 6762 8.1): the instance name and the hostname are taken as unique. No alloc.

use zmidi_net::dns::{read_name, HostRecords, Hostname, Name, MAX_NAME, MAX_RECORD_SET};

/// The mDNS port (RFC 6762 3).
pub const PORT: u16 = 5353;
const TYPE_A: u16 = 1;
const TYPE_ANY: u16 = 255;
const CLASS_IN: u16 = 1;
/// The cache-flush bit of the record class (RFC 6762 10.2).
const CACHE_FLUSH: u16 = 0x8000;
/// The largest TTL of a reply to a one-shot query (RFC 6762 6.7).
const LEGACY_TTL: u32 = 10;
/// Space for the reply.
pub const MAX_REPLY: usize = 12 + 512 + 4 * 10 + MAX_RECORD_SET + 2 * MAX_NAME + 4;

/// The records of one Host and its IPv4 address.
pub struct Responder {
    records: [u8; MAX_RECORD_SET],
    ptr: (usize, usize, usize, usize),
    srv: (usize, usize),
    txt: (usize, usize),
    ttl: u32,
    host: [u8; MAX_NAME],
    host_len: usize,
    ip: [u8; 4],
}

/// One question of a query: its name and type.
#[derive(Clone, Copy)]
struct Question<'a> {
    name: Name<'a>,
    qtype: u16,
}

fn be16(m: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*m.get(at)?, *m.get(at + 1)?]))
}

/// The offset of `inner` in the buffer that starts at address `base`.
fn offset_in(base: usize, inner: &[u8]) -> usize {
    inner.as_ptr() as usize - base
}

impl Responder {
    /// The responder for `records` (from `Host::host_records`), `hostname` and the address `ip`.
    pub fn new(records: &HostRecords, hostname: &Hostname, ip: [u8; 4]) -> Option<Self> {
        let mut r = Responder {
            records: [0; MAX_RECORD_SET],
            ptr: (0, 0, 0, 0),
            srv: (0, 0),
            txt: (0, 0),
            ttl: records.ttl().seconds(),
            host: [0; MAX_NAME],
            host_len: 0,
            ip,
        };
        let base = r.records.as_ptr() as usize;
        let set = records.write(&mut r.records).ok()?;
        r.ptr = (
            offset_in(base, set.ptr.name),
            set.ptr.name.len(),
            offset_in(base, set.ptr.rdata),
            set.ptr.rdata.len(),
        );
        r.srv = (offset_in(base, set.srv.rdata), set.srv.rdata.len());
        r.txt = (offset_in(base, set.txt.rdata), set.txt.rdata.len());
        r.host_len = hostname.write_wire(&mut r.host).ok()?;
        Some(r)
    }

    fn service(&self) -> &[u8] {
        &self.records[self.ptr.0..self.ptr.0 + self.ptr.1]
    }

    /// "<instance>._midi2._udp.local.": the PTR RDATA, and the SRV and TXT record name.
    fn instance(&self) -> &[u8] {
        &self.records[self.ptr.2..self.ptr.2 + self.ptr.3]
    }

    fn hostname(&self) -> &[u8] {
        &self.host[..self.host_len]
    }

    /// The unsolicited reply of RFC 6762 8.3, to send to 224.0.0.251:5353.
    pub fn announcement(&self, out: &mut [u8]) -> Option<usize> {
        self.reply(0, None, false, out)
    }

    /// The reply to the query `m` from UDP port `src_port`: the reply length and true for a
    /// unicast reply to the sender (else multicast). `None`: no question for this Host.
    pub fn answer(&self, m: &[u8], src_port: u16, out: &mut [u8]) -> Option<(usize, bool)> {
        let flags = be16(m, 2)?;
        if flags & 0x8000 != 0 {
            return None; // a reply, not a query
        }
        let qd = be16(m, 4)?;
        let mut at = 12;
        let mut ours = false;
        for _ in 0..qd {
            let (name, end) = read_name(m, at).ok()?;
            let q = Question {
                name,
                qtype: be16(m, end)?,
            };
            let class = be16(m, end + 2)? & 0x7FFF;
            ours |= class == CLASS_IN && self.wants(&q);
            at = end + 4;
        }
        if !ours {
            return None;
        }
        let legacy = src_port != PORT;
        let question = legacy.then(|| &m[12..at]);
        let id = if legacy { be16(m, 0)? } else { 0 };
        let n = self.reply(id, question.map(|q| (qd, q)), legacy, out)?;
        Some((n, legacy))
    }

    fn wants(&self, q: &Question<'_>) -> bool {
        let same = |wire: &[u8]| {
            let mut probe = [0u8; MAX_NAME];
            q.name
                .write_wire(&mut probe)
                .is_ok_and(|n| eq_ignore_case(&probe[..n], wire))
        };
        if q.name.is_service_type() {
            return true;
        }
        if same(self.instance()) {
            return true;
        }
        same(self.hostname()) && matches!(q.qtype, TYPE_A | TYPE_ANY)
    }

    fn reply(
        &self,
        id: u16,
        question: Option<(u16, &[u8])>,
        legacy: bool,
        out: &mut [u8],
    ) -> Option<usize> {
        let mut w = Writer { buf: out, pos: 0 };
        w.put(&id.to_be_bytes())?;
        w.put(&0x8400u16.to_be_bytes())?; // response, authoritative
        w.put(&question.map_or(0, |q| q.0).to_be_bytes())?;
        w.put(&4u16.to_be_bytes())?;
        w.put(&[0, 0, 0, 0])?;
        if let Some((_, q)) = question {
            w.put(q)?;
        }
        let ttl = if legacy { self.ttl.min(LEGACY_TTL) } else { self.ttl };
        let unique = if legacy { CLASS_IN } else { CLASS_IN | CACHE_FLUSH };
        let ptr = &self.records[self.ptr.2..self.ptr.2 + self.ptr.3];
        let srv = &self.records[self.srv.0..self.srv.0 + self.srv.1];
        let txt = &self.records[self.txt.0..self.txt.0 + self.txt.1];
        // The PTR record is shared, so it never has the cache-flush bit (RFC 6762 10.2).
        w.record(self.service(), 12, CLASS_IN, ttl, ptr)?;
        w.record(self.instance(), 33, unique, ttl, srv)?;
        w.record(self.instance(), 16, unique, ttl, txt)?;
        w.record(self.hostname(), TYPE_A, unique, ttl, &self.ip)?;
        Some(w.pos)
    }
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

struct Writer<'b> {
    buf: &'b mut [u8],
    pos: usize,
}

impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Option<()> {
        let end = self.pos + bytes.len();
        self.buf.get_mut(self.pos..end)?.copy_from_slice(bytes);
        self.pos = end;
        Some(())
    }

    fn record(&mut self, name: &[u8], rtype: u16, class: u16, ttl: u32, rdata: &[u8]) -> Option<()> {
        self.put(name)?;
        self.put(&rtype.to_be_bytes())?;
        self.put(&class.to_be_bytes())?;
        self.put(&ttl.to_be_bytes())?;
        self.put(&u16::try_from(rdata.len()).ok()?.to_be_bytes())?;
        self.put(rdata)
    }
}
