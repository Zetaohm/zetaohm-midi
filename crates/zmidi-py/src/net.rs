//! Network MIDI 2.0 Host and Client over UDP (zmidi-net, M2-124), sans-IO: the program owns the
//! socket, gives each received payload, and sends each payload that the endpoint writes.
//!
//! With zmidi-net `auth` (SHA-256, DEC-013), a Host can ask each new Client for authentication
//! with the shared secret method, the user method or both (M2-124 6.7 to 6.10), and a Client
//! answers with its shared secret or its user. The object keeps the credentials that the program
//! sets and gives them to zmidi-net at each receive call.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_net::command::ByeReason;
use zmidi_net::handshake::{CredentialsError, NONCE_RANDOM_LEN};
use zmidi_net::packet::MAX_PAYLOAD;
use zmidi_net::session::CloseCause;
use zmidi_net::{
    Capabilities, Client, ClientConfig, Diagnostic, EndpointError, Event, EventKind, Host,
    HostConfig, HostPolicy, Identity, Method, PolicyKind, SessionEvent,
};

use crate::{invalid, rejected};

/// The random bytes that a Host takes from its random source when its pool is low: 4 CryptoNonces.
const RANDOM_FILL: usize = 64;

/// One event of a peer.
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone)]
pub struct NetEvent {
    /// The kind: "established", "data", "ping_reply", "closed", and so on.
    #[pyo3(get)]
    kind: String,
    /// The peer address, `(ip, port)`.
    #[pyo3(get)]
    peer: (String, u16),
    /// A code of the kind: Ping Id, Bye Reason, NAK Reason, close cause or diagnostic; the method
    /// of "auth_requested", "auth_failed" and "established"; the code of "credentials_error".
    #[pyo3(get)]
    code: u32,
    /// A value of the kind: the round-trip time in ms of a "ping_reply", the Bye Reason of a
    /// "closed" event, the failure count of "auth_failed", the Authentication State of
    /// "auth_requested".
    #[pyo3(get)]
    value: u64,
    /// "data": the UMPs, each a list of words.
    #[pyo3(get)]
    umps: Vec<Vec<u32>>,
    /// "auth_requested", "auth_failed", "established": the authentication method, 1 shared
    /// secret or 2 user; `None` for a session with no authentication and for other kinds.
    #[pyo3(get)]
    method: Option<u8>,
    /// "auth_requested": the Authentication State (M2-124 Tables 15 and 17): 0x00 first request,
    /// 0x01 digest not correct, 0x02 Username not found.
    #[pyo3(get)]
    auth_state: Option<u8>,
    /// "auth_failed": true when the Host did not find the Username.
    #[pyo3(get)]
    username_not_found: bool,
    /// "credentials_error": the problem with the credentials of the program.
    #[pyo3(get)]
    credentials_error: Option<String>,
}

#[pymethods]
impl NetEvent {
    fn __repr__(&self) -> String {
        format!(
            "NetEvent(kind={:?}, peer={:?}, code={})",
            self.kind, self.peer, self.code
        )
    }
}

fn addr(a: (String, u16)) -> PyResult<SocketAddr> {
    let ip: IpAddr = a.0.parse().map_err(|_| invalid("not an IP address"))?;
    Ok(SocketAddr::new(ip, a.1))
}

fn py_addr(a: SocketAddr) -> (String, u16) {
    (a.ip().to_string(), a.port())
}

/// The method code: 1 shared secret, 2 user (the Capabilities bit of M2-124 Table 10).
fn method_code(m: Method) -> u8 {
    match m {
        Method::SharedSecret => 1,
        Method::User => 2,
    }
}

/// The code and the name of a credentials problem.
fn credentials_error(c: CredentialsError) -> (u32, &'static str) {
    match c {
        CredentialsError::NoRandom => (0, "no_random"),
        CredentialsError::NoSecret => (1, "no_secret"),
        CredentialsError::EmptySecret => (2, "empty_secret"),
        CredentialsError::EmptyPassword => (3, "empty_password"),
        CredentialsError::InvalidUsername => (4, "invalid_username"),
        CredentialsError::UsernameTooLong => (5, "username_too_long"),
    }
}

