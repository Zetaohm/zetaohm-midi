//! The C ABI of the zmidi-net Host and Client (Network MIDI 2.0, M2-124) with the default sizes
//! (DEC-019). It is sans-IO: the program owns the UDP socket, gives each received payload, and
//! sends each payload that the endpoint writes. There is no callback.
//!
//! With the feature `net-auth` (zmidi-net `auth`, SHA-256, DEC-013), a Host can ask for
//! Invitation with Authentication or with User Authentication (M2-124 6.7 to 6.10), and a Client
//! can answer. The program keeps the shared secret, the user table and the random bytes in a
//! `ZmidiNetCredentials` in its own storage and gives it to each receive call; the C ABI copies
//! no secret and no password (N-52).

use core::mem::{align_of, size_of};
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use zmidi_net::command::ByeReason;
use zmidi_net::handshake::CredentialsError;
use zmidi_net::packet::MAX_PAYLOAD;
use zmidi_net::session::CloseCause;
#[cfg(not(feature = "net-auth"))]
use zmidi_net::NoCredentials;
#[cfg(feature = "net-auth")]
use zmidi_net::{Capabilities, PolicyKind};
use zmidi_net::{
    Client, ClientConfig, Credentials, Diagnostic, EndpointError, Event, EventKind, Host,
    HostConfig, HostPolicy, Identity, Method, SessionEvent,
};

use crate::status::ZmidiStatus;

/// Storage words of a [`ZmidiNetHost`] on a 64-bit target.
#[cfg(all(target_pointer_width = "64", not(feature = "net-auth")))]
pub const ZMIDI_NET_HOST_WORDS: usize = 1216;
/// Storage words of a [`ZmidiNetHost`] on a 32-bit or 16-bit target.
#[cfg(all(not(target_pointer_width = "64"), not(feature = "net-auth")))]
pub const ZMIDI_NET_HOST_WORDS: usize = 1160;
/// Storage words of a [`ZmidiNetClient`] on a 64-bit target.
#[cfg(all(target_pointer_width = "64", not(feature = "net-auth")))]
pub const ZMIDI_NET_CLIENT_WORDS: usize = 520;
/// Storage words of a [`ZmidiNetClient`] on a 32-bit or 16-bit target.
#[cfg(all(not(target_pointer_width = "64"), not(feature = "net-auth")))]
pub const ZMIDI_NET_CLIENT_WORDS: usize = 496;
/// Storage words of a [`ZmidiNetHost`] with the feature `net-auth` on a 64-bit target: each
/// session keeps the CryptoNonce, the digest and the Username (zmidi-net `auth`).
#[cfg(all(target_pointer_width = "64", feature = "net-auth"))]
pub const ZMIDI_NET_HOST_WORDS: usize = 1336;
/// Storage words of a [`ZmidiNetHost`] with the feature `net-auth` on a 32-bit or 16-bit target.
#[cfg(all(not(target_pointer_width = "64"), feature = "net-auth"))]
pub const ZMIDI_NET_HOST_WORDS: usize = 1280;
/// Storage words of a [`ZmidiNetClient`] with the feature `net-auth` on a 64-bit target.
#[cfg(all(target_pointer_width = "64", feature = "net-auth"))]
pub const ZMIDI_NET_CLIENT_WORDS: usize = 552;
/// Storage words of a [`ZmidiNetClient`] with the feature `net-auth` on a 32-bit or 16-bit
/// target.
#[cfg(all(not(target_pointer_width = "64"), feature = "net-auth"))]
pub const ZMIDI_NET_CLIENT_WORDS: usize = 528;
/// Bytes before the payload of each reply record: the peer address (19 bytes: family, 16 address
/// bytes, port high, port low) and the payload length (2 bytes, high first).
pub const ZMIDI_NET_RECORD_HEADER: usize = 21;
/// The largest UDP payload that an endpoint writes (zmidi-net `MAX_PAYLOAD`).
pub const ZMIDI_NET_MAX_PAYLOAD: usize = 1400;

/// The shared secret method (M2-124 6.9): Capabilities bit D0, Invitation Reply: Authentication
/// Required (0x12), Invitation with Authentication (0x02). A bit of the `methods` of the
/// `*_init_auth` functions and the method code of the authentication events.
pub const ZMIDI_NET_AUTH_SHARED_SECRET: u8 = 0x01;
/// The user method (M2-124 6.10): Capabilities bit D1, Invitation Reply: User Authentication
/// Required (0x13), Invitation with User Authentication (0x03). A bit as
/// [`ZMIDI_NET_AUTH_SHARED_SECRET`].
pub const ZMIDI_NET_AUTH_USER: u8 = 0x02;
/// The flag in the `code` of an `AuthFailed` event when the Host did not find the Username
/// (M2-124 6.10, Authentication State 0x02).
pub const ZMIDI_NET_AUTH_USERNAME_NOT_FOUND: u32 = 0x100;

const _: () = assert!(ZMIDI_NET_MAX_PAYLOAD == MAX_PAYLOAD);

/// Storage of one Host. The program gives it; [`zmidi_net_host_init`] sets it up. 16-byte
/// alignment.
#[repr(C, align(16))]
pub struct ZmidiNetHost {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_NET_HOST_WORDS],
}

/// Storage of one Client. The program gives it; [`zmidi_net_client_init`] sets it up. 16-byte
/// alignment.
#[repr(C, align(16))]
pub struct ZmidiNetClient {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_NET_CLIENT_WORDS],
}

/// The state in a [`ZmidiNetHost`]: `Host::init` builds the Host in place, so the Host (about
/// 9 KB) is never on the stack.
type HostSlot = Option<Host>;
/// The state in a [`ZmidiNetClient`], built in place as [`HostSlot`].
type ClientSlot = Option<Client>;

const _: () = assert!(size_of::<HostSlot>() <= size_of::<ZmidiNetHost>());
const _: () = assert!(align_of::<HostSlot>() <= align_of::<ZmidiNetHost>());
const _: () = assert!(size_of::<ClientSlot>() <= size_of::<ZmidiNetClient>());
const _: () = assert!(align_of::<ClientSlot>() <= align_of::<ZmidiNetClient>());

