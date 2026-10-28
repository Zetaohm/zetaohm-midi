//! DNS-SD for the desktop example (M2-124-UM v1.0.1 section 4). `zmidi-net` gives the record data
//! only (DEC-012); this module does the network part.
//!
//! - Host: [`register`] gives the records to the platform DNS-SD service (REQ-NET-DISC-002): on
//!   macOS the system API `DNSServiceRegister`, else the text fields of
//!   `HostRecords::text_fields` to `dns-sd -R` (Windows with Bonjour) or `avahi-publish-service`
//!   (Linux). The platform responder owns UDP port 5353, so the example
//!   runs no responder of its own.
//! - Client: [`browse`] sends one mDNS query for "_midi2._udp.local." PTR from a normal UDP
//!   port (a one-shot query, RFC 6762 5.1 and 6.7: the responders reply by unicast) and reads
//!   the PTR, SRV, TXT and A records of the replies with the `zmidi-net` readers.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use zmidi_net::dns::{
    read_name, read_ptr, read_srv, read_txt, HostRecords, Name, SERVICE_NAME_WIRE,
};

/// The mDNS IPv4 group and port (RFC 6762 3).
const MDNS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), 5353);
const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
const TYPE_TXT: u16 = 16;
const TYPE_SRV: u16 = 33;
/// The "unicast response" bit of the question class (RFC 6762 5.4).
const QU: u16 = 0x8000;

/// A registration with the platform DNS-SD service; dropping it ends the registration.
pub enum Registration {
    /// macOS: a `DNSServiceRegister` reference of the system API. The registration belongs to
    /// this process, so the system also removes it when the process stops for another reason.
    #[cfg(target_os = "macos")]
    Native(macos::ServiceRef),
    /// Other systems: the `dns-sd` or `avahi-publish-service` child process. A signal that
    /// stops this process does not stop the child; stop it by hand then.
    Child(Child),
}

impl Drop for Registration {
    // Only `Child` exists on systems other than macOS, so there the pattern always matches.
    #[allow(irrefutable_let_patterns)]
    fn drop(&mut self) {
        if let Registration::Child(c) = self {
            let _ = c.kill();
        }
    }
}

/// Registers the records with the platform DNS-SD service (REQ-NET-DISC-002); `None` when
/// the platform has no service that this example knows.
pub fn register(records: &HostRecords) -> Option<Registration> {
    #[cfg(target_os = "macos")]
    if let Some(r) = macos::register(records) {
        return Some(Registration::Native(r));
    }
    let f = records.text_fields();
    let txt: Vec<String> = f.txt.iter().map(|e| e.to_string()).collect();
    let port = f.port.to_string();
    let dns_sd = Command::new("dns-sd")
        .args(["-R", f.instance_name, f.service_type, f.domain, &port])
        .args(&txt)
        .stdout(Stdio::null())
        .spawn();
    if let Ok(c) = dns_sd {
        return Some(Registration::Child(c));
    }
    Command::new("avahi-publish-service")
        .args([f.instance_name, f.service_type, &port])
        .args(&txt)
        .stdout(Stdio::null())
        .spawn()
        .ok()
        .map(Registration::Child)
}

#[cfg(target_os = "macos")]
mod macos {
    //! `DNSServiceRegister` of the system DNS-SD API (`dns_sd.h`, in libSystem).

    use std::ffi::{c_char, c_void, CString};

    use zmidi_net::dns::{HostRecords, MAX_RECORD_SET};

    #[repr(C)]
    pub struct DnsServiceRefT {
        _private: [u8; 0],
    }

    extern "C" {
        fn DNSServiceRegister(
            sd_ref: *mut *mut DnsServiceRefT,
            flags: u32,
            interface_index: u32,
            name: *const c_char,
            regtype: *const c_char,
            domain: *const c_char,
            host: *const c_char,
            port: u16,
            txt_len: u16,
            txt_record: *const c_void,
            callback: *const c_void,
            context: *mut c_void,
        ) -> i32;
        fn DNSServiceRefDeallocate(sd_ref: *mut DnsServiceRefT);
    }

    /// An owned `DNSServiceRef`.
    pub struct ServiceRef(*mut DnsServiceRefT);

    impl Drop for ServiceRef {
        fn drop(&mut self) {
            // SAFETY: the reference came from a successful DNSServiceRegister and is freed once.
            unsafe { DNSServiceRefDeallocate(self.0) }
        }
    }