fn net_event(e: &Event<'_>) -> NetEvent {
    let mut out = NetEvent {
        kind: String::new(),
        peer: py_addr(e.peer),
        code: 0,
        value: 0,
        umps: Vec::new(),
        method: None,
        auth_state: None,
        username_not_found: false,
        credentials_error: None,
    };
    let (kind, code, value) = match e.kind {
        EventKind::Session(s) => match s {
            SessionEvent::InvitationReceived { .. } => ("invitation_received", 0, 0),
            SessionEvent::Pending => ("pending", 0, 0),
            SessionEvent::AuthRequested(c) => {
                let m = method_code(c.method);
                out.method = Some(m);
                out.auth_state = Some(c.state);
                ("auth_requested", u32::from(m), u64::from(c.state))
            }
            SessionEvent::AuthFailed {
                method,
                failures,
                username_not_found,
            } => {
                let m = method_code(method);
                out.method = Some(m);
                out.username_not_found = username_not_found;
                ("auth_failed", u32::from(m), u64::from(failures))
            }
            SessionEvent::CredentialsError(c) => {
                let (code, name) = credentials_error(c);
                out.credentials_error = Some(name.to_owned());
                ("credentials_error", code, 0)
            }
            SessionEvent::Established { method } => {
                out.method = method.map(method_code);
                ("established", u32::from(out.method.unwrap_or(0)), 0)
            }
            SessionEvent::Ending(r) => ("ending", u32::from(r.0), 0),
            SessionEvent::Closed(c) => match c {
                CloseCause::ByeReceived(b) => ("closed", 0, u64::from(b.reason.0)),
                CloseCause::ByeReply => ("closed", 1, 0),
                CloseCause::ByeTimeout => ("closed", 2, 0),
                CloseCause::Expired => ("closed", 3, 0),
                CloseCause::Ended(r) => ("closed", 4, u64::from(r.0)),
            },
            SessionEvent::Reset => ("reset", 0, 0),
            SessionEvent::ResetStarted => ("reset_started", 0, 0),
            SessionEvent::Nak(n) => ("nak", u32::from(n.reason.0), 0),
            SessionEvent::PingReply { id, rtt_ms } => ("ping_reply", id, rtt_ms),
            SessionEvent::UnexpectedPingReply(id) => ("unexpected_ping_reply", id, 0),
            SessionEvent::Timeout => ("timeout", 0, 0),
            SessionEvent::Stream(_) => ("stream", 0, 0),
        },
        EventKind::Data(d) => {
            out.umps = d.messages().map(|m| m.words().to_vec()).collect();
            ("data", 0, 0)
        }
        EventKind::Diagnostic(d) => match d {
            Diagnostic::BadSignature => ("diagnostic", 0, 0),
            Diagnostic::InvalidPeerAddress => ("diagnostic", 1, 0),
            Diagnostic::TableFull => ("diagnostic", 2, 0),
            Diagnostic::NakSent { reason, .. } => ("diagnostic", 3, u64::from(reason.0)),
        },
    };
    out.kind = kind.to_owned();
    out.code = code;
    out.value = value;
    out
}

fn endpoint_error(e: EndpointError) -> PyErr {
    match e {
        EndpointError::InvalidAddress => invalid(e.to_string()),
        _ => rejected(e.to_string()),
    }
}

/// The [`Credentials`] of zmidi-net over the values that the program sets.
mod creds {
    use std::collections::HashMap;

    use zeroize::Zeroize;
    use zmidi_net::handshake::NONCE_RANDOM_LEN;
    use zmidi_net::Credentials;

    /// The credentials that the program sets on a Host or a Client (M2-124 6.7 to 6.10).
    #[derive(Default)]
    pub(super) struct Creds {
        /// The shared secret: Host, to verify a digest; Client, to answer.
        pub(super) secret: Option<Vec<u8>>,
        /// Host: the user table, Username to password. Client: not used.
        pub(super) users: HashMap<Vec<u8>, Vec<u8>>,
        /// Client: the Username and the password that answer the user method.
        pub(super) user: Option<(Vec<u8>, Vec<u8>)>,
        /// Host: random bytes for the CryptoNonces, 16 for each.
        pub(super) random: Vec<u8>,
    }