/// An IP address and a UDP port.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiNetAddr {
    /// 4: IPv4 (the first 4 bytes of `ip`); 6: IPv6.
    pub family: u8,
    /// The address bytes, network order.
    pub ip: [u8; 16],
    /// The UDP port.
    pub port: u16,
}

impl ZmidiNetAddr {
    fn to_rust(self) -> Option<SocketAddr> {
        let ip = match self.family {
            4 => IpAddr::V4(Ipv4Addr::new(
                self.ip[0], self.ip[1], self.ip[2], self.ip[3],
            )),
            6 => IpAddr::V6(Ipv6Addr::from(self.ip)),
            _ => return None,
        };
        Some(SocketAddr::new(ip, self.port))
    }

    fn from_rust(a: SocketAddr) -> Self {
        let mut ip = [0u8; 16];
        let family = match a.ip() {
            IpAddr::V4(v) => {
                ip[..4].copy_from_slice(&v.octets());
                4
            }
            IpAddr::V6(v) => {
                ip = v.octets();
                6
            }
        };
        ZmidiNetAddr {
            family,
            ip,
            port: a.port(),
        }
    }
}

/// The kind of an event.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiNetEventKind {
    /// No event.
    None = 0,
    /// Host: a new Client asks for a session.
    InvitationReceived = 1,
    /// Client: the Host answered Pending.
    Pending = 2,
    /// Client: the Host asks for authentication; `code` is the method
    /// ([`ZMIDI_NET_AUTH_SHARED_SECRET`] or [`ZMIDI_NET_AUTH_USER`]), `value` the Authentication
    /// State (0x00 first request, 0x01 digest not correct, 0x02 Username not found).
    AuthRequested = 3,
    /// Host: a failed authentication; `code` is the method, with
    /// [`ZMIDI_NET_AUTH_USERNAME_NOT_FOUND`] when the Username was not found; `value` is the
    /// count of failures of the session.
    AuthFailed = 4,
    /// A problem with the credentials of the program; `code`: 0 no random bytes (Host), 1 no
    /// shared secret (Host), 2 empty shared secret, 3 empty password, 4 Username not valid
    /// (Client), 5 Username too long (Client).
    CredentialsError = 5,
    /// The session is established; `code` is the authentication method, 0 for none.
    Established = 6,
    /// This side ends the session with a Bye; `code` is the Bye Reason.
    Ending = 7,
    /// The session went to Idle; `code`: 0 Bye received (`value`: its Bye Reason), 1 Bye Reply,
    /// 2 Bye timeout, 3 expired, 4 ended by this side (`value`: the Bye Reason).
    Closed = 8,
    /// The session was reset.
    Reset = 9,
    /// This side started a Session Reset.
    ResetStarted = 10,
    /// A NAK was received; `code` is the NAK Reason.
    Nak = 11,
    /// A Ping Reply; `code` is the Ping Id, `value` the round-trip time in ms.
    PingReply = 12,
    /// A Ping Reply with no open Ping; `code` is the Ping Id.
    UnexpectedPingReply = 13,
    /// The session timed out.
    Timeout = 14,
    /// An event of the UMP Data stream (data loss, Retransmit Error).
    Stream = 15,
    /// UMP Data: `word_offset` and `word_count` in the word array.
    Data = 16,
    /// A diagnostic; `code`: 0 bad signature, 1 invalid peer address, 2 table full, 3 NAK sent
    /// (`value`: its NAK Reason).
    Diagnostic = 17,
}

/// One event of a peer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiNetEvent {
    /// The kind of the event.
    pub kind: ZmidiNetEventKind,
    /// The address of the peer.
    pub peer: ZmidiNetAddr,
    /// A code of the kind (see [`ZmidiNetEventKind`]).
    pub code: u32,
    /// A value of the kind (see [`ZmidiNetEventKind`]).
    pub value: u64,
    /// Data: the index of the first word in the word array.
    pub word_offset: usize,
    /// Data: the count of words.
    pub word_count: usize,
}

/// The arrays of the program for one receive or poll call. Each `*_len` is set by the call.
#[repr(C)]
#[derive(Debug)]
pub struct ZmidiNetBuffers {
    /// Event array.
    pub events: *mut ZmidiNetEvent,
    /// Capacity of `events`.
    pub events_capacity: usize,
    /// Events written.
    pub events_len: usize,
    /// Word array for the UMP Data.
    pub words: *mut u32,
    /// Capacity of `words`.
    pub words_capacity: usize,
    /// Words written.
    pub words_len: usize,
    /// Byte buffer for the reply records.
    pub out: *mut u8,
    /// Capacity of `out`.
    pub out_capacity: usize,
    /// Bytes written.
    pub out_len: usize,
}

/// The slices of a [`ZmidiNetBuffers`] and the write positions.
struct Sink<'a> {
    events: &'a mut [ZmidiNetEvent],
    words: &'a mut [u32],
    out: &'a mut [u8],
    ne: usize,
    nw: usize,
    no: usize,
    full: bool,
}