    pub fn register(records: &HostRecords) -> Option<ServiceRef> {
        let f = records.text_fields();
        let mut buf = [0u8; MAX_RECORD_SET];
        let set = records.write(&mut buf).ok()?;
        // The TXT RDATA is the TXT record format of the API (length-prefixed strings).
        let txt = set.txt.rdata;
        let name = CString::new(f.instance_name).ok()?;
        let regtype = CString::new(f.service_type).ok()?;
        let mut sd = std::ptr::null_mut();
        // SAFETY: the strings and the TXT bytes live for the call; the API copies them. No
        // callback, so no context is read.
        let err = unsafe {
            DNSServiceRegister(
                &mut sd,
                0,
                0,
                name.as_ptr(),
                regtype.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                f.port.to_be(),
                txt.len() as u16,
                txt.as_ptr().cast(),
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        (err == 0 && !sd.is_null()).then_some(ServiceRef(sd))
    }
}

/// One Host instance found by [`browse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The Service Instance Name (REQ-NET-DISC-006).
    pub instance: String,
    /// The UMP Endpoint Name of the TXT record, if any (REQ-NET-DISC-010).
    pub endpoint_name: Option<String>,
    /// The address of the A record of the SRV target, else the address of the reply, and the
    /// SRV port (REQ-NET-DISC-007).
    pub addr: SocketAddr,
}

/// The mDNS query: one question "_midi2._udp.local." PTR IN, with the unicast response bit.
pub fn query() -> Vec<u8> {
    let mut q = vec![0x4d, 0x32, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    q.extend_from_slice(&SERVICE_NAME_WIRE);
    q.extend_from_slice(&TYPE_PTR.to_be_bytes());
    q.extend_from_slice(&(QU | 1).to_be_bytes());
    q
}

/// Sends the query and collects the replies for `wait`.
pub fn browse(wait: Duration) -> std::io::Result<Vec<Found>> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_multicast_ttl_v4(255)?;
    socket.send_to(&query(), MDNS)?;
    let end = Instant::now() + wait;
    let mut answers = Answers::default();
    let mut buf = [0u8; 9000];
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        socket.set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => answers.read(&buf[..n], from.ip()),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break
            }
            Err(e) => return Err(e),
        }
    }
    Ok(answers.found())
}

/// The records of the replies, by owner name (lower case, dot separated).
#[derive(Debug, Default)]
pub struct Answers {
    instances: Vec<(String, String, IpAddr)>,
    srv: HashMap<String, (u16, String)>,
    txt: HashMap<String, Option<String>>,
    a: HashMap<String, Ipv4Addr>,
}

fn text(name: &Name<'_>) -> String {
    let labels: Vec<String> = name
        .labels()
        .map(|l| String::from_utf8_lossy(l).to_lowercase())
        .collect();
    labels.join(".")
}

fn be16(m: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*m.get(at)?, *m.get(at + 1)?]))
}

impl Answers {
    /// Reads one DNS message; a malformed message is skipped where it breaks.
    pub fn read(&mut self, m: &[u8], from: IpAddr) {
        let _ = self.try_read(m, from);
    }

    fn try_read(&mut self, m: &[u8], from: IpAddr) -> Option<()> {
        let qd = be16(m, 4)?;
        let rr = be16(m, 6)? as usize + be16(m, 8)? as usize + be16(m, 10)? as usize;
        let mut at = 12;
        for _ in 0..qd {
            at = read_name(m, at).ok()?.1 + 4;
        }
        for _ in 0..rr {
            let (owner, end) = read_name(m, at).ok()?;
            let rtype = be16(m, end)?;
            let len = be16(m, end + 8)? as usize;
            let rdata = end + 10;
            let data = m.get(rdata..rdata + len)?;
            let owner_text = text(&owner);
            match rtype {
                TYPE_PTR if owner.is_service_type() => {
                    if let Ok(p) = read_ptr(m, rdata) {
                        let instance = String::from_utf8_lossy(p.instance).into_owned();
                        self.instances.push((text(&p.name), instance, from));
                    }
                }
                TYPE_SRV => {
                    if let Ok(s) = read_srv(m, rdata) {
                        self.srv.insert(owner_text, (s.port, text(&s.target)));
                    }
                }
                TYPE_TXT => {
                    let t = read_txt(data);
                    let name = t
                        .endpoint_name
                        .map(|n| String::from_utf8_lossy(n).into_owned());
                    self.txt.insert(owner_text, name);
                }
                TYPE_A if len == 4 => {
                    self.a.insert(
                        owner_text,
                        Ipv4Addr::new(data[0], data[1], data[2], data[3]),
                    );
                }
                _ => {}
            }
            at = rdata + len;
        }
        Some(())
    }

    /// The instances with a PTR and an SRV record, each one once.
    pub fn found(&self) -> Vec<Found> {
        let mut out: Vec<Found> = Vec::new();
        for (full, instance, from) in &self.instances {
            let Some((port, target)) = self.srv.get(full) else {
                continue;
            };
            let ip = self.a.get(target).map_or(*from, |a| IpAddr::V4(*a));
            let f = Found {
                instance: instance.clone(),
                endpoint_name: self.txt.get(full).cloned().flatten(),
                addr: SocketAddr::new(ip, *port),
            };
            if !out.contains(&f) {
                out.push(f);
            }
        }
        out
    }
}