    impl Creds {
        /// Sets the shared secret; clears the old one first (REQ-PY-NET-005).
        pub(super) fn set_secret(&mut self, secret: Option<Vec<u8>>) {
            if let Some(s) = self.secret.as_mut() {
                s.zeroize();
            }
            self.secret = secret;
        }

        /// Sets the user table; clears the old passwords first.
        pub(super) fn set_users(&mut self, users: HashMap<Vec<u8>, Vec<u8>>) {
            self.users.values_mut().for_each(Zeroize::zeroize);
            self.users = users;
        }

        /// Sets the Username and the password of a Client; clears the old password first.
        pub(super) fn set_user(&mut self, user: Option<(Vec<u8>, Vec<u8>)>) {
            if let Some((_, p)) = self.user.as_mut() {
                p.zeroize();
            }
            self.user = user;
        }
    }

    /// Clears the secret, the passwords and the random bytes (also the unused capacity of each
    /// buffer) when the Host or the Client goes (REQ-PY-NET-005).
    impl Drop for Creds {
        fn drop(&mut self) {
            self.clear();
        }
    }

    impl Creds {
        /// Sets each secret buffer to zero in place, with its unused capacity; the buffers stay
        /// allocated until the drop frees them.
        fn clear(&mut self) {
            if let Some(s) = self.secret.as_mut() {
                s.zeroize();
            }
            self.users.values_mut().for_each(Zeroize::zeroize);
            if let Some((_, p)) = self.user.as_mut() {
                p.zeroize();
            }
            self.random.zeroize();
        }
    }

    impl Credentials for Creds {
        fn random_nonce(&mut self) -> Option<[u8; NONCE_RANDOM_LEN]> {
            let bytes = self.random.get(..NONCE_RANDOM_LEN)?.try_into().ok()?;
            self.random.drain(..NONCE_RANDOM_LEN);
            Some(bytes)
        }

        fn shared_secret(&mut self) -> Option<&[u8]> {
            self.secret.as_deref()
        }

        fn password(&mut self, username: &[u8]) -> Option<&[u8]> {
            self.users.get(username).map(Vec::as_slice)
        }

        fn user(&mut self) -> Option<(&[u8], &[u8])> {
            self.user
                .as_ref()
                .map(|(u, p)| (u.as_slice(), p.as_slice()))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The bytes of the whole allocation of `v` (its capacity).
        fn raw(v: &Vec<u8>) -> &[u8] {
            // SAFETY: the allocation of `v` has `capacity` bytes, and zeroize writes every one
            // of them, so all are initialized here.
            unsafe { core::slice::from_raw_parts(v.as_ptr(), v.capacity()) }
        }

        #[test]
        fn clear_sets_every_secret_byte_to_zero() {
            // REQ-PY-NET-005.
            let mut c = Creds::default();
            c.set_secret(Some(b"5483".to_vec()));
            c.set_users(HashMap::from([(b"Rosa".to_vec(), b"RPBqBno".to_vec())]));
            c.set_user(Some((b"Rosa".to_vec(), b"RPBqBno".to_vec())));
            c.random = (1..=64).collect();
            assert!(c.random_nonce().is_some());
            c.clear();
            assert!(raw(c.secret.as_ref().unwrap()).iter().all(|&b| b == 0));
            assert!(raw(&c.users[b"Rosa".as_slice()]).iter().all(|&b| b == 0));
            assert!(raw(&c.user.as_ref().unwrap().1).iter().all(|&b| b == 0));
            assert!(raw(&c.random).iter().all(|&b| b == 0));
            assert!(c.random.capacity() >= 64);
        }
    }
}

use creds::Creds;

type Payloads<'py> = Vec<((String, u16), Bound<'py, PyBytes>)>;