impl Sink<'_> {
    /// Takes one event; false when an array is full.
    fn event(&mut self, e: &Event<'_>) {
        let mut out = ZmidiNetEvent {
            kind: ZmidiNetEventKind::None,
            peer: ZmidiNetAddr::from_rust(e.peer),
            code: 0,
            value: 0,
            word_offset: 0,
            word_count: 0,
        };
        use ZmidiNetEventKind as K;
        match e.kind {
            EventKind::Session(s) => {
                let (kind, code, value) = match s {
                    SessionEvent::InvitationReceived { .. } => (K::InvitationReceived, 0, 0),
                    SessionEvent::Pending => (K::Pending, 0, 0),
                    SessionEvent::AuthRequested(c) => {
                        (K::AuthRequested, method_code(c.method), u64::from(c.state))
                    }
                    SessionEvent::AuthFailed {
                        method,
                        failures,
                        username_not_found,
                    } => {
                        let flag = if username_not_found {
                            ZMIDI_NET_AUTH_USERNAME_NOT_FOUND
                        } else {
                            0
                        };
                        (
                            K::AuthFailed,
                            method_code(method) | flag,
                            u64::from(failures),
                        )
                    }
                    SessionEvent::CredentialsError(c) => {
                        (K::CredentialsError, credentials_code(c), 0)
                    }
                    SessionEvent::Established { method } => {
                        (K::Established, method.map_or(0, method_code), 0)
                    }
                    SessionEvent::Ending(r) => (K::Ending, u32::from(r.0), 0),
                    SessionEvent::Closed(c) => match c {
                        CloseCause::ByeReceived(b) => (K::Closed, 0, u64::from(b.reason.0)),
                        CloseCause::ByeReply => (K::Closed, 1, 0),
                        CloseCause::ByeTimeout => (K::Closed, 2, 0),
                        CloseCause::Expired => (K::Closed, 3, 0),
                        CloseCause::Ended(r) => (K::Closed, 4, u64::from(r.0)),
                    },
                    SessionEvent::Reset => (K::Reset, 0, 0),
                    SessionEvent::ResetStarted => (K::ResetStarted, 0, 0),
                    SessionEvent::Nak(n) => (K::Nak, u32::from(n.reason.0), 0),
                    SessionEvent::PingReply { id, rtt_ms } => (K::PingReply, id, rtt_ms),
                    SessionEvent::UnexpectedPingReply(id) => (K::UnexpectedPingReply, id, 0),
                    SessionEvent::Timeout => (K::Timeout, 0, 0),
                    SessionEvent::Stream(_) => (K::Stream, 0, 0),
                };
                out.kind = kind;
                out.code = code;
                out.value = value;
            }
            EventKind::Data(d) => {
                out.kind = K::Data;
                out.word_offset = self.nw;
                for m in d.messages() {
                    let w = m.words();
                    match self.words.get_mut(self.nw..self.nw + w.len()) {
                        Some(dst) => {
                            dst.copy_from_slice(w);
                            self.nw += w.len();
                        }
                        None => self.full = true,
                    }
                }
                out.word_count = self.nw - out.word_offset;
            }
            EventKind::Diagnostic(d) => {
                out.kind = K::Diagnostic;
                (out.code, out.value) = match d {
                    Diagnostic::BadSignature => (0, 0),
                    Diagnostic::InvalidPeerAddress => (1, 0),
                    Diagnostic::TableFull => (2, 0),
                    Diagnostic::NakSent { reason, .. } => (3, u64::from(reason.0)),
                };
            }
        }
        match self.events.get_mut(self.ne) {
            Some(slot) => {
                *slot = out;
                self.ne += 1;
            }
            None => self.full = true,
        }
    }

    /// The tail of `out` after room for a record header, when it holds a whole payload.
    fn reply_room(&mut self) -> Option<&mut [u8]> {
        let start = self.no + ZMIDI_NET_RECORD_HEADER;
        let room = self.out.get_mut(start..)?;
        (room.len() >= MAX_PAYLOAD).then_some(room)
    }

    /// Writes the header of a record of `len` bytes to `to` at the current position.
    fn reply_done(&mut self, to: SocketAddr, len: usize) {
        let a = ZmidiNetAddr::from_rust(to);
        let h = &mut self.out[self.no..self.no + ZMIDI_NET_RECORD_HEADER];
        h[0] = a.family;
        h[1..17].copy_from_slice(&a.ip);
        h[17..19].copy_from_slice(&a.port.to_be_bytes());
        h[19..21].copy_from_slice(&(len as u16).to_be_bytes());
        self.no += ZMIDI_NET_RECORD_HEADER + len;
    }
}

/// Gives the slices of `b`, or `None` for null arrays.
///
/// # Safety
/// `b` is null or valid, with each pointer valid for its capacity.
unsafe fn sink<'a>(b: *mut ZmidiNetBuffers) -> Option<Sink<'a>> {
    // SAFETY: caller contract above.
    let b = unsafe { b.as_mut()? };
    if b.events.is_null() || b.words.is_null() || b.out.is_null() {
        return None;
    }
    // SAFETY: caller contract above.
    unsafe {
        Some(Sink {
            events: core::slice::from_raw_parts_mut(b.events, b.events_capacity),
            words: core::slice::from_raw_parts_mut(b.words, b.words_capacity),
            out: core::slice::from_raw_parts_mut(b.out, b.out_capacity),
            ne: 0,
            nw: 0,
            no: 0,
            full: false,
        })
    }
}

/// The credentials of the receive functions without credentials: none.
#[cfg(not(feature = "net-auth"))]
fn plain_credentials() -> NoCredentials {
    NoCredentials
}

/// The credentials of the receive functions without credentials: none. With `net-auth` they are
/// an empty `CCredentials` (the same answers as `NoCredentials`), so that the receive path of
/// zmidi-net is built once, not once for each credentials type (code size).
#[cfg(feature = "net-auth")]
fn plain_credentials() -> CCredentials<'static> {
    CCredentials::EMPTY
}

/// The method code of the events.
fn method_code(m: Method) -> u32 {
    u32::from(match m {
        Method::SharedSecret => ZMIDI_NET_AUTH_SHARED_SECRET,
        Method::User => ZMIDI_NET_AUTH_USER,
    })
}

/// The `code` of a `CredentialsError` event.
fn credentials_code(c: CredentialsError) -> u32 {
    match c {
        CredentialsError::NoRandom => 0,
        CredentialsError::NoSecret => 1,
        CredentialsError::EmptySecret => 2,
        CredentialsError::EmptyPassword => 3,
        CredentialsError::InvalidUsername => 4,
        CredentialsError::UsernameTooLong => 5,
    }
}

/// Writes the lengths of `s` to `b` and gives the status.
///
/// # Safety
/// `b` is valid.
unsafe fn finish(b: *mut ZmidiNetBuffers, s: &Sink<'_>) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let b = unsafe { &mut *b };
    b.events_len = s.ne;
    b.words_len = s.nw;
    b.out_len = s.no;
    if s.full {
        ZmidiStatus::BufferTooSmall
    } else {
        ZmidiStatus::Ok
    }
}

/// Gives the UTF-8 text of a NUL-terminated C string of at most `max` bytes.
///
/// # Safety
/// `p` is null or points to a NUL-terminated string.
unsafe fn c_text<'a>(p: *const u8, max: usize) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: caller contract above.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
        if n > max {
            return None;
        }
    }
    // SAFETY: caller contract above.
    core::str::from_utf8(unsafe { core::slice::from_raw_parts(p, n) }).ok()
}

/// Gives the Identity of two C texts.
///
/// # Safety
/// As [`c_text`] for both.
unsafe fn identity(name: *const u8, product_instance_id: *const u8) -> Option<Identity> {
    // SAFETY: caller contract above.
    let (n, p) = unsafe { (c_text(name, 512)?, c_text(product_instance_id, 512)?) };
    Identity::new(n, p).ok()
}

fn endpoint_error(e: EndpointError) -> ZmidiStatus {
    match e {
        EndpointError::InvalidAddress => ZmidiStatus::InvalidArgument,
        _ => ZmidiStatus::Rejected,
    }
}

/// Sets up `host` with the UMP Endpoint Name `name` and the Product Instance Id
/// `product_instance_id` (NUL-terminated UTF-8) and the default configuration.
///
/// # Safety
/// `host` points to writable storage of a [`ZmidiNetHost`]; the texts are null or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_init(
    host: *mut ZmidiNetHost,
    name: *const u8,
    product_instance_id: *const u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { host_init(host, name, product_instance_id, HostPolicy::DEFAULT) }
}