/// The requests that the Host and the Client share.
trait Endpoint {
    fn rx<'py>(
        &mut self,
        py: Python<'py>,
        now: u64,
        from: SocketAddr,
        payload: &[u8],
        creds: &mut Creds,
    ) -> (Vec<NetEvent>, Payloads<'py>);
    fn poll_all(&mut self, now: u64) -> Vec<NetEvent>;
    fn pending_all<'py>(&mut self, py: Python<'py>, now: u64) -> Payloads<'py>;
}

macro_rules! endpoint_impl {
    ($t:ty) => {
        impl Endpoint for $t {
            fn rx<'py>(
                &mut self,
                py: Python<'py>,
                now: u64,
                from: SocketAddr,
                payload: &[u8],
                creds: &mut Creds,
            ) -> (Vec<NetEvent>, Payloads<'py>) {
                let mut r = self.receive(now, from, payload, creds);
                let events = r.by_ref().map(|e| net_event(&e)).collect();
                let mut replies = Vec::new();
                let mut buf = vec![0u8; MAX_PAYLOAD];
                while let Some((to, n)) = r.write_next(&mut buf) {
                    replies.push((py_addr(to), PyBytes::new(py, &buf[..n])));
                }
                (events, replies)
            }

            fn poll_all(&mut self, now: u64) -> Vec<NetEvent> {
                self.poll(now).map(|e| net_event(&e)).collect()
            }

            fn pending_all<'py>(&mut self, py: Python<'py>, now: u64) -> Payloads<'py> {
                let mut out = Vec::new();
                let mut buf = vec![0u8; MAX_PAYLOAD];
                while let Some((to, n)) = self.write_pending(now, &mut buf) {
                    out.push((py_addr(to), PyBytes::new(py, &buf[..n])));
                }
                out
            }
        }
    };
}

endpoint_impl!(Host);
endpoint_impl!(Client);

fn identity(name: &str, product_instance_id: &str) -> PyResult<Identity> {
    Identity::new(name, product_instance_id).map_err(|e| invalid(e.to_string()))
}

fn bye_events(
    r: Result<zmidi_net::session::Events<'static>, EndpointError>,
    peer: SocketAddr,
) -> PyResult<Vec<NetEvent>> {
    let events = r.map_err(endpoint_error)?;
    Ok(events
        .iter()
        .map(|e| {
            net_event(&Event {
                peer,
                kind: EventKind::Session(*e),
            })
        })
        .collect())
}

/// A Network MIDI 2.0 Host with 4 sessions. With no authentication it accepts each Client.
#[pyclass(module = "zetaohm_midi._native")]
pub struct NetHost {
    host: Box<Host>,
    creds: Creds,
    /// True when the Host asks new Clients for authentication (it needs CryptoNonces).
    auth: bool,
    /// The random source `(n) -> bytes`; `None` is `os.urandom`.
    random: Option<Py<PyAny>>,
}

#[pymethods]
impl NetHost {
    /// A Host with the UMP Endpoint Name `name` and the Product Instance Id
    /// `product_instance_id`. `auth` selects the authentication of new Clients (M2-124 6.7,
    /// 6.8): 0 none, 1 shared secret, 2 user, 3 both (a Client with both gets the user method,
    /// N-44). A Client with no matching Capabilities bit gets Bye 0x45. `random` is the source of
    /// the CryptoNonce bytes, a function `(n) -> bytes`; `None` is `os.urandom`.
    #[new]
    #[pyo3(signature = (name, product_instance_id, auth=0, random=None))]
    fn new(
        name: &str,
        product_instance_id: &str,
        auth: u8,
        random: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let kind = match auth {
            0 => PolicyKind::Accept,
            1 => PolicyKind::SharedSecret,
            2 => PolicyKind::UserList,
            3 => PolicyKind::Both,
            _ => return Err(invalid("auth is 0, 1, 2 or 3")),
        };
        let config = HostConfig {
            policy: HostPolicy::new(kind),
            ..HostConfig::new(identity(name, product_instance_id)?)
        };
        Host::new(config)
            .map(|h| Self {
                host: Box::new(h),
                creds: Creds::default(),
                auth: auth != 0,
                random,
            })
            .map_err(endpoint_error)
    }

    /// Reads one UDP `payload` from `peer` `(ip, port)` at `now_ms`. Gives `(events, replies)`;
    /// each reply is `((ip, port), payload)` to send at once.
    fn receive<'py>(
        &mut self,
        py: Python<'py>,
        now_ms: u64,
        peer: (String, u16),
        payload: &[u8],
    ) -> PyResult<(Vec<NetEvent>, Payloads<'py>)> {
        let from = addr(peer)?;
        // M2-124 6.7: each new Session needs a new CryptoNonce. Fill the pool from the random
        // source when fewer than 16 bytes are left.
        if self.auth && self.creds.random.len() < NONCE_RANDOM_LEN {
            let more: Vec<u8> = match &self.random {
                Some(f) => f.bind(py).call1((RANDOM_FILL,))?.extract()?,
                None => py
                    .import("os")?
                    .call_method1("urandom", (RANDOM_FILL,))?
                    .extract()?,
            };
            self.creds.random.extend_from_slice(&more);
        }
        Ok(self.host.rx(py, now_ms, from, payload, &mut self.creds))
    }

    /// Runs the timers at `now_ms` and gives their events.
    fn poll(&mut self, now_ms: u64) -> Vec<NetEvent> {
        self.host.poll_all(now_ms)
    }

    /// Gives all pending UDP payloads as `((ip, port), payload)`.
    fn write_pending<'py>(&mut self, py: Python<'py>, now_ms: u64) -> Payloads<'py> {
        self.host.pending_all(py, now_ms)
    }

    /// The next time in ms at which the Host needs a poll, or `None`.
    fn next_deadline(&self) -> Option<u64> {
        self.host.next_deadline()
    }

    /// Queues whole UMPs (`words`) for `peer`.
    fn send_ump(&mut self, peer: (String, u16), words: Vec<u32>) -> PyResult<()> {
        self.host
            .send_ump(addr(peer)?, &words)
            .map_err(endpoint_error)
    }

    /// Queues a Ping to `peer` and gives its Ping Id.
    fn ping(&mut self, now_ms: u64, peer: (String, u16)) -> PyResult<u32> {
        self.host
            .ping(now_ms, addr(peer)?, None)
            .map_err(endpoint_error)
    }

    /// Ends the session with `peer` with a Bye of `reason` and gives its events.
    #[pyo3(signature = (now_ms, peer, reason=1))]
    fn bye(&mut self, now_ms: u64, peer: (String, u16), reason: u8) -> PyResult<Vec<NetEvent>> {
        let a = addr(peer)?;
        bye_events(self.host.bye(now_ms, a, ByeReason(reason), ""), a)
    }

    /// The count of events that the Host dropped.
    #[getter]
    fn dropped_events(&self) -> u32 {
        self.host.dropped_events()
    }

    /// Sets the shared secret that verifies the digests of the shared secret method (M2-124
    /// 6.9), or removes it with `None`.
    fn set_shared_secret(&mut self, secret: Option<Vec<u8>>) {
        self.creds.set_secret(secret);
    }

    /// Sets the user table of the user method (M2-124 6.10): Username to password bytes. It
    /// replaces the table.
    fn set_users(&mut self, users: HashMap<String, Vec<u8>>) {
        self.creds.set_users(
            users
                .into_iter()
                .map(|(u, p)| (u.into_bytes(), p))
                .collect(),
        );
    }

    /// Adds random bytes for the CryptoNonces (M2-124 6.7): the Host takes 16 bytes for each.
    /// With fewer than 16 bytes, a Client that needs a CryptoNonce gives a "credentials_error"
    /// event ("no_random").
    fn add_random(&mut self, data: &[u8]) {
        self.creds.random.extend_from_slice(data);
    }

    /// The count of random bytes that the Host did not use.
    #[getter]
    fn random_left(&self) -> usize {
        self.creds.random.len()
    }
}

/// A Network MIDI 2.0 Client with 1 session.
#[pyclass(module = "zetaohm_midi._native")]
pub struct NetClient {
    client: Box<Client>,
    creds: Creds,
}