/// The Host init with a policy.
///
/// # Safety
/// As [`zmidi_net_host_init`].
unsafe fn host_init(
    host: *mut ZmidiNetHost,
    name: *const u8,
    product_instance_id: *const u8,
    policy: HostPolicy,
) -> ZmidiStatus {
    if host.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(id) = (unsafe { identity(name, product_instance_id) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    let slot = unsafe {
        host.cast::<HostSlot>().write(None);
        &mut *host.cast::<HostSlot>()
    };
    let config = HostConfig {
        policy,
        ..HostConfig::new(id)
    };
    match Host::init(slot, config) {
        Ok(_) => ZmidiStatus::Ok,
        Err(e) => endpoint_error(e),
    }
}

/// Sets up `client` with the UMP Endpoint Name `name` and the Product Instance Id
/// `product_instance_id` (NUL-terminated UTF-8) and the default configuration.
///
/// # Safety
/// `client` points to writable storage of a [`ZmidiNetClient`]; the texts are null or
/// NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_init(
    client: *mut ZmidiNetClient,
    name: *const u8,
    product_instance_id: *const u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { client_init(client, name, product_instance_id, |c| c) }
}

/// The Client init with a change of the default configuration.
///
/// # Safety
/// As [`zmidi_net_client_init`].
unsafe fn client_init(
    client: *mut ZmidiNetClient,
    name: *const u8,
    product_instance_id: *const u8,
    config: impl FnOnce(ClientConfig) -> ClientConfig,
) -> ZmidiStatus {
    if client.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(id) = (unsafe { identity(name, product_instance_id) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    let slot = unsafe {
        client.cast::<ClientSlot>().write(None);
        &mut *client.cast::<ClientSlot>()
    };
    match Client::init(slot, config(ClientConfig::new(id))) {
        Ok(_) => ZmidiStatus::Ok,
        Err(e) => endpoint_error(e),
    }
}

/// Starts an Invitation to the Host at `addr` at `now_ms`.
///
/// # Safety
/// `client` was set up by [`zmidi_net_client_init`]; `addr` is valid.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_connect(
    client: *mut ZmidiNetClient,
    now_ms: u64,
    addr: *const ZmidiNetAddr,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(c), Some(a)) = (unsafe { ep(client.cast::<ClientSlot>()) }, unsafe {
        addr.as_ref()
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Some(a) = a.to_rust() else {
        return ZmidiStatus::InvalidArgument;
    };
    match c.connect(now_ms, a) {
        Ok(()) => ZmidiStatus::Ok,
        Err(e) => endpoint_error(e),
    }
}

/// The requests that the Host and the Client share (the inherent methods of zmidi-net).
trait Endpoint {
    fn rx<Cr: Credentials + ?Sized>(
        &mut self,
        now: u64,
        from: SocketAddr,
        payload: &[u8],
        creds: &mut Cr,
        s: &mut Sink<'_>,
    );
    fn poll_into(&mut self, now: u64, s: &mut Sink<'_>);
    fn pending(&mut self, now: u64, buf: &mut [u8]) -> Option<(SocketAddr, usize)>;
    fn deadline(&self) -> Option<u64>;
    fn send(&mut self, addr: SocketAddr, words: &[u32]) -> Result<(), EndpointError>;
    fn ping_peer(&mut self, now: u64, addr: SocketAddr) -> Result<u32, EndpointError>;
    fn end(
        &mut self,
        now: u64,
        addr: SocketAddr,
        reason: u8,
        s: &mut Sink<'_>,
    ) -> Result<(), EndpointError>;
    fn dropped(&self) -> u32;
}

macro_rules! endpoint_impl {
    ($t:ty) => {
        impl Endpoint for $t {
            fn rx<Cr: Credentials + ?Sized>(
                &mut self,
                now: u64,
                from: SocketAddr,
                payload: &[u8],
                creds: &mut Cr,
                s: &mut Sink<'_>,
            ) {
                let mut r = self.receive(now, from, payload, creds);
                for e in r.by_ref() {
                    s.event(&e);
                }
                loop {
                    let Some(room) = s.reply_room() else {
                        s.full = true;
                        break;
                    };
                    let Some((to, n)) = r.write_next(room) else {
                        break;
                    };
                    s.reply_done(to, n);
                }
            }

            fn poll_into(&mut self, now: u64, s: &mut Sink<'_>) {
                let mut p = self.poll(now);
                while s.ne < s.events.len() {
                    let Some(e) = p.next() else { break };
                    s.event(&e);
                }
            }

            fn pending(&mut self, now: u64, buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
                self.write_pending(now, buf)
            }

            fn deadline(&self) -> Option<u64> {
                self.next_deadline()
            }

            fn send(&mut self, addr: SocketAddr, words: &[u32]) -> Result<(), EndpointError> {
                self.send_ump(addr, words)
            }

            fn ping_peer(&mut self, now: u64, addr: SocketAddr) -> Result<u32, EndpointError> {
                self.ping(now, addr, None)
            }

            fn end(
                &mut self,
                now: u64,
                addr: SocketAddr,
                reason: u8,
                s: &mut Sink<'_>,
            ) -> Result<(), EndpointError> {
                let events = self.bye(now, addr, ByeReason(reason), "")?;
                for e in events.iter() {
                    s.event(&Event {
                        peer: addr,
                        kind: EventKind::Session(*e),
                    });
                }
                Ok(())
            }

            fn dropped(&self) -> u32 {
                self.dropped_events()
            }
        }
    };
}

endpoint_impl!(Host);
endpoint_impl!(Client);

/// Gives the endpoint in the storage `p`, or `None` for a null pointer or storage with no endpoint.
///
/// # Safety
/// `p` is null or was set up by the init function of `E`.
unsafe fn ep<'a, E>(p: *mut Option<E>) -> Option<&'a mut E> {
    // SAFETY: caller contract above.
    unsafe { p.as_mut() }.and_then(Option::as_mut)
}

/// # Safety
/// As the receive functions.
#[allow(clippy::too_many_arguments)]
unsafe fn receive<E: Endpoint, Cr: Credentials + ?Sized>(
    e: *mut Option<E>,
    now: u64,
    from: *const ZmidiNetAddr,
    payload: *const u8,
    len: usize,
    creds: &mut Cr,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(ep), Some(from), Some(mut s)) =
        (unsafe { ep(e) }, unsafe { from.as_ref() }, unsafe {
            sink(bufs)
        })
    else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Some(from), false) = (from.to_rust(), payload.is_null()) else {
        return ZmidiStatus::InvalidArgument;
    };
    if s.words.len() < len / 4 {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    let payload = unsafe { core::slice::from_raw_parts(payload, len) };
    ep.rx(now, from, payload, creds, &mut s);
    // SAFETY: caller contract above.
    unsafe { finish(bufs, &s) }
}

/// # Safety
/// As the poll functions.
unsafe fn poll<E: Endpoint>(
    e: *mut Option<E>,
    now: u64,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(ep), Some(mut s)) = (unsafe { ep(e) }, unsafe { sink(bufs) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    ep.poll_into(now, &mut s);
    // SAFETY: caller contract above.
    unsafe { finish(bufs, &s) }
}

/// # Safety
/// As the write_pending functions.
unsafe fn write_pending<E: Endpoint>(
    e: *mut Option<E>,
    now: u64,
    buf: *mut u8,
    capacity: usize,
    to: *mut ZmidiNetAddr,
    len: *mut usize,
) -> ZmidiStatus {
    if buf.is_null() || to.is_null() || len.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(ep) = (unsafe { ep(e) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    if capacity < MAX_PAYLOAD {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    let buf = unsafe { core::slice::from_raw_parts_mut(buf, capacity) };
    let (addr, n) = match ep.pending(now, buf) {
        Some((a, n)) => (ZmidiNetAddr::from_rust(a), n),
        None => (
            ZmidiNetAddr {
                family: 0,
                ip: [0; 16],
                port: 0,
            },
            0,
        ),
    };
    // SAFETY: caller contract above.
    unsafe {
        to.write(addr);
        len.write(n);
    }
    ZmidiStatus::Ok
}

/// # Safety
/// As the next_deadline functions.
unsafe fn next_deadline<E: Endpoint>(e: *mut Option<E>, deadline_ms: *mut u64) -> bool {
    // SAFETY: caller contract above.
    let Some(ep) = (unsafe { ep(e) }) else {
        return false;
    };
    match (ep.deadline(), deadline_ms.is_null()) {
        (Some(d), false) => {
            // SAFETY: caller contract above.
            unsafe { deadline_ms.write(d) };
            true
        }
        _ => false,
    }
}

/// # Safety
/// As the send_ump functions.
unsafe fn send_ump<E: Endpoint>(
    e: *mut Option<E>,
    addr: *const ZmidiNetAddr,
    words: *const u32,
    word_count: usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(ep), Some(a)) = (unsafe { ep(e) }, unsafe { addr.as_ref() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Some(a), false) = (a.to_rust(), words.is_null()) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let words = unsafe { core::slice::from_raw_parts(words, word_count) };
    match ep.send(a, words) {
        Ok(()) => ZmidiStatus::Ok,
        Err(e) => endpoint_error(e),
    }
}

/// # Safety
/// As the ping functions.
unsafe fn ping<E: Endpoint>(
    e: *mut Option<E>,
    now: u64,
    addr: *const ZmidiNetAddr,
    id: *mut u32,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(ep), Some(a)) = (unsafe { ep(e) }, unsafe { addr.as_ref() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Some(a), false) = (a.to_rust(), id.is_null()) else {
        return ZmidiStatus::InvalidArgument;
    };
    match ep.ping_peer(now, a) {
        Ok(v) => {
            // SAFETY: caller contract above.
            unsafe { id.write(v) };
            ZmidiStatus::Ok
        }
        Err(e) => endpoint_error(e),
    }
}

/// # Safety
/// As the bye functions.
unsafe fn bye<E: Endpoint>(
    e: *mut Option<E>,
    now: u64,
    addr: *const ZmidiNetAddr,
    reason: u8,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(ep), Some(a), Some(mut s)) = (unsafe { ep(e) }, unsafe { addr.as_ref() }, unsafe {
        sink(bufs)
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Some(peer) = a.to_rust() else {
        return ZmidiStatus::InvalidArgument;
    };
    match ep.end(now, peer, reason, &mut s) {
        // SAFETY: caller contract above.
        Ok(()) => unsafe { finish(bufs, &s) },
        Err(e) => endpoint_error(e),
    }
}

/// # Safety
/// As the dropped_events functions.
unsafe fn dropped<E: Endpoint>(e: *mut Option<E>) -> u32 {
    // SAFETY: caller contract above.
    unsafe { ep(e) }.map_or(0, |ep| ep.dropped())
}

/// Reads one UDP `payload` (`len` bytes) from `from` at `now_ms` into the arrays of `bufs`: events,
/// UMP Data words, reply records. A word array of fewer than `len` / 4 words gives
/// [`ZmidiStatus::BufferTooSmall`] before the payload is read; a full array gives it after.
///
/// # Safety
/// `host` was set up by [`zmidi_net_host_init`]; `from` and `bufs` are valid; `payload` points to
/// `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_receive(
    host: *mut ZmidiNetHost,
    now_ms: u64,
    from: *const ZmidiNetAddr,
    payload: *const u8,
    len: usize,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        receive(
            host.cast::<HostSlot>(),
            now_ms,
            from,
            payload,
            len,
            &mut plain_credentials(),
            bufs,
        )
    }
}

/// The Client form of [`zmidi_net_host_receive`].
///
/// # Safety
/// `client` was set up by [`zmidi_net_client_init`]; as [`zmidi_net_host_receive`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_receive(
    client: *mut ZmidiNetClient,
    now_ms: u64,
    from: *const ZmidiNetAddr,
    payload: *const u8,
    len: usize,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        receive(
            client.cast::<ClientSlot>(),
            now_ms,
            from,
            payload,
            len,
            &mut plain_credentials(),
            bufs,
        )
    }
}

/// Runs the timers at `now_ms` and writes their events into `bufs` (events and words; no records).
/// Events that do not fit wait for the next poll.
///
/// # Safety
/// `host` was set up by [`zmidi_net_host_init`]; `bufs` is valid.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_poll(
    host: *mut ZmidiNetHost,
    now_ms: u64,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { poll(host.cast::<HostSlot>(), now_ms, bufs) }
}

/// The Client form of [`zmidi_net_host_poll`].
///
/// # Safety
/// `client` was set up by [`zmidi_net_client_init`]; `bufs` is valid.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_poll(
    client: *mut ZmidiNetClient,
    now_ms: u64,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { poll(client.cast::<ClientSlot>(), now_ms, bufs) }
}

/// Writes the next pending UDP payload into `buf` (`capacity` bytes, at least
/// [`ZMIDI_NET_MAX_PAYLOAD`]) with its destination in `to` and its length in `len`; `len` is 0 when
/// nothing is pending.
///
/// # Safety
/// `host` was set up by [`zmidi_net_host_init`]; `buf` points to `capacity` writable bytes; `to`
/// and `len` are writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_write_pending(
    host: *mut ZmidiNetHost,
    now_ms: u64,
    buf: *mut u8,
    capacity: usize,
    to: *mut ZmidiNetAddr,
    len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { write_pending(host.cast::<HostSlot>(), now_ms, buf, capacity, to, len) }
}

/// The Client form of [`zmidi_net_host_write_pending`].
///
/// # Safety
/// `client` was set up by [`zmidi_net_client_init`]; as [`zmidi_net_host_write_pending`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_write_pending(
    client: *mut ZmidiNetClient,
    now_ms: u64,
    buf: *mut u8,
    capacity: usize,
    to: *mut ZmidiNetAddr,
    len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { write_pending(client.cast::<ClientSlot>(), now_ms, buf, capacity, to, len) }
}