#[pymethods]
impl NetClient {
    /// A Client with the UMP Endpoint Name `name` and the Product Instance Id
    /// `product_instance_id`. `auth` gives the Capabilities bits of its Invitations (M2-124
    /// Table 10): 0 none, 1 shared secret (D0), 2 user (D1), 3 both.
    #[new]
    #[pyo3(signature = (name, product_instance_id, auth=0))]
    fn new(name: &str, product_instance_id: &str, auth: u8) -> PyResult<Self> {
        if auth > 3 {
            return Err(invalid("auth is 0, 1, 2 or 3"));
        }
        let config = ClientConfig {
            capabilities: Capabilities::from_bits(auth),
            ..ClientConfig::new(identity(name, product_instance_id)?)
        };
        Client::new(config)
            .map(|c| Self {
                client: Box::new(c),
                creds: Creds::default(),
            })
            .map_err(endpoint_error)
    }

    /// Starts an Invitation to the Host at `host` `(ip, port)`; it goes out with `write_pending`.
    fn connect(&mut self, now_ms: u64, host: (String, u16)) -> PyResult<()> {
        self.client
            .connect(now_ms, addr(host)?)
            .map_err(endpoint_error)
    }

    /// Reads one UDP `payload` from `peer` `(ip, port)` at `now_ms`. Gives `(events, replies)`.
    fn receive<'py>(
        &mut self,
        py: Python<'py>,
        now_ms: u64,
        peer: (String, u16),
        payload: &[u8],
    ) -> PyResult<(Vec<NetEvent>, Payloads<'py>)> {
        Ok(self
            .client
            .rx(py, now_ms, addr(peer)?, payload, &mut self.creds))
    }

    /// Runs the timers at `now_ms` and gives their events.
    fn poll(&mut self, now_ms: u64) -> Vec<NetEvent> {
        self.client.poll_all(now_ms)
    }

    /// Gives all pending UDP payloads as `((ip, port), payload)`.
    fn write_pending<'py>(&mut self, py: Python<'py>, now_ms: u64) -> Payloads<'py> {
        self.client.pending_all(py, now_ms)
    }

    /// The next time in ms at which the Client needs a poll, or `None`.
    fn next_deadline(&self) -> Option<u64> {
        self.client.next_deadline()
    }

    /// Queues whole UMPs (`words`) for `peer`.
    fn send_ump(&mut self, peer: (String, u16), words: Vec<u32>) -> PyResult<()> {
        self.client
            .send_ump(addr(peer)?, &words)
            .map_err(endpoint_error)
    }

    /// Queues a Ping to `peer` and gives its Ping Id.
    fn ping(&mut self, now_ms: u64, peer: (String, u16)) -> PyResult<u32> {
        self.client
            .ping(now_ms, addr(peer)?, None)
            .map_err(endpoint_error)
    }

    /// Ends the session with `peer` with a Bye of `reason` and gives its events.
    #[pyo3(signature = (now_ms, peer, reason=1))]
    fn bye(&mut self, now_ms: u64, peer: (String, u16), reason: u8) -> PyResult<Vec<NetEvent>> {
        let a = addr(peer)?;
        bye_events(self.client.bye(now_ms, a, ByeReason(reason), ""), a)
    }

    /// The count of events that the Client dropped.
    #[getter]
    fn dropped_events(&self) -> u32 {
        self.client.dropped_events()
    }

    /// Sets the shared secret that answers the shared secret method (M2-124 6.9), or removes it
    /// with `None` (the Client then declines with Bye 0x80).
    fn set_shared_secret(&mut self, secret: Option<Vec<u8>>) {
        self.creds.set_secret(secret);
    }

    /// Sets the Username and the password that answer the user method (M2-124 6.10), or removes
    /// them with `None` (the Client then declines with Bye 0x80).
    #[pyo3(signature = (username, password=Vec::new()))]
    fn set_user(&mut self, username: Option<String>, password: Vec<u8>) {
        self.creds
            .set_user(username.map(|u| (u.into_bytes(), password)));
    }
}