/// Writes the next time (ms) at which the Host needs a poll to `deadline_ms` and gives true; false
/// when there is none.
///
/// # Safety
/// `host` is null or was set up by [`zmidi_net_host_init`]; `deadline_ms` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_next_deadline(
    host: *mut ZmidiNetHost,
    deadline_ms: *mut u64,
) -> bool {
    // SAFETY: caller contract above.
    unsafe { next_deadline(host.cast::<HostSlot>(), deadline_ms) }
}

/// The Client form of [`zmidi_net_host_next_deadline`].
///
/// # Safety
/// `client` is null or was set up by [`zmidi_net_client_init`]; `deadline_ms` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_next_deadline(
    client: *mut ZmidiNetClient,
    deadline_ms: *mut u64,
) -> bool {
    // SAFETY: caller contract above.
    unsafe { next_deadline(client.cast::<ClientSlot>(), deadline_ms) }
}

/// Queues whole UMP messages (`words`, `word_count` words) for the peer `addr`.
///
/// # Safety
/// `host` was set up by [`zmidi_net_host_init`]; `addr` is valid; `words` points to `word_count`
/// words.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_send_ump(
    host: *mut ZmidiNetHost,
    addr: *const ZmidiNetAddr,
    words: *const u32,
    word_count: usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { send_ump(host.cast::<HostSlot>(), addr, words, word_count) }
}

/// The Client form of [`zmidi_net_host_send_ump`].
///
/// # Safety
/// `client` was set up by [`zmidi_net_client_init`]; as [`zmidi_net_host_send_ump`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_send_ump(
    client: *mut ZmidiNetClient,
    addr: *const ZmidiNetAddr,
    words: *const u32,
    word_count: usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { send_ump(client.cast::<ClientSlot>(), addr, words, word_count) }
}

/// Sends a Ping to `addr` at `now_ms` and writes its Ping Id to `id`.
///
/// # Safety
/// `host` was set up by [`zmidi_net_host_init`]; `addr` is valid; `id` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_ping(
    host: *mut ZmidiNetHost,
    now_ms: u64,
    addr: *const ZmidiNetAddr,
    id: *mut u32,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { ping(host.cast::<HostSlot>(), now_ms, addr, id) }
}

/// The Client form of [`zmidi_net_host_ping`].
///
/// # Safety
/// `client` was set up by [`zmidi_net_client_init`]; as [`zmidi_net_host_ping`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_ping(
    client: *mut ZmidiNetClient,
    now_ms: u64,
    addr: *const ZmidiNetAddr,
    id: *mut u32,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { ping(client.cast::<ClientSlot>(), now_ms, addr, id) }
}

/// Ends the session with `addr` with a Bye of `reason` (Bye Reason value) at `now_ms`, and writes
/// its events into `bufs`.
///
/// # Safety
/// `host` was set up by [`zmidi_net_host_init`]; `addr` and `bufs` are valid.
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_bye(
    host: *mut ZmidiNetHost,
    now_ms: u64,
    addr: *const ZmidiNetAddr,
    reason: u8,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { bye(host.cast::<HostSlot>(), now_ms, addr, reason, bufs) }
}

/// The Client form of [`zmidi_net_host_bye`].
///
/// # Safety
/// `client` was set up by [`zmidi_net_client_init`]; as [`zmidi_net_host_bye`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_bye(
    client: *mut ZmidiNetClient,
    now_ms: u64,
    addr: *const ZmidiNetAddr,
    reason: u8,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { bye(client.cast::<ClientSlot>(), now_ms, addr, reason, bufs) }
}

/// Gives the count of events that the Host dropped, or 0 for a null pointer.
///
/// # Safety
/// `host` is null or was set up by [`zmidi_net_host_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_dropped_events(host: *mut ZmidiNetHost) -> u32 {
    // SAFETY: caller contract above.
    unsafe { dropped(host.cast::<HostSlot>()) }
}

/// Gives the count of events that the Client dropped, or 0 for a null pointer.
///
/// # Safety
/// `client` is null or was set up by [`zmidi_net_client_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_dropped_events(client: *mut ZmidiNetClient) -> u32 {
    // SAFETY: caller contract above.
    unsafe { dropped(client.cast::<ClientSlot>()) }
}

/// Gives the size in bytes of the Rust state of a Host (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_net_host_size() -> usize {
    size_of::<HostSlot>()
}

/// Gives the size in bytes of the Rust state of a Client (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_net_client_size() -> usize {
    size_of::<ClientSlot>()
}

// ----- authentication (feature net-auth, zmidi-net auth, DEC-013) -----

/// One user of the user method (M2-124 6.10): a Username and its password, as bytes (UTF-8 for
/// the Username) in storage of the program, with no 0x00 terminator. A null pointer is allowed
/// for a length of 0.
#[cfg(feature = "net-auth")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiNetUser {
    /// The Username bytes.
    pub username: *const u8,
    /// The count of Username bytes.
    pub username_len: usize,
    /// The password bytes.
    pub password: *const u8,
    /// The count of password bytes.
    pub password_len: usize,
}

/// The credentials that the program gives to each receive call of the authentication
/// (M2-124 6.7 to 6.10). The program owns this struct and all the storage that it points to;
/// the C ABI keeps no pointer after the call and copies no secret and no password (N-52).
///
/// - Host: `shared_secret` verifies the digest of the shared secret method; `users` is the user
///   table of the user method; `random` gives 16 bytes for each new CryptoNonce, from
///   `random_used`.
/// - Client: `shared_secret` answers the shared secret method; `users[0]` is the Username and
///   the password that answer the user method. No value declines (Bye 0x80).
#[cfg(feature = "net-auth")]
#[repr(C)]
#[derive(Debug)]
pub struct ZmidiNetCredentials {
    /// The shared secret bytes; null: no shared secret.
    pub shared_secret: *const u8,
    /// The count of shared secret bytes.
    pub shared_secret_len: usize,
    /// The user table; null with a count of 0: no user.
    pub users: *const ZmidiNetUser,
    /// The count of users.
    pub user_count: usize,
    /// Host: random bytes for the CryptoNonces (M2-124 6.7); null with a length of 0: none.
    pub random: *const u8,
    /// The count of random bytes.
    pub random_len: usize,
    /// The count of random bytes used. The call adds 16 for each CryptoNonce; the program sets
    /// it to 0 when it writes new random bytes. When fewer than 16 bytes are left, a Host that
    /// needs a CryptoNonce gives a `CredentialsError` event (code 0).
    pub random_used: usize,
}

/// The [`Credentials`] of zmidi-net over a [`ZmidiNetCredentials`] (feature `net-auth`).
#[cfg(feature = "net-auth")]
mod creds {
    use zmidi_net::handshake::NONCE_RANDOM_LEN;
    use zmidi_net::Credentials;

    use super::{ZmidiNetCredentials, ZmidiNetUser};

    /// Gives the slice of a pointer and a length; `None` for a null pointer with a length above 0.
    ///
    /// # Safety
    /// `p` is null or valid for `len` items for `'a`.
    pub(super) unsafe fn c_slice<'a, T>(p: *const T, len: usize) -> Option<&'a [T]> {
        if p.is_null() {
            return (len == 0).then_some(&[]);
        }
        // SAFETY: caller contract above.
        Some(unsafe { core::slice::from_raw_parts(p, len) })
    }

    /// A user of the table, as slices: the Username and the password.
    type UserSlices<'a> = (&'a [u8], &'a [u8]);

    /// The [`Credentials`] of zmidi-net over a [`ZmidiNetCredentials`].
    pub(super) struct CCredentials<'a> {
        secret: Option<&'a [u8]>,
        users: &'a [ZmidiNetUser],
        random: &'a [u8],
        /// The count of random bytes used, for `random_used`.
        pub(super) used: usize,
    }

    impl CCredentials<'static> {
        /// No secret, no user, no random bytes.
        pub(super) const EMPTY: Self = CCredentials {
            secret: None,
            users: &[],
            random: &[],
            used: 0,
        };
    }

    impl<'a> CCredentials<'a> {
        /// Checks the pointers of `c` and gives the credentials; `None` for a null pointer with a
        /// length above 0 (also in a user).
        ///
        /// # Safety
        /// Each pointer of `c` and of its users is null or valid for its length for `'a`.
        pub(super) unsafe fn new(c: &ZmidiNetCredentials) -> Option<Self> {
            // SAFETY: caller contract above.
            let users = unsafe { c_slice(c.users, c.user_count)? };
            for u in users {
                // SAFETY: caller contract above.
                unsafe { Self::user_slices(u)? };
            }
            let secret = if c.shared_secret.is_null() {
                if c.shared_secret_len != 0 {
                    return None;
                }
                None
            } else {
                // SAFETY: caller contract above.
                Some(unsafe { c_slice(c.shared_secret, c.shared_secret_len)? })
            };
            Some(CCredentials {
                secret,
                users,
                // SAFETY: caller contract above.
                random: unsafe { c_slice(c.random, c.random_len)? },
                used: c.random_used,
            })
        }

        /// The Username and the password of `u`.
        ///
        /// # Safety
        /// As [`CCredentials::new`].
        unsafe fn user_slices(u: &ZmidiNetUser) -> Option<UserSlices<'a>> {
            // SAFETY: caller contract above.
            unsafe {
                Some((
                    c_slice(u.username, u.username_len)?,
                    c_slice(u.password, u.password_len)?,
                ))
            }
        }

        /// The users that [`CCredentials::new`] checked.
        fn user_list(&self) -> impl Iterator<Item = UserSlices<'a>> + 'a {
            self.users.iter().filter_map(|u| {
                // SAFETY: `new` checked each user, and the storage is valid for `'a` (its contract).
                unsafe { Self::user_slices(u) }
            })
        }
    }

    impl Credentials for CCredentials<'_> {
        fn random_nonce(&mut self) -> Option<[u8; NONCE_RANDOM_LEN]> {
            let end = self.used.checked_add(NONCE_RANDOM_LEN)?;
            let bytes = self.random.get(self.used..end)?.try_into().ok()?;
            self.used = end;
            Some(bytes)
        }

        fn shared_secret(&mut self) -> Option<&[u8]> {
            self.secret
        }

        fn password(&mut self, username: &[u8]) -> Option<&[u8]> {
            self.user_list()
                .find(|(u, _)| *u == username)
                .map(|(_, p)| p)
        }

        fn user(&mut self) -> Option<(&[u8], &[u8])> {
            self.user_list().next()
        }
    }
}

#[cfg(feature = "net-auth")]
use creds::CCredentials;

/// # Safety
/// As the receive_auth functions.
#[cfg(feature = "net-auth")]
#[allow(clippy::too_many_arguments)]
unsafe fn receive_auth<E: Endpoint>(
    e: *mut Option<E>,
    now: u64,
    from: *const ZmidiNetAddr,
    payload: *const u8,
    len: usize,
    creds: *mut ZmidiNetCredentials,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(c) = (unsafe { creds.as_mut() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let Some(mut cc) = (unsafe { CCredentials::new(c) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let st = unsafe { receive(e, now, from, payload, len, &mut cc, bufs) };
    c.random_used = cc.used;
    st
}

/// Sets up `host` as [`zmidi_net_host_init`] does, with a policy that asks each new Client for
/// authentication (M2-124 6.7, 6.8). `methods` is [`ZMIDI_NET_AUTH_SHARED_SECRET`],
/// [`ZMIDI_NET_AUTH_USER`] or both; with both, a Client that has both methods gets the user
/// method (N-44). A Client with no matching Capabilities bit gets Bye 0x45. Other values of
/// `methods` give invalid argument. Give the credentials with [`zmidi_net_host_receive_auth`].
///
/// # Safety
/// As [`zmidi_net_host_init`].
#[cfg(feature = "net-auth")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_init_auth(
    host: *mut ZmidiNetHost,
    name: *const u8,
    product_instance_id: *const u8,
    methods: u8,
) -> ZmidiStatus {
    let kind = match methods {
        ZMIDI_NET_AUTH_SHARED_SECRET => PolicyKind::SharedSecret,
        ZMIDI_NET_AUTH_USER => PolicyKind::UserList,
        0x03 => PolicyKind::Both,
        _ => return ZmidiStatus::InvalidArgument,
    };
    // SAFETY: caller contract above.
    unsafe { host_init(host, name, product_instance_id, HostPolicy::new(kind)) }
}

/// Sets up `client` as [`zmidi_net_client_init`] does, with the Capabilities bits `methods`
/// ([`ZMIDI_NET_AUTH_SHARED_SECRET`] D0, [`ZMIDI_NET_AUTH_USER`] D1, or both) in each
/// Invitation (M2-124 Table 10). Other values of `methods` give invalid argument. Give the
/// credentials with [`zmidi_net_client_receive_auth`].
///
/// # Safety
/// As [`zmidi_net_client_init`].
#[cfg(feature = "net-auth")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_init_auth(
    client: *mut ZmidiNetClient,
    name: *const u8,
    product_instance_id: *const u8,
    methods: u8,
) -> ZmidiStatus {
    if !(1..=3).contains(&methods) {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe {
        client_init(client, name, product_instance_id, |c| ClientConfig {
            capabilities: Capabilities::from_bits(methods),
            ..c
        })
    }
}

/// [`zmidi_net_host_receive`] with the credentials `creds` of the program: the shared secret
/// and the user table verify the digests, and `random` gives the CryptoNonces (the call sets
/// `creds->random_used`). A null pointer with a length above 0 in `creds` gives invalid
/// argument.
///
/// # Safety
/// As [`zmidi_net_host_receive`]; `creds` is valid and writable, and each of its pointers is null
/// or valid for its length.
#[cfg(feature = "net-auth")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_host_receive_auth(
    host: *mut ZmidiNetHost,
    now_ms: u64,
    from: *const ZmidiNetAddr,
    payload: *const u8,
    len: usize,
    creds: *mut ZmidiNetCredentials,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        receive_auth(
            host.cast::<HostSlot>(),
            now_ms,
            from,
            payload,
            len,
            creds,
            bufs,
        )
    }
}

/// [`zmidi_net_client_receive`] with the credentials `creds` of the program: the Client answers
/// a request for authentication of the Host at once with the shared secret or with `users[0]`;
/// with no value it declines (Bye 0x80).
///
/// # Safety
/// As [`zmidi_net_host_receive_auth`], for a `client` set up by [`zmidi_net_client_init`] or
/// [`zmidi_net_client_init_auth`].
#[cfg(feature = "net-auth")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_net_client_receive_auth(
    client: *mut ZmidiNetClient,
    now_ms: u64,
    from: *const ZmidiNetAddr,
    payload: *const u8,
    len: usize,
    creds: *mut ZmidiNetCredentials,
    bufs: *mut ZmidiNetBuffers,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        receive_auth(
            client.cast::<ClientSlot>(),
            now_ms,
            from,
            payload,
            len,
            creds,
            bufs,
        )
    }
}
