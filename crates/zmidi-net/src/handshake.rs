//! The invitation and the authentication of one session, for the Host role and for the Client
//! role (M2-124-UM v1.0.1 sections 6.1, 6.4 to 6.10).
//!
//! - [`Handshake`] is the invitation state of ONE session entry. The Session module keeps it
//!   next to the [`SessionState`] and gives that state to each call. The Handshake does not own
//!   the state: each call returns an [`Outcome`] with the reply to write, the next state, the
//!   change of the repeat and the events for the caller.
//! - It handles every invitation-family Command (0x01, 0x02, 0x03, 0x10, 0x11, 0x12, 0x13) in
//!   every state, also in Idle (an address with no entry: use a new Handshake and the state
//!   Idle). Other Commands give an empty Outcome.
//! - Sans-IO (DEC-012): the caller gives the time as u64 ms, the random bytes and the
//!   credentials ([`Credentials`]). No clock, no I/O, no alloc (DEC-002). Nothing panics on
//!   received bytes.
//! - Authentication (Commands 0x02, 0x03, 0x12, 0x13 with a payload) needs the Cargo feature
//!   `auth` (DEC-013). The public API is the same without the feature: the Host then never asks
//!   for authentication, the Client writes Capabilities D0 = D1 = 0 and the authentication
//!   Commands get the replies of REQ-NET-AUTH-012, -027, -030, -031 and -032.
//! - The entry keeps the CryptoNonce (16 bytes), the digest (32 bytes) and the Client Username
//!   (at most `U` bytes, default 128); it never keeps a secret or a password (N-52).
//! - The readings that M2-124 does not give are the choices of DEC-NET-001 (N-04, N-05, N-06,
//!   N-16, N-19, N-22, N-25, N-44 to N-48, N-51 to N-56).

#[cfg(feature = "auth")]
use crate::auth::{
    self, AuthDigest, AuthError, AuthRequired, AuthState, CryptoNonce, InvitationWithAuth,
    InvitationWithUserAuth, UserAuthRequired, UserAuthState, MAX_USERNAME,
};
use crate::command::{
    Bye, ByeReason, Capabilities, Command, CommandCode, CommandError, Invitation, InvitationReply,
    Nak, NakReason, Role, SessionState,
};
use crate::identity::{Identity, MAX_ENDPOINT_NAME, MAX_PRODUCT_INSTANCE_ID};
#[cfg(feature = "auth")]
use crate::packet::RawCommand;
use crate::packet::{Header, NetStr, PayloadWriter};

#[cfg(feature = "auth")]
pub use crate::auth::FailureDelay;

/// The delay of the reply to a failed authentication. Without the feature `auth` this is an
/// empty type with the same constructors, so that the API does not change (DEC-013); a Host
/// without the feature never fails an authentication.
#[cfg(not(feature = "auth"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FailureDelay {
    _private: (),
}

#[cfg(not(feature = "auth"))]
impl FailureDelay {
    /// A delay with the configuration values `initial_ms` and `max_ms` (not used without the
    /// feature `auth`).
    pub const fn new(initial_ms: u64, max_ms: u64) -> Self {
        let _ = (initial_ms, max_ms);
        FailureDelay { _private: () }
    }
}

/// Number of random bytes for one CryptoNonce (6.7).
pub const NONCE_RANDOM_LEN: usize = 16;
/// Default capacity of the Username buffer of a Client entry, in bytes (N-52).
pub const DEFAULT_USERNAME_CAPACITY: usize = 128;
/// Default Invitation timeout, in ms (REQ-NET-INV-011, N-22).
pub const DEFAULT_INVITATION_TIMEOUT_MS: u64 = 10_000;
/// Default Pending timeout, in ms (REQ-NET-INV-027, N-22).
pub const DEFAULT_PENDING_TIMEOUT_MS: u64 = 60_000;
/// Default Host answer timeout, in ms (REQ-NET-SES-002, N-25).
pub const DEFAULT_HOST_ANSWER_TIMEOUT_MS: u64 = 60_000;
/// Default limit of failed authentications of one session entry (REQ-NET-AUTH-022, N-46).
pub const DEFAULT_FAILURE_LIMIT: u32 = 5;

/// Error of the Handshake writer and of the caller decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HandshakeError {
    /// The Command writer refused (does not fit, too large).
    #[error("command: {0}")]
    Command(#[from] CommandError),
    /// An authentication Command in a build without the feature `auth`, or a HostPolicy that
    /// needs authentication in such a build (DEC-013).
    #[error("authentication not available")]
    AuthNotAvailable,
    /// The reply needs a CryptoNonce or a digest that the entry does not keep.
    #[error("no data for this reply")]
    NothingToWrite,
    /// The authentication writer refused the kept data.
    #[error("invalid authentication data")]
    InvalidAuthData,
    /// A Bye Reason that is not in Table 27 or not allowed for the role (REQ-NET-BYE-004).
    #[error("reason not allowed")]
    ReasonNotAllowed,
    /// A configuration value out of range (a failure limit of 0).
    #[error("invalid configuration")]
    InvalidConfig,
}

#[cfg(feature = "auth")]
fn from_auth(e: AuthError) -> HandshakeError {
    match e {
        AuthError::Command(c) => HandshakeError::Command(c),
        _ => HandshakeError::InvalidAuthData,
    }
}

/// A problem with the values of the [`Credentials`] of the caller (N-53). The Handshake
/// reports it as an event and continues as the event says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CredentialsError {
    /// Host: no random bytes for a CryptoNonce; the Host does as for no matching method.
    NoRandom,
    /// Host: no shared secret to verify a digest; counted as a failed authentication.
    NoSecret,
    /// An empty shared secret (N-53). Host: a failed authentication. Client: a decline.
    EmptySecret,
    /// An empty password (N-53). Host: a failed authentication. Client: a decline.
    EmptyPassword,
    /// Client: an empty Username, one that is not UTF-8, contains 0x00 or starts with a Byte
    /// Order Mark (REQ-NET-AUTH-044); a decline.
    InvalidUsername,
    /// Client: a Username longer than `U` bytes or than 988 bytes; a decline.
    UsernameTooLong,
}

/// An authentication method of 6.9 and 6.10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// Shared secret: Invitation Reply: Authentication Required (0x12) and Invitation with
    /// Authentication (0x02); Capabilities bit D0.
    SharedSecret,
    /// User list: Invitation Reply: User Authentication Required (0x13) and Invitation with
    /// User Authentication (0x03); Capabilities bit D1.
    User,
}

/// The Authentication State of a received 0x12 or 0x13, for the Client caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Challenge {
    /// The method that the Host asks for.
    pub method: Method,
    /// The raw Authentication State (Tables 15 and 17): 0x00 first request, 0x01 digest not
    /// correct, 0x02 Username not found (0x13 only); other values count as 0x00.
    pub state: u8,
}

/// The source of the secret values, given to each call that can need one (N-52). The
/// Handshake copies no secret and no password. The trait is not behind the feature `auth`, so
/// that the API is the same in each build; without the feature no method is called.
pub trait Credentials {
    /// Host: 16 random bytes for a new CryptoNonce (REQ-NET-AUTH-018); give new bytes at each
    /// call. `None`: the Host can not ask for authentication.
    fn random_nonce(&mut self) -> Option<[u8; NONCE_RANDOM_LEN]>;
    /// Host: the shared secret to verify a digest. Client: the shared secret for the Host of
    /// this session. `None`: Host: no secret; Client: the caller declines (Bye 0x80).
    fn shared_secret(&mut self) -> Option<&[u8]>;
    /// Host: the password of `username` (the received bytes up to the first 0x00). `None`:
    /// the Username is not found (REQ-NET-AUTH-025).
    fn password(&mut self, username: &[u8]) -> Option<&[u8]>;
    /// Client: the Username and the password for the Host of this session. `None`: the
    /// caller declines (Bye 0x80, REQ-NET-AUTH-035).
    fn user(&mut self) -> Option<(&[u8], &[u8])>;
    /// Client: called when the Client accepts a challenge, before `shared_secret` or `user`.
    /// Return false to answer later with [`Handshake::answer`] (for example after a user
    /// prompt); the Client then waits in Authentication Required. The default is true.
    fn answer_now(&mut self, challenge: Challenge) -> bool {
        let _ = challenge;
        true
    }
}

/// [`Credentials`] that give nothing: no random bytes, no secret, no user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoCredentials;

impl Credentials for NoCredentials {
    fn random_nonce(&mut self) -> Option<[u8; NONCE_RANDOM_LEN]> {
        None
    }
    fn shared_secret(&mut self) -> Option<&[u8]> {
        None
    }
    fn password(&mut self, _username: &[u8]) -> Option<&[u8]> {
        None
    }
    fn user(&mut self) -> Option<(&[u8], &[u8])> {
        None
    }
}

/// What the Host does with an Invitation from a new Client (REQ-NET-INV-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyKind {
    /// Accept each Client.
    Accept,
    /// Reply Pending and ask the caller (REQ-NET-INV-026).
    AskCaller,
    /// Require the shared secret method (D0).
    SharedSecret,
    /// Require the user list method (D1).
    UserList,
    /// Require one of the two methods; `preferred` selects when the Client has both (N-44).
    Both,
}

/// The choice of the Host for new Clients (REQ-NET-INV-012, -014, -015, REQ-NET-AUTH-015).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostPolicy {
    /// The policy.
    pub kind: PolicyKind,
    /// When an authentication policy finds no matching Capabilities bit: true gives
    /// Invitation Reply: Pending and the caller decides (REQ-NET-INV-014); false gives Bye
    /// 0x45 (REQ-NET-INV-015).
    pub pending_fallback: bool,
    /// The method for [`PolicyKind::Both`] when the Client sets D0 and D1 (default user
    /// authentication, N-44).
    pub preferred: Method,
}

impl HostPolicy {
    /// Accept, no fallback, preferred method user authentication.
    pub const DEFAULT: HostPolicy = HostPolicy {
        kind: PolicyKind::Accept,
        pending_fallback: false,
        preferred: Method::User,
    };

    /// A policy of `kind` with the other values of [`HostPolicy::DEFAULT`].
    pub const fn new(kind: PolicyKind) -> Self {
        HostPolicy {
            kind,
            ..HostPolicy::DEFAULT
        }
    }

    /// True when the policy needs authentication.
    pub const fn needs_auth(&self) -> bool {
        matches!(
            self.kind,
            PolicyKind::SharedSecret | PolicyKind::UserList | PolicyKind::Both
        )
    }

    /// `AuthNotAvailable` for an authentication policy in a build without the feature
    /// `auth`. Such a policy still works: no Client has a matching method, so the Host gives
    /// the Pending fallback or Bye 0x45.
    pub const fn check(&self) -> Result<(), HandshakeError> {
        if self.needs_auth() && !cfg!(feature = "auth") {
            return Err(HandshakeError::AuthNotAvailable);
        }
        Ok(())
    }
}

impl Default for HostPolicy {
    fn default() -> Self {
        HostPolicy::DEFAULT
    }
}

/// The timeouts and the failure limit of a Handshake (N-22, N-25, N-46, N-55). The repeat
/// interval belongs to the Session (REQ-NET-SES-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandshakeConfig {
    /// Client: the Invitation timeout; `None` repeats until a reply or a cancel
    /// (REQ-NET-INV-011).
    pub invitation_timeout_ms: Option<u64>,
    /// Client: the Pending timeout, from the first Invitation Reply: Pending
    /// (REQ-NET-INV-027).
    pub pending_timeout_ms: u64,
    /// Host: the answer timeout of an entry in Pending Invitation or Authentication Required,
    /// from the last Command of that Client (REQ-NET-SES-002).
    pub host_answer_timeout_ms: u64,
    /// Client: start the Invitation timeout again for each new CryptoNonce or Authentication
    /// State (REQ-NET-AUTH-038, N-55).
    pub restart_timeout_on_challenge: bool,
    /// Host: the number of failed authentications of one entry that gives Bye 0x43 or 0x44
    /// (REQ-NET-AUTH-022, N-46). At least 1.
    pub failure_limit: u32,
}

impl HandshakeConfig {
    /// 10000 ms, 60000 ms, 60000 ms, restart on, limit 5.
    pub const DEFAULT: HandshakeConfig = HandshakeConfig {
        invitation_timeout_ms: Some(DEFAULT_INVITATION_TIMEOUT_MS),
        pending_timeout_ms: DEFAULT_PENDING_TIMEOUT_MS,
        host_answer_timeout_ms: DEFAULT_HOST_ANSWER_TIMEOUT_MS,
        restart_timeout_on_challenge: true,
        failure_limit: DEFAULT_FAILURE_LIMIT,
    };

    /// `InvalidConfig` for a failure limit of 0.
    pub const fn check(&self) -> Result<(), HandshakeError> {
        if self.failure_limit == 0 {
            return Err(HandshakeError::InvalidConfig);
        }
        Ok(())
    }
}

impl Default for HandshakeConfig {
    fn default() -> Self {
        HandshakeConfig::DEFAULT
    }
}

/// The UMP Endpoint Name and the Product Instance Id of the peer, copied from a received
/// Command (the payload is borrowed). The bytes are as received up to the first 0x00 and are
/// not checked (N-32); a longer field is cut at 98 or 42 bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerInfo {
    name: [u8; MAX_ENDPOINT_NAME],
    name_len: u8,
    id: [u8; MAX_PRODUCT_INSTANCE_ID],
    id_len: u8,
    truncated: bool,
}

impl core::fmt::Debug for PeerInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PeerInfo")
            .field("name", &NetStr::from_field(self.name()))
            .field(
                "product_instance_id",
                &NetStr::from_field(self.product_instance_id()),
            )
            .finish()
    }
}

impl PeerInfo {
    /// A copy of two received fields.
    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    pub fn new(name: NetStr<'_>, product_instance_id: NetStr<'_>) -> Self {
        let mut p = PeerInfo {
            name: [0; MAX_ENDPOINT_NAME],
            name_len: 0,
            id: [0; MAX_PRODUCT_INSTANCE_ID],
            id_len: 0,
            truncated: false,
        };
        let n = name.as_bytes();
        let nl = n.len().min(MAX_ENDPOINT_NAME);
        p.name[..nl].copy_from_slice(&n[..nl]);
        p.name_len = nl as u8;
        let i = product_instance_id.as_bytes();
        let il = i.len().min(MAX_PRODUCT_INSTANCE_ID);
        p.id[..il].copy_from_slice(&i[..il]);
        p.id_len = il as u8;
        p.truncated = nl < n.len() || il < i.len();
        p
    }

    /// The UMP Endpoint Name bytes.
    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.name_len)]
    }

    /// The Product Instance Id bytes.
    pub fn product_instance_id(&self) -> &[u8] {
        &self.id[..usize::from(self.id_len)]
    }

    /// The name as UTF-8 text, checked (N-32).
    pub fn name_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.name())
    }

    /// The Product Instance Id as text, checked (N-32).
    pub fn product_instance_id_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.product_instance_id())
    }

    /// True when a received field was longer than 98 or 42 bytes and was cut.
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// One Command that the Handshake asks the caller to write. [`Reply::write`] writes it with
/// the data that the Handshake keeps (identity of the caller, nonce, digest, Username).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reply {
    /// Client: Invitation (0x01) with the identity and the Capabilities of the Client.
    Invitation,
    /// Client: Invitation with Authentication (0x02) with the kept digest.
    InvitationWithAuth,
    /// Client: Invitation with User Authentication (0x03) with the kept digest and Username.
    InvitationWithUserAuth,
    /// Host: Invitation Reply: Accepted (0x10).
    Accepted,
    /// Host: Invitation Reply: Pending (0x11).
    Pending,
    /// Host: Invitation Reply: Authentication Required (0x12) with the kept CryptoNonce and
    /// this Authentication State.
    AuthRequired {
        /// Authentication State (Table 15).
        state: u8,
    },
    /// Host: Invitation Reply: User Authentication Required (0x13) with the kept CryptoNonce
    /// and this Authentication State.
    UserAuthRequired {
        /// Authentication State (Table 17).
        state: u8,
    },
    /// Bye (0xF0) with this reason and no Text Message.
    Bye(ByeReason),
    /// NAK (0x8F) with this reason, the header word of the refused Command and no Text
    /// Message.
    Nak(NakReason, Header),
}

impl Reply {
    /// Writes the Command. `identity` is the identity of this endpoint (the caller). The
    /// Handshake `hs` gives the role (for the Bye check), the Capabilities and the kept
    /// authentication data.
    pub fn write<const U: usize>(
        &self,
        hs: &Handshake<U>,
        identity: &Identity,
        w: &mut PayloadWriter<'_>,
    ) -> Result<(), HandshakeError> {
        let name = &identity.name;
        let id = &identity.product_instance_id;
        match *self {
            Reply::Invitation => Invitation::write(w, name, id, hs.capabilities())?,
            Reply::Accepted => InvitationReply::write(w, false, name, id)?,
            Reply::Pending => InvitationReply::write(w, true, name, id)?,
            Reply::Bye(reason) => Bye::write(w, hs.role, reason, "")?,
            Reply::Nak(reason, header) => Nak::write(w, reason, header, "")?,
            Reply::InvitationWithAuth
            | Reply::InvitationWithUserAuth
            | Reply::AuthRequired { .. }
            | Reply::UserAuthRequired { .. } => hs.write_auth(*self, identity, w)?,
        }
        Ok(())
    }
}

/// The change of the repeated Command of the Session (REQ-NET-SES-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Repeat {
    /// No change.
    #[default]
    Keep,
    /// The reply of this Outcome is a new Command to repeat: send it now and again at each
    /// repeat interval from now (REQ-NET-INV-009, REQ-NET-AUTH-037). The repeats have the same
    /// bytes, because the Handshake keeps the data.
    Start(Reply),
    /// Stop the repeats of the invitation Command.
    Stop,
}

/// An event of the Handshake for the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandshakeEvent {
    /// Host: a new Client asks for a session and the caller decides: call
    /// [`Handshake::accept`] or [`Handshake::cancel`] (REQ-NET-INV-026, -014).
    InvitationReceived {
        /// The identity of the Client.
        peer: PeerInfo,
        /// The Capabilities of the Invitation.
        capabilities: Capabilities,
    },
    /// The session is established (REQ-NET-INV-020). Host: the identity of the Client;
    /// Client: the identity of the Host from Invitation Reply: Accepted.
    Established {
        /// The identity of the peer.
        peer: PeerInfo,
        /// The authentication method, if any.
        method: Option<Method>,
    },
    /// Client: the first Invitation Reply: Pending of the Invitation, with the identity of
    /// the Host (REQ-NET-INV-027).
    Pending {
        /// The identity of the Host.
        peer: PeerInfo,
    },
    /// Client: the Host asks for authentication (REQ-NET-AUTH-034, -035).
    AuthRequested {
        /// The identity of the Host.
        peer: PeerInfo,
        /// The method and the Authentication State.
        challenge: Challenge,
    },
    /// Host: a failed authentication of this entry (REQ-NET-AUTH-022, -025, -046).
    AuthFailed {
        /// The method that the Host asked for.
        method: Method,
        /// The number of failed authentications of this entry.
        failures: u32,
        /// True when the Username was not found.
        username_not_found: bool,
    },
    /// This endpoint ends the handshake with a Bye of this reason (timeout, cancel, decline,
    /// reject, failure limit, unexpected reply).
    Ended(ByeReason),
    /// Host: the Host answer timeout ended; the entry goes to Idle with no Command
    /// (REQ-NET-SES-002).
    Expired,
    /// A problem with the credentials of the caller (N-53).
    CredentialsError(CredentialsError),
}

/// The events of one Outcome: at most 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Events {
    items: [Option<HandshakeEvent>; 3],
}

impl Events {
    fn push(&mut self, e: HandshakeEvent) {
        if let Some(slot) = self.items.iter_mut().find(|s| s.is_none()) {
            *slot = Some(e);
        }
    }

    /// The events, in order.
    pub fn iter(&self) -> impl Iterator<Item = &HandshakeEvent> {
        self.items.iter().flatten()
    }

    /// True when there is no event.
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(Option::is_none)
    }

    /// The number of events.
    pub fn len(&self) -> usize {
        self.iter().count()
    }
}

/// The result of one Handshake call.
///
/// - `reply`: the Command to write to the peer (with [`Reply::write`]), if any.
/// - `next`: the next session state; `None` keeps the state. `Some(Idle)` deletes the entry
///   (the Handshake has then cleared its data). For a call in Idle (no entry), the caller
///   keeps an entry only when `next` is a state other than Idle; a Bye or a NAK from Idle is
///   sent once and no entry is kept (N-16).
/// - `repeat`: the change of the repeated Command.
/// - `events`: the events for the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Outcome {
    /// The Command to write.
    pub reply: Option<Reply>,
    /// The next session state.
    pub next: Option<SessionState>,
    /// The change of the repeat.
    pub repeat: Repeat,
    /// The events.
    pub events: Events,
}

impl Outcome {
    fn reply(r: Reply) -> Self {
        Outcome {
            reply: Some(r),
            ..Outcome::default()
        }
    }

    fn nak(reason: NakReason, header: Header) -> Self {
        Outcome::reply(Reply::Nak(reason, header))
    }

    fn not_expected(header: Header) -> Self {
        Outcome::nak(NakReason::COMMAND_NOT_EXPECTED, header)
    }

    fn next(mut self, s: SessionState) -> Self {
        self.next = Some(s);
        self
    }

    fn event(mut self, e: HandshakeEvent) -> Self {
        self.events.push(e);
        self
    }

    fn repeat(mut self, r: Repeat) -> Self {
        self.repeat = r;
        self
    }

    /// True when the Outcome has nothing: no reply, no state change, no event.
    pub fn is_empty(&self) -> bool {
        self.reply.is_none()
            && self.next.is_none()
            && self.repeat == Repeat::Keep
            && self.events.is_empty()
    }
}

/// What the caller gives to [`Handshake::on_command`] and [`Handshake::poll`]: the
/// credentials, the Host-wide failure delay and the Host policy. A Client uses only `creds`.
pub struct Env<'a, C: Credentials + ?Sized> {
    /// The credentials provider.
    pub creds: &'a mut C,
    /// The failure delay of the Host (one for the Host, N-45); `None` sends each reply at
    /// once.
    pub delay: Option<&'a mut FailureDelay>,
    /// The Host policy.
    pub policy: HostPolicy,
}

impl<'a, C: Credentials + ?Sized> Env<'a, C> {
    /// `creds`, no delay, the default policy.
    pub fn new(creds: &'a mut C) -> Self {
        Env {
            creds,
            delay: None,
            policy: HostPolicy::DEFAULT,
        }
    }

    /// The same with a failure delay.
    pub fn with_delay(mut self, delay: &'a mut FailureDelay) -> Self {
        self.delay = Some(delay);
        self
    }

    /// The same with a policy.
    pub fn with_policy(mut self, policy: HostPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// A reply of the Host that waits for its due time (REQ-NET-AUTH-047).
#[cfg(feature = "auth")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Delayed {
    reply: Reply,
    next: Option<SessionState>,
    due: u64,
}

/// The authentication data of one entry (feature `auth`).
#[cfg(feature = "auth")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthData<const U: usize> {
    /// Host: the CryptoNonce sent; Client: the CryptoNonce of the last accepted challenge.
    nonce: Option<CryptoNonce>,
    /// The method of the challenge.
    asked: Option<Method>,
    /// Client: the Authentication State of the last accepted challenge.
    state: u8,
    /// Client: the challenge that the kept digest answers (method, nonce, state).
    answered: Option<(Method, CryptoNonce, u8)>,
    /// Client: the digest for the repeats. Host: the accepted digest (REQ-NET-AUTH-029).
    digest: Option<AuthDigest>,
    /// Host: the method of the accepted digest.
    accepted: Option<Method>,
    /// Client: true while the caller has to call `answer`.
    waiting: bool,
    /// Client: the Username of the last Invitation with User Authentication.
    username: [u8; U],
    username_len: usize,
    /// Host: the delayed reply.
    delayed: Option<Delayed>,
}

#[cfg(feature = "auth")]
impl<const U: usize> AuthData<U> {
    const EMPTY: Self = AuthData {
        nonce: None,
        asked: None,
        state: 0,
        answered: None,
        digest: None,
        accepted: None,
        waiting: false,
        username: [0; U],
        username_len: 0,
        delayed: None,
    };
}

/// A clock time in ms, or none: 8 bytes, where `Option<u64>` takes 16 on a 32-bit target
/// (OPT-012). `u64::MAX` is "none"; a deadline at `u64::MAX` (about 584 million years) never
/// comes in either form, so the behavior is the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Deadline(u64);

impl Deadline {
    /// No deadline.
    pub(crate) const NONE: Deadline = Deadline(u64::MAX);

    /// The deadline at `t`.
    pub(crate) const fn at(t: u64) -> Deadline {
        Deadline(t)
    }

    /// The deadline, or none.
    pub(crate) const fn from(t: Option<u64>) -> Deadline {
        match t {
            Some(t) => Deadline(t),
            None => Deadline::NONE,
        }
    }

    /// The time, or `None`.
    pub(crate) const fn get(self) -> Option<u64> {
        if self.0 == u64::MAX {
            None
        } else {
            Some(self.0)
        }
    }
}

/// The invitation and authentication state of one session entry (Host or Client role).
///
/// Use:
/// - Client: [`Handshake::client`], then [`Handshake::start`] (writes the Invitation, state
///   Pending Invitation). Host: [`Handshake::host`] for each new address, then
///   [`Handshake::on_command`] with the state Idle.
/// - Give each received invitation-family Command to [`Handshake::on_command`], with the
///   current state. Give the time to [`Handshake::poll`] at [`Handshake::next_deadline`].
/// - Repeat [`Handshake::repeating`] at each repeat interval while the Outcome says so.
/// - Call [`Handshake::touch`] for each other Command from the peer (Host answer timeout),
///   and [`Handshake::clear`] when the Session goes to Idle by itself (Bye).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handshake<const U: usize = DEFAULT_USERNAME_CAPACITY> {
    role: Role,
    /// Client: the Capabilities sent; Host: the Capabilities received.
    caps: Capabilities,
    peer: Option<PeerInfo>,
    /// Client: the end of the Invitation timeout.
    invitation_deadline: Deadline,
    /// Client: the end of the Pending timeout, from the first Pending.
    pending_deadline: Deadline,
    /// Client: a Pending was received for the current Command.
    pending_seen: bool,
    /// Host: the end of the Host answer timeout.
    answer_deadline: Deadline,
    /// Client: the Command to repeat.
    repeating: Option<Reply>,
    /// Host: failed authentications of this entry.
    failures: u32,
    #[cfg(feature = "auth")]
    auth: AuthData<U>,
}

impl<const U: usize> Handshake<U> {
    const fn new(role: Role, caps: Capabilities) -> Self {
        Handshake {
            role,
            caps,
            peer: None,
            invitation_deadline: Deadline::NONE,
            pending_deadline: Deadline::NONE,
            pending_seen: false,
            answer_deadline: Deadline::NONE,
            repeating: None,
            failures: 0,
            #[cfg(feature = "auth")]
            auth: AuthData::EMPTY,
        }
    }

    /// A Client entry. `capabilities` gives the methods that the caller enables for this Host
    /// (REQ-NET-AUTH-013); without the feature `auth` D0 and D1 are always 0.
    pub const fn client(capabilities: Capabilities) -> Self {
        let caps = if cfg!(feature = "auth") {
            capabilities
        } else {
            Capabilities {
                auth: false,
                user_auth: false,
            }
        };
        Handshake::new(Role::Client, caps)
    }

    /// A Host entry for one Client address. The entry keeps no [`HandshakeConfig`]: the steps
    /// that read it take it (OPT-009).
    pub const fn host() -> Self {
        Handshake::new(Role::Host, Capabilities::from_bits(0))
    }

    /// The role.
    pub const fn role(&self) -> Role {
        self.role
    }

    /// Client: the Capabilities that the Invitation carries. Host: the Capabilities of the
    /// received Invitation.
    pub const fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// The identity of the peer, after the first Command that carries it.
    pub const fn peer(&self) -> Option<&PeerInfo> {
        self.peer.as_ref()
    }

    /// Client: the Command to repeat at each repeat interval, if any.
    pub const fn repeating(&self) -> Option<Reply> {
        self.repeating
    }

    /// Host: the number of failed authentications of this entry.
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// Host: the time at which a delayed reply becomes due, if one waits
    /// (REQ-NET-AUTH-047).
    pub fn delayed_due(&self) -> Option<u64> {
        #[cfg(feature = "auth")]
        {
            self.auth.delayed.map(|d| d.due)
        }
        #[cfg(not(feature = "auth"))]
        {
            None
        }
    }

    /// The method of the authentication that this entry asked for (Host) or answered
    /// (Client), if any.
    pub fn method(&self) -> Option<Method> {
        #[cfg(feature = "auth")]
        {
            self.auth.accepted.or(self.auth.asked)
        }
        #[cfg(not(feature = "auth"))]
        {
            None
        }
    }

    /// Deletes the data of the entry: CryptoNonce, digest, Username, delayed reply, timeouts
    /// and repeats (REQ-NET-AUTH-017, -033). Call it when the session goes to Idle. The role
    /// and the Client Capabilities stay.
    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    pub fn clear(&mut self) {
        let caps = if self.role == Role::Client {
            self.caps
        } else {
            Capabilities::default()
        };
        *self = Handshake::new(self.role, caps);
    }

    fn stop_timers(&mut self) {
        self.invitation_deadline = Deadline::NONE;
        self.pending_deadline = Deadline::NONE;
        self.pending_seen = false;
        self.answer_deadline = Deadline::NONE;
        self.repeating = None;
    }

    /// The time of the next deadline in `state`: the Invitation or Pending timeout (Client),
    /// the Host answer timeout or a delayed reply (Host). Call [`Handshake::poll`] at or after
    /// it.
    pub fn next_deadline(&self, state: SessionState) -> Option<u64> {
        if !matches!(
            state,
            SessionState::PendingInvitation | SessionState::AuthenticationRequired
        ) {
            return None;
        }
        match self.role {
            Role::Client => self.client_deadline(),
            Role::Host => {
                let due = if state == SessionState::AuthenticationRequired {
                    self.delayed_due()
                } else {
                    None
                };
                match (due, self.answer_deadline.get()) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            }
        }
    }

    fn client_deadline(&self) -> Option<u64> {
        self.pending_deadline
            .get()
            .or(self.invitation_deadline.get())
    }

    /// Writes an authentication Reply with the kept data.
    #[cfg(feature = "auth")]
    fn write_auth(
        &self,
        reply: Reply,
        identity: &Identity,
        w: &mut PayloadWriter<'_>,
    ) -> Result<(), HandshakeError> {
        let name = &identity.name;
        let id = &identity.product_instance_id;
        match reply {
            Reply::AuthRequired { state } => {
                let nonce = self.auth.nonce.ok_or(HandshakeError::NothingToWrite)?;
                AuthRequired::write(w, AuthState::from_bits(state), &nonce, name, id)
                    .map_err(from_auth)
            }
            Reply::UserAuthRequired { state } => {
                let nonce = self.auth.nonce.ok_or(HandshakeError::NothingToWrite)?;
                UserAuthRequired::write(w, UserAuthState::from_bits(state), &nonce, name, id)
                    .map_err(from_auth)
            }
            Reply::InvitationWithAuth => {
                let d = self.auth.digest.ok_or(HandshakeError::NothingToWrite)?;
                InvitationWithAuth::write(w, &d).map_err(from_auth)
            }
            Reply::InvitationWithUserAuth => {
                let d = self.auth.digest.ok_or(HandshakeError::NothingToWrite)?;
                let user = self
                    .auth
                    .username
                    .get(..self.auth.username_len)
                    .unwrap_or(&[]);
                InvitationWithUserAuth::write(w, &d, user).map_err(from_auth)
            }
            _ => Err(HandshakeError::NothingToWrite),
        }
    }

    #[cfg(not(feature = "auth"))]
    fn write_auth(
        &self,
        _reply: Reply,
        _identity: &Identity,
        _w: &mut PayloadWriter<'_>,
    ) -> Result<(), HandshakeError> {
        Err(HandshakeError::AuthNotAvailable)
    }

    /// Host: a Command from the Client of this entry starts the Host answer timeout again
    /// (REQ-NET-SES-002). [`Handshake::on_command`] does it for the invitation Commands; the
    /// Session calls it for the other Commands of the Client.
    ///
    /// `cfg` is the configuration of this entry (OPT-009: the Handshake does not keep it).
    pub fn touch(&mut self, cfg: &HandshakeConfig, now: u64) {
        Hs {
            hs: self,
            config: cfg,
        }
        .touch(now)
    }

    /// Client: starts the Invitation. The Outcome writes Invitation, goes to Pending
    /// Invitation and starts the repeat (REQ-NET-INV-009) and the Invitation timeout
    /// (REQ-NET-INV-011). Host: an empty Outcome.
    ///
    /// `cfg` is the configuration of this entry (OPT-009: the Handshake does not keep it).
    pub fn start(&mut self, cfg: &HandshakeConfig, now: u64) -> Outcome {
        Hs {
            hs: self,
            config: cfg,
        }
        .start(now)
    }

    /// Handles one received invitation-family Command (0x01, 0x02, 0x03, 0x10, 0x11, 0x12,
    /// 0x13) from the peer of this entry, in the session state `state`. `header` is the header
    /// word of the received Command (for a NAK). Other Commands give an empty Outcome.
    ///
    /// For an address with no entry, use a new Handshake of the role and `state` = Idle.
    ///
    /// `cfg` is the configuration of this entry (OPT-009: the Handshake does not keep it).
    pub fn on_command<C: Credentials + ?Sized>(
        &mut self,
        cfg: &HandshakeConfig,
        now: u64,
        state: SessionState,
        header: Header,
        cmd: &Command<'_>,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        Hs {
            hs: self,
            config: cfg,
        }
        .on_command(now, state, header, cmd, env)
    }

    /// Host: the caller accepts the Client of an entry in Pending Invitation
    /// (REQ-NET-INV-026, -020): Invitation Reply: Accepted and Established Session. Other
    /// states or role: an empty Outcome.
    ///
    /// `cfg` is the configuration of this entry (OPT-009: the Handshake does not keep it).
    pub fn accept(&mut self, cfg: &HandshakeConfig, now: u64, state: SessionState) -> Outcome {
        Hs {
            hs: self,
            config: cfg,
        }
        .accept(now, state)
    }

    /// Ends the Invitation of an entry in Pending Invitation or Authentication Required
    /// (REQ-NET-INV-010). Client: Bye 0x80 (`reason` must be `None` or 0x80). Host: Bye with
    /// `reason`, default 0x42 (N-56); this is also the reject of the caller after
    /// [`HandshakeEvent::InvitationReceived`]. The entry goes to Pending Bye; the Session
    /// repeats the Bye. In other states: an empty Outcome.
    ///
    /// `cfg` is the configuration of this entry (OPT-009: the Handshake does not keep it).
    pub fn cancel(
        &mut self,
        cfg: &HandshakeConfig,
        now: u64,
        state: SessionState,
        reason: Option<ByeReason>,
    ) -> Result<Outcome, HandshakeError> {
        Hs {
            hs: self,
            config: cfg,
        }
        .cancel(now, state, reason)
    }

    /// Host: the same as [`Handshake::cancel`] for the reject of the caller
    /// (REQ-NET-INV-026); `None` gives Bye 0x42.
    ///
    /// `cfg` is the configuration of this entry (OPT-009: the Handshake does not keep it).
    pub fn reject(
        &mut self,
        cfg: &HandshakeConfig,
        now: u64,
        state: SessionState,
        reason: Option<ByeReason>,
    ) -> Result<Outcome, HandshakeError> {
        Hs {
            hs: self,
            config: cfg,
        }
        .reject(now, state, reason)
    }

    /// Client: the caller answers a challenge later (after [`Credentials::answer_now`] gave
    /// false), in Authentication Required: the Outcome sends the Invitation with
    /// Authentication or with User Authentication, or Bye 0x80 when the caller declines
    /// (REQ-NET-AUTH-034, -035). Other cases: an empty Outcome.
    ///
    /// `cfg` is the configuration of this entry (OPT-009: the Handshake does not keep it).
    pub fn answer<C: Credentials + ?Sized>(
        &mut self,
        cfg: &HandshakeConfig,
        now: u64,
        state: SessionState,
        creds: &mut C,
    ) -> Outcome {
        Hs {
            hs: self,
            config: cfg,
        }
        .answer(now, state, creds)
    }

    /// Handles the timeouts at `now` in `state`:
    /// - Client, Invitation or Pending timeout: Bye 0x80, Pending Bye (REQ-NET-INV-011,
    ///   REQ-NET-AUTH-038).
    /// - Host, delayed reply due: that reply (REQ-NET-AUTH-047).
    /// - Host, answer timeout: Idle with no Command (REQ-NET-SES-002).
    ///
    /// `cfg` is the configuration of this entry (OPT-009: the Handshake does not keep it).
    pub fn poll(&mut self, cfg: &HandshakeConfig, now: u64, state: SessionState) -> Outcome {
        Hs {
            hs: self,
            config: cfg,
        }
        .poll(now, state)
    }
}

/// A [`Handshake`] with the configuration of its entry, for the steps that read the
/// configuration. The Handshake does not keep a copy of it (OPT-009); the Session gives its own.
struct Hs<'a, const U: usize> {
    hs: &'a mut Handshake<U>,
    config: &'a HandshakeConfig,
}

impl<const U: usize> core::ops::Deref for Hs<'_, U> {
    type Target = Handshake<U>;
    fn deref(&self) -> &Handshake<U> {
        self.hs
    }
}

impl<const U: usize> core::ops::DerefMut for Hs<'_, U> {
    fn deref_mut(&mut self) -> &mut Handshake<U> {
        self.hs
    }
}

impl<const U: usize> Hs<'_, U> {
    /// Host: a Command from the Client of this entry starts the Host answer timeout again
    /// (REQ-NET-SES-002). [`Handshake::on_command`] does it for the invitation Commands; the
    /// Session calls it for the other Commands of the Client.
    fn touch(&mut self, now: u64) {
        if self.role == Role::Host && self.answer_deadline.get().is_some() {
            self.answer_deadline =
                Deadline::at(now.saturating_add(self.config.host_answer_timeout_ms));
        }
    }

    fn invitation_deadline_from(&self, now: u64) -> Option<u64> {
        self.config
            .invitation_timeout_ms
            .map(|t| now.saturating_add(t))
    }

    /// Client: starts the Invitation. The Outcome writes Invitation, goes to Pending
    /// Invitation and starts the repeat (REQ-NET-INV-009) and the Invitation timeout
    /// (REQ-NET-INV-011). Host: an empty Outcome.
    fn start(&mut self, now: u64) -> Outcome {
        if self.role != Role::Client {
            return Outcome::default();
        }
        self.clear();
        self.repeating = Some(Reply::Invitation);
        self.invitation_deadline = Deadline::from(self.invitation_deadline_from(now));
        Outcome::reply(Reply::Invitation)
            .next(SessionState::PendingInvitation)
            .repeat(Repeat::Start(Reply::Invitation))
    }

    /// Handles one received invitation-family Command (0x01, 0x02, 0x03, 0x10, 0x11, 0x12,
    /// 0x13) from the peer of this entry, in the session state `state`. `header` is the header
    /// word of the received Command (for a NAK). Other Commands give an empty Outcome.
    ///
    /// For an address with no entry, use a new Handshake of the role and `state` = Idle.
    fn on_command<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        state: SessionState,
        header: Header,
        cmd: &Command<'_>,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        match self.role {
            Role::Host => self.host_command(now, state, header, cmd, env),
            Role::Client => self.client_command(now, state, header, cmd, env),
        }
    }

    /// Host: the caller accepts the Client of an entry in Pending Invitation
    /// (REQ-NET-INV-026, -020): Invitation Reply: Accepted and Established Session. Other
    /// states or role: an empty Outcome.
    fn accept(&mut self, now: u64, state: SessionState) -> Outcome {
        let _ = now;
        if self.role != Role::Host || state != SessionState::PendingInvitation {
            return Outcome::default();
        }
        self.establish(None)
    }

    /// Ends the Invitation of an entry in Pending Invitation or Authentication Required
    /// (REQ-NET-INV-010). Client: Bye 0x80 (`reason` must be `None` or 0x80). Host: Bye with
    /// `reason`, default 0x42 (N-56); this is also the reject of the caller after
    /// [`HandshakeEvent::InvitationReceived`]. The entry goes to Pending Bye; the Session
    /// repeats the Bye. In other states: an empty Outcome.
    fn cancel(
        &mut self,
        now: u64,
        state: SessionState,
        reason: Option<ByeReason>,
    ) -> Result<Outcome, HandshakeError> {
        let _ = now;
        let reason = match (self.role, reason) {
            (Role::Client, None) => ByeReason::INVITATION_CANCELED,
            (Role::Host, None) => ByeReason::USER_DID_NOT_ACCEPT,
            (_, Some(r)) => r,
        };
        let allowed = match self.role {
            Role::Client => reason.0 == ByeReason::INVITATION_CANCELED.0,
            Role::Host => reason.is_defined() && reason.allowed_for(Role::Host),
        };
        if !allowed {
            return Err(HandshakeError::ReasonNotAllowed);
        }
        if !matches!(
            state,
            SessionState::PendingInvitation | SessionState::AuthenticationRequired
        ) {
            return Ok(Outcome::default());
        }
        Ok(self.end_with_bye(reason))
    }

    /// Host: the same as [`Handshake::cancel`] for the reject of the caller
    /// (REQ-NET-INV-026); `None` gives Bye 0x42.
    fn reject(
        &mut self,
        now: u64,
        state: SessionState,
        reason: Option<ByeReason>,
    ) -> Result<Outcome, HandshakeError> {
        if self.role != Role::Host {
            return Ok(Outcome::default());
        }
        self.cancel(now, state, reason)
    }

    /// Client: the caller answers a challenge later (after [`Credentials::answer_now`] gave
    /// false), in Authentication Required: the Outcome sends the Invitation with
    /// Authentication or with User Authentication, or Bye 0x80 when the caller declines
    /// (REQ-NET-AUTH-034, -035). Other cases: an empty Outcome.
    fn answer<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        state: SessionState,
        creds: &mut C,
    ) -> Outcome {
        #[cfg(feature = "auth")]
        {
            if self.role == Role::Client
                && state == SessionState::AuthenticationRequired
                && self.auth.waiting
            {
                return self.client_answer(now, creds, Outcome::default());
            }
            Outcome::default()
        }
        #[cfg(not(feature = "auth"))]
        {
            let _ = (now, state, creds);
            Outcome::default()
        }
    }

    /// Handles the timeouts at `now` in `state`:
    /// - Client, Invitation or Pending timeout: Bye 0x80, Pending Bye (REQ-NET-INV-011,
    ///   REQ-NET-AUTH-038).
    /// - Host, delayed reply due: that reply (REQ-NET-AUTH-047).
    /// - Host, answer timeout: Idle with no Command (REQ-NET-SES-002).
    fn poll(&mut self, now: u64, state: SessionState) -> Outcome {
        if !matches!(
            state,
            SessionState::PendingInvitation | SessionState::AuthenticationRequired
        ) {
            return Outcome::default();
        }
        match self.role {
            Role::Client => match self.client_deadline() {
                Some(t) if now >= t => self.end_with_bye(ByeReason::INVITATION_CANCELED),
                _ => Outcome::default(),
            },
            Role::Host => {
                #[cfg(feature = "auth")]
                if state == SessionState::AuthenticationRequired {
                    if let Some(d) = self.auth.delayed {
                        if now >= d.due {
                            self.auth.delayed = None;
                            let mut o = Outcome::reply(d.reply);
                            o.next = d.next;
                            if let Reply::Bye(r) = d.reply {
                                o = o.event(HandshakeEvent::Ended(r));
                            }
                            if d.next == Some(SessionState::Idle) {
                                self.clear();
                            }
                            return o;
                        }
                    }
                }
                match self.answer_deadline.get() {
                    Some(t) if now >= t => {
                        self.clear();
                        Outcome::default()
                            .next(SessionState::Idle)
                            .event(HandshakeEvent::Expired)
                    }
                    _ => Outcome::default(),
                }
            }
        }
    }

    /// Bye of this side, Pending Bye, no more repeats or timeouts.
    fn end_with_bye(&mut self, reason: ByeReason) -> Outcome {
        self.stop_timers();
        #[cfg(feature = "auth")]
        {
            self.auth.delayed = None;
            self.auth.waiting = false;
        }
        Outcome::reply(Reply::Bye(reason))
            .next(SessionState::PendingBye)
            .repeat(Repeat::Stop)
            .event(HandshakeEvent::Ended(reason))
    }

    /// Host: Accepted and Established Session.
    fn establish(&mut self, method: Option<Method>) -> Outcome {
        self.stop_timers();
        let peer = self.peer.unwrap_or(PeerInfo::new(
            NetStr::from_field(&[]),
            NetStr::from_field(&[]),
        ));
        Outcome::reply(Reply::Accepted)
            .next(SessionState::EstablishedSession)
            .event(HandshakeEvent::Established { peer, method })
    }

    // ----- Host -----

    fn host_command<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        state: SessionState,
        header: Header,
        cmd: &Command<'_>,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        match cmd {
            Command::Invitation(inv) => {
                self.touch(now);
                self.host_invitation(now, state, header, inv, env)
            }
            // REQ-NET-INV-030, REQ-NET-SES-004: a Host gets no Invitation Reply.
            Command::InvitationReply(_) => Outcome::not_expected(header),
            Command::Auth(raw) => match CommandCode::from_bits(raw.header.code) {
                CommandCode::InvitationWithAuth | CommandCode::InvitationWithUserAuth => {
                    self.touch(now);
                    self.host_auth_command(now, state, header, raw, env)
                }
                // REQ-NET-AUTH-030.
                _ => Outcome::not_expected(header),
            },
            _ => Outcome::default(),
        }
    }

    fn host_invitation<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        state: SessionState,
        header: Header,
        inv: &Invitation<'_>,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        match state {
            SessionState::Idle => self.host_new_invitation(now, inv, env),
            // REQ-NET-INV-028: Pending again, no new event.
            SessionState::PendingInvitation => Outcome::reply(Reply::Pending),
            // REQ-NET-AUTH-019: the same reply with the same CryptoNonce and state 0x00.
            SessionState::AuthenticationRequired => match self.method() {
                Some(Method::SharedSecret) => Outcome::reply(Reply::AuthRequired { state: 0 }),
                Some(Method::User) => Outcome::reply(Reply::UserAuthRequired { state: 0 }),
                None => Outcome::not_expected(header),
            },
            // REQ-NET-INV-018: Accepted, no change.
            SessionState::EstablishedSession | SessionState::PendingSessionReset => {
                Outcome::reply(Reply::Accepted)
            }
            // REQ-NET-SES-004: no action in Pending Bye.
            SessionState::PendingBye => Outcome::not_expected(header),
        }
    }

    fn host_new_invitation<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        inv: &Invitation<'_>,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        self.clear();
        // REQ-NET-INV-007: empty name or id.
        if inv.name.is_empty() || inv.product_instance_id.is_empty() {
            return Outcome::reply(Reply::Bye(ByeReason::PROTOCOL_ERROR));
        }
        let peer = PeerInfo::new(inv.name, inv.product_instance_id);
        self.peer = Some(peer);
        self.caps = inv.capabilities;
        let policy = env.policy;
        match policy.kind {
            PolicyKind::Accept => self.establish(None),
            PolicyKind::AskCaller => self.host_pending(now),
            _ => {
                let mut extra = None;
                if let Some(m) = self.host_method(&policy) {
                    match self.host_challenge(now, m, env.creds) {
                        Ok(o) => return o,
                        Err(e) => extra = Some(HandshakeEvent::CredentialsError(e)),
                    }
                }
                let o = if policy.pending_fallback {
                    // REQ-NET-INV-014.
                    self.host_pending(now)
                } else {
                    // REQ-NET-INV-015, -036: Bye 0x45 once, no entry.
                    self.clear();
                    Outcome::reply(Reply::Bye(ByeReason::NO_MATCHING_METHOD))
                };
                match extra {
                    Some(e) => o.event(e),
                    None => o,
                }
            }
        }
    }

    /// The method that the Host asks for, from the policy and the Capabilities
    /// (REQ-NET-AUTH-014, -015, N-44). Never one without the feature `auth`.
    fn host_method(&self, policy: &HostPolicy) -> Option<Method> {
        if !cfg!(feature = "auth") {
            return None;
        }
        let s =
            matches!(policy.kind, PolicyKind::SharedSecret | PolicyKind::Both) && self.caps.auth;
        let u =
            matches!(policy.kind, PolicyKind::UserList | PolicyKind::Both) && self.caps.user_auth;
        match (s, u) {
            (true, true) => Some(policy.preferred),
            (true, false) => Some(Method::SharedSecret),
            (false, true) => Some(Method::User),
            (false, false) => None,
        }
    }

    /// Pending Invitation and the event for the caller (REQ-NET-INV-026).
    fn host_pending(&mut self, now: u64) -> Outcome {
        self.answer_deadline = Deadline::at(now.saturating_add(self.config.host_answer_timeout_ms));
        let peer = self.peer.unwrap_or(PeerInfo::new(
            NetStr::from_field(&[]),
            NetStr::from_field(&[]),
        ));
        Outcome::reply(Reply::Pending)
            .next(SessionState::PendingInvitation)
            .event(HandshakeEvent::InvitationReceived {
                peer,
                capabilities: self.caps,
            })
    }

    /// Authentication Required with a new CryptoNonce and state 0x00 (REQ-NET-AUTH-014 to
    /// -017, -020).
    #[cfg(feature = "auth")]
    fn host_challenge<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        method: Method,
        creds: &mut C,
    ) -> Result<Outcome, CredentialsError> {
        let random = creds.random_nonce().ok_or(CredentialsError::NoRandom)?;
        self.auth.nonce = Some(CryptoNonce::from_random(random));
        self.auth.asked = Some(method);
        self.failures = 0;
        self.answer_deadline = Deadline::at(now.saturating_add(self.config.host_answer_timeout_ms));
        let reply = match method {
            Method::SharedSecret => Reply::AuthRequired { state: 0 },
            Method::User => Reply::UserAuthRequired { state: 0 },
        };
        Ok(Outcome::reply(reply).next(SessionState::AuthenticationRequired))
    }

    #[cfg(not(feature = "auth"))]
    fn host_challenge<C: Credentials + ?Sized>(
        &mut self,
        _now: u64,
        _method: Method,
        _creds: &mut C,
    ) -> Result<Outcome, CredentialsError> {
        Err(CredentialsError::NoRandom)
    }

    /// Invitation with Authentication or with User Authentication at the Host
    /// (REQ-NET-AUTH-027, and with the feature -021 to -025, -028, -029, -046, -047).
    fn host_auth_command<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        state: SessionState,
        header: Header,
        raw: &crate::packet::RawCommand<'_>,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        match state {
            SessionState::Idle => Outcome::reply(Reply::Bye(ByeReason::AUTH_INVITATION_REJECTED)),
            #[cfg(feature = "auth")]
            SessionState::AuthenticationRequired => self.host_verify(now, header, raw, env),
            #[cfg(feature = "auth")]
            SessionState::EstablishedSession => self.host_reverify(header, raw),
            _ => {
                let _ = (now, raw, &env);
                Outcome::not_expected(header)
            }
        }
    }

    #[cfg(feature = "auth")]
    fn host_verify<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        header: Header,
        raw: &RawCommand<'_>,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        // REQ-NET-AUTH-047: ignore while a delayed reply is due.
        if self.auth.delayed.is_some() {
            return Outcome::default();
        }
        let (Some(asked), Some(nonce)) = (self.auth.asked, self.auth.nonce) else {
            return Outcome::not_expected(header);
        };
        let malformed = Outcome::nak(NakReason::COMMAND_MALFORMED, header);
        let user = header.code == CommandCode::InvitationWithUserAuth.into_bits();
        let mut events = Events::default();
        // Ok(digest): pass; Err(not_found): fail.
        let verdict: Result<AuthDigest, bool> = if user {
            let Ok(c) = InvitationWithUserAuth::read(raw) else {
                return malformed;
            };
            if asked != Method::User {
                // REQ-NET-AUTH-028.
                Err(false)
            } else {
                let name = c.username.as_bytes();
                match env.creds.password(name) {
                    // REQ-NET-AUTH-025.
                    None => Err(true),
                    Some(pw) => match auth::verify_user(&nonce, name, pw, &c.digest) {
                        Ok(true) => Ok(c.digest),
                        Ok(false) => Err(false),
                        Err(_) => {
                            events.push(HandshakeEvent::CredentialsError(
                                CredentialsError::EmptyPassword,
                            ));
                            Err(false)
                        }
                    },
                }
            }
        } else {
            let Ok(c) = InvitationWithAuth::read(raw) else {
                return malformed;
            };
            if asked != Method::SharedSecret {
                // REQ-NET-AUTH-028.
                Err(false)
            } else {
                match env.creds.shared_secret() {
                    None => {
                        events.push(HandshakeEvent::CredentialsError(CredentialsError::NoSecret));
                        Err(false)
                    }
                    Some(s) => match auth::verify_shared_secret(&nonce, s, &c.digest) {
                        Ok(true) => Ok(c.digest),
                        Ok(false) => Err(false),
                        Err(_) => {
                            events.push(HandshakeEvent::CredentialsError(
                                CredentialsError::EmptySecret,
                            ));
                            Err(false)
                        }
                    },
                }
            }
        };
        let mut o = match verdict {
            // REQ-NET-AUTH-021, -024.
            Ok(digest) => {
                if let Some(d) = env.delay.as_deref_mut() {
                    d.succeed();
                }
                self.auth.digest = Some(digest);
                self.auth.accepted = Some(asked);
                self.establish(Some(asked))
            }
            Err(not_found) => self.host_fail(now, asked, not_found, env),
        };
        for e in events.iter() {
            o.events.push(*e);
        }
        o
    }

    /// A failed authentication: re-challenge or Bye at the limit (REQ-NET-AUTH-022, -025,
    /// -046), delayed (REQ-NET-AUTH-026, -047).
    #[cfg(feature = "auth")]
    fn host_fail<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        method: Method,
        not_found: bool,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        self.failures = self.failures.saturating_add(1);
        let limit = self.config.failure_limit.max(1);
        let (reply, next) = if self.failures >= limit {
            let r = if not_found {
                ByeReason::USERNAME_NOT_FOUND
            } else {
                ByeReason::AUTHENTICATION_FAILED
            };
            (Reply::Bye(r), Some(SessionState::Idle))
        } else {
            let r = match (method, not_found) {
                (Method::SharedSecret, _) => Reply::AuthRequired { state: 0x01 },
                (Method::User, false) => Reply::UserAuthRequired { state: 0x01 },
                (Method::User, true) => Reply::UserAuthRequired { state: 0x02 },
            };
            (r, None)
        };
        let failed = HandshakeEvent::AuthFailed {
            method,
            failures: self.failures,
            username_not_found: not_found,
        };
        let due = env.delay.as_deref_mut().map_or(now, |d| d.fail(now));
        if due > now {
            self.auth.delayed = Some(Delayed { reply, next, due });
            return Outcome::default().event(failed);
        }
        let mut o = Outcome::reply(reply).event(failed);
        o.next = next;
        if let Reply::Bye(r) = reply {
            o = o.event(HandshakeEvent::Ended(r));
            self.clear();
        }
        o
    }

    /// REQ-NET-AUTH-029: a repeated accepted Command in Established Session.
    #[cfg(feature = "auth")]
    fn host_reverify(&mut self, header: Header, raw: &RawCommand<'_>) -> Outcome {
        let user = header.code == CommandCode::InvitationWithUserAuth.into_bits();
        let received = if user {
            InvitationWithUserAuth::read(raw).map(|c| c.digest)
        } else {
            InvitationWithAuth::read(raw).map(|c| c.digest)
        };
        let Ok(received) = received else {
            return Outcome::nak(NakReason::COMMAND_MALFORMED, header);
        };
        let method = if user {
            Method::User
        } else {
            Method::SharedSecret
        };
        match (self.auth.accepted, self.auth.digest) {
            (Some(m), Some(d)) if m == method && auth::verify(&d, &received) => {
                Outcome::reply(Reply::Accepted)
            }
            _ => Outcome::not_expected(header),
        }
    }

    // ----- Client -----

    fn client_command<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        state: SessionState,
        header: Header,
        cmd: &Command<'_>,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        match cmd {
            // REQ-NET-INV-019.
            Command::Invitation(_) => Outcome::not_expected(header),
            Command::InvitationReply(r) if r.pending => self.client_pending(now, state, r),
            Command::InvitationReply(r) => self.client_accepted(state, r),
            Command::Auth(raw) => match CommandCode::from_bits(raw.header.code) {
                CommandCode::InvitationAuthRequired => {
                    self.client_challenge(now, state, header, raw, Method::SharedSecret, env)
                }
                CommandCode::InvitationUserAuthRequired => {
                    self.client_challenge(now, state, header, raw, Method::User, env)
                }
                // 0x02 and 0x03 go from Client to Host only (REQ-NET-SES-004).
                _ => Outcome::not_expected(header),
            },
            _ => Outcome::default(),
        }
    }

    /// Invitation Reply: Accepted at the Client (REQ-NET-INV-020, -022, -023).
    fn client_accepted(&mut self, state: SessionState, r: &InvitationReply<'_>) -> Outcome {
        match state {
            SessionState::Idle => Outcome::reply(Reply::Bye(ByeReason::NO_PENDING_SESSION)),
            SessionState::PendingInvitation => {
                let peer = PeerInfo::new(r.name, r.product_instance_id);
                self.peer = Some(peer);
                self.stop_timers();
                #[cfg(feature = "auth")]
                let method = self.auth.answered.map(|(m, _, _)| m);
                #[cfg(not(feature = "auth"))]
                let method = None;
                #[cfg(feature = "auth")]
                {
                    self.auth.accepted = method;
                }
                Outcome::default()
                    .next(SessionState::EstablishedSession)
                    .repeat(Repeat::Stop)
                    .event(HandshakeEvent::Established { peer, method })
            }
            SessionState::AuthenticationRequired => {
                self.end_with_bye(ByeReason::NO_PENDING_SESSION)
            }
            SessionState::EstablishedSession
            | SessionState::PendingSessionReset
            | SessionState::PendingBye => Outcome::default(),
        }
    }

    /// Invitation Reply: Pending at the Client (REQ-NET-INV-027, -029).
    fn client_pending(
        &mut self,
        now: u64,
        state: SessionState,
        r: &InvitationReply<'_>,
    ) -> Outcome {
        match state {
            SessionState::Idle => Outcome::reply(Reply::Bye(ByeReason::NO_PENDING_SESSION)),
            SessionState::PendingInvitation => {
                let peer = PeerInfo::new(r.name, r.product_instance_id);
                self.peer = Some(peer);
                if self.pending_seen {
                    return Outcome::default();
                }
                self.pending_seen = true;
                self.invitation_deadline = Deadline::NONE;
                self.pending_deadline =
                    Deadline::at(now.saturating_add(self.config.pending_timeout_ms));
                Outcome::default().event(HandshakeEvent::Pending { peer })
            }
            SessionState::AuthenticationRequired => {
                self.end_with_bye(ByeReason::NO_PENDING_SESSION)
            }
            SessionState::EstablishedSession
            | SessionState::PendingSessionReset
            | SessionState::PendingBye => Outcome::default(),
        }
    }

    /// Invitation Reply: (User) Authentication Required at the Client (REQ-NET-AUTH-031,
    /// -032, and with the feature -020, -033 to -038).
    fn client_challenge<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        state: SessionState,
        header: Header,
        raw: &crate::packet::RawCommand<'_>,
        method: Method,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        if !matches!(
            state,
            SessionState::PendingInvitation | SessionState::AuthenticationRequired
        ) {
            return Outcome::not_expected(header);
        }
        let enabled = match method {
            Method::SharedSecret => self.caps.auth,
            Method::User => self.caps.user_auth,
        };
        if !enabled {
            return Outcome::not_expected(header);
        }
        #[cfg(feature = "auth")]
        {
            self.client_accept_challenge(now, state, header, raw, method, env)
        }
        #[cfg(not(feature = "auth"))]
        {
            let _ = (now, raw, env);
            Outcome::not_expected(header)
        }
    }

    #[cfg(feature = "auth")]
    fn client_accept_challenge<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        state: SessionState,
        header: Header,
        raw: &RawCommand<'_>,
        method: Method,
        env: &mut Env<'_, C>,
    ) -> Outcome {
        let read = match method {
            Method::SharedSecret => AuthRequired::read(raw).map(|c| {
                let first = c.state.is_first_request();
                (
                    c.state.into_bits(),
                    first,
                    c.nonce,
                    c.name,
                    c.product_instance_id,
                )
            }),
            Method::User => UserAuthRequired::read(raw).map(|c| {
                let first = c.state.is_first_request();
                (
                    c.state.into_bits(),
                    first,
                    c.nonce,
                    c.name,
                    c.product_instance_id,
                )
            }),
        };
        let Ok((st, first, nonce, name, id)) = read else {
            return Outcome::nak(NakReason::COMMAND_MALFORMED, header);
        };
        let peer = PeerInfo::new(name, id);
        self.peer = Some(peer);
        // REQ-NET-AUTH-003, -004: a reserved state is processed as 0x00, so it has the key of
        // 0x00; the caller gets the raw value.
        let key = (method, nonce, if first { 0x00 } else { st });
        // A duplicate first request that the kept digest answers already (a reply to a
        // repeated Invitation): no new prompt, the repeats go on.
        if state == SessionState::PendingInvitation
            && first
            && self.auth.answered == Some(key)
            && self.auth.digest.is_some()
        {
            return Outcome::default();
        }
        // REQ-NET-AUTH-033: the nonce of the last accepted challenge.
        self.auth.nonce = Some(nonce);
        self.auth.asked = Some(method);
        self.auth.state = st;
        self.auth.digest = None;
        self.auth.username_len = 0;
        self.repeating = None;
        if self.config.restart_timeout_on_challenge && self.auth.answered != Some(key) {
            // N-55: the wait for the caller also has a timeout.
            self.pending_deadline = Deadline::NONE;
            self.pending_seen = false;
            self.invitation_deadline = Deadline::from(self.invitation_deadline_from(now));
        }
        let challenge = Challenge { method, state: st };
        // REQ-NET-AUTH-020: Authentication Required, the Invitation repeat stops.
        let o = Outcome::default()
            .next(SessionState::AuthenticationRequired)
            .repeat(Repeat::Stop)
            .event(HandshakeEvent::AuthRequested { peer, challenge });
        if !env.creds.answer_now(challenge) {
            self.auth.waiting = true;
            return o;
        }
        self.client_answer(now, env.creds, o)
    }

    /// Sends the authentication Command, or Bye 0x80 when the caller declines
    /// (REQ-NET-AUTH-034 to -037, N-54).
    #[cfg(feature = "auth")]
    fn client_answer<C: Credentials + ?Sized>(
        &mut self,
        now: u64,
        creds: &mut C,
        mut o: Outcome,
    ) -> Outcome {
        self.auth.waiting = false;
        let (Some(method), Some(nonce)) = (self.auth.asked, self.auth.nonce) else {
            return o;
        };
        let result: Result<(AuthDigest, Reply), Option<CredentialsError>> = match method {
            Method::SharedSecret => match creds.shared_secret() {
                None => Err(None),
                Some(s) => auth::shared_secret_digest(&nonce, s)
                    .map(|d| (d, Reply::InvitationWithAuth))
                    .map_err(|_| Some(CredentialsError::EmptySecret)),
            },
            Method::User => match creds.user() {
                None => Err(None),
                Some((user, pw)) => match check_username(user, U) {
                    Err(e) => Err(Some(e)),
                    Ok(()) if pw.is_empty() => Err(Some(CredentialsError::EmptyPassword)),
                    Ok(()) => match auth::user_digest(&nonce, user, pw) {
                        Ok(d) => {
                            self.auth.username[..user.len()].copy_from_slice(user);
                            self.auth.username_len = user.len();
                            Ok((d, Reply::InvitationWithUserAuth))
                        }
                        Err(_) => Err(Some(CredentialsError::InvalidUsername)),
                    },
                },
            },
        };
        match result {
            Ok((digest, reply)) => {
                let key = (method, nonce, self.auth.state);
                self.auth.digest = Some(digest);
                if self.config.restart_timeout_on_challenge && self.auth.answered != Some(key) {
                    // REQ-NET-AUTH-038, N-55.
                    self.pending_deadline = Deadline::NONE;
                    self.pending_seen = false;
                    self.invitation_deadline = Deadline::from(self.invitation_deadline_from(now));
                }
                self.pending_seen = false;
                self.auth.answered = Some(key);
                self.repeating = Some(reply);
                o.reply = Some(reply);
                o.next = Some(SessionState::PendingInvitation);
                o.repeat = Repeat::Start(reply);
                o
            }
            Err(e) => {
                if let Some(e) = e {
                    o.events.push(HandshakeEvent::CredentialsError(e));
                }
                let bye = self.end_with_bye(ByeReason::INVITATION_CANCELED);
                o.reply = bye.reply;
                o.next = bye.next;
                o.repeat = bye.repeat;
                for e in bye.events.iter() {
                    o.events.push(*e);
                }
                o
            }
        }
    }
}

/// The Username rules of the writer (REQ-NET-AUTH-044) and the capacity `cap`.
#[cfg(feature = "auth")]
fn check_username(user: &[u8], cap: usize) -> Result<(), CredentialsError> {
    if user.is_empty() {
        return Err(CredentialsError::InvalidUsername);
    }
    if user.len() > cap || user.len() > MAX_USERNAME {
        return Err(CredentialsError::UsernameTooLong);
    }
    let s = core::str::from_utf8(user).map_err(|_| CredentialsError::InvalidUsername)?;
    if s.contains('\0') || s.starts_with('\u{FEFF}') {
        return Err(CredentialsError::InvalidUsername);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn deadline_is_8_bytes_and_keeps_each_time() {
        // OPT-012: u64::MAX is "none"; every other time is kept.
        assert_eq!(core::mem::size_of::<Deadline>(), 8);
        assert_eq!(Deadline::NONE.get(), None);
        assert_eq!(Deadline::from(None).get(), None);
        for t in [0, 1, 60_000, u64::MAX - 1] {
            assert_eq!(Deadline::at(t).get(), Some(t));
            assert_eq!(Deadline::from(Some(t)).get(), Some(t));
        }
    }
    use crate::packet::{command_packets, MaxPayload, PacketItem};
    use proptest::prelude::*;
    use std::vec::Vec;
    use SessionState::*;

    fn host_id() -> Identity {
        Identity::new("Host", "H1").unwrap()
    }

    fn client_id() -> Identity {
        Identity::new("MyDev", "8shYe3h5").unwrap()
    }

    /// Test credentials.
    #[derive(Default)]
    struct Creds {
        random: Option<[u8; 16]>,
        secret: Option<Vec<u8>>,
        users: Vec<(Vec<u8>, Vec<u8>)>,
        user: Option<(Vec<u8>, Vec<u8>)>,
        later: bool,
        challenges: Vec<Challenge>,
        secret_calls: usize,
    }

    impl Credentials for Creds {
        fn random_nonce(&mut self) -> Option<[u8; 16]> {
            self.random
        }
        fn shared_secret(&mut self) -> Option<&[u8]> {
            self.secret_calls += 1;
            self.secret.as_deref()
        }
        fn password(&mut self, username: &[u8]) -> Option<&[u8]> {
            self.users
                .iter()
                .find(|(u, _)| u == username)
                .map(|(_, p)| p.as_slice())
        }
        fn user(&mut self) -> Option<(&[u8], &[u8])> {
            self.user
                .as_ref()
                .map(|(u, p)| (u.as_slice(), p.as_slice()))
        }
        fn answer_now(&mut self, challenge: Challenge) -> bool {
            self.challenges.push(challenge);
            !self.later
        }
    }

    fn bytes<const U: usize>(r: Reply, hs: &Handshake<U>, id: &Identity) -> Vec<u8> {
        let mut buf = [0u8; 1400];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        r.write(hs, id, &mut w).unwrap();
        let n = w.finish().unwrap();
        buf[..n].to_vec()
    }

    fn parse(p: &[u8]) -> (Header, Command<'_>) {
        let Some(PacketItem::Command(raw)) = command_packets(p).unwrap().next() else {
            panic!("no command")
        };
        (raw.header, Command::parse(raw).unwrap())
    }

    /// A Command Packet from its fields; the payload is filled with 0x00 to a word.
    fn pkt(code: u8, csd1: u8, csd2: u8, payload: &[u8]) -> Vec<u8> {
        let mut p = Vec::from(*b"MIDI");
        let mut body = Vec::from(payload);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        p.extend_from_slice(&[code, (body.len() / 4) as u8, csd1, csd2]);
        p.extend_from_slice(&body);
        p
    }

    /// 0x12 or 0x13 from a Host "Host"/"H1" with a nonce and a state.
    fn challenge_pkt(code: u8, state: u8, nonce: &[u8; 16]) -> Vec<u8> {
        let mut b = Vec::from(&nonce[..]);
        b.extend_from_slice(b"Host");
        b.extend_from_slice(b"H1\0\0");
        pkt(code, 1, state, &b)
    }

    fn events(o: &Outcome) -> Vec<HandshakeEvent> {
        o.events.iter().copied().collect()
    }

    fn bye_reason(p: &[u8]) -> u8 {
        let (h, c) = parse(p);
        assert!(matches!(c, Command::Bye(_)), "{c:?}");
        h.csd1
    }

    /// One side of a test session.
    struct Side {
        hs: Handshake,
        /// The configuration of the entry (OPT-009: the Handshake does not keep it).
        cfg: HandshakeConfig,
        state: SessionState,
        id: Identity,
        creds: Creds,
        delay: FailureDelay,
        use_delay: bool,
        policy: HostPolicy,
    }

    impl Side {
        fn host(policy: HostPolicy) -> Self {
            Side {
                hs: Handshake::host(),
                cfg: HandshakeConfig::DEFAULT,
                state: Idle,
                id: host_id(),
                creds: Creds::default(),
                delay: FailureDelay::new(0, 0),
                use_delay: false,
                policy,
            }
        }

        fn client(caps: Capabilities) -> Self {
            Side {
                hs: Handshake::client(caps),
                cfg: HandshakeConfig::DEFAULT,
                state: Idle,
                id: client_id(),
                creds: Creds::default(),
                delay: FailureDelay::new(0, 0),
                use_delay: false,
                policy: HostPolicy::DEFAULT,
            }
        }

        fn apply(&mut self, o: &Outcome) {
            if let Some(n) = o.next {
                self.state = n;
            }
        }

        fn receive(&mut self, now: u64, p: &[u8]) -> Outcome {
            let (h, c) = parse(p);
            let delay = if self.use_delay {
                Some(&mut self.delay)
            } else {
                None
            };
            let mut env = Env {
                creds: &mut self.creds,
                delay,
                policy: self.policy,
            };
            let o = self
                .hs
                .on_command(&self.cfg, now, self.state, h, &c, &mut env);
            self.apply(&o);
            o
        }

        fn wire(&self, o: &Outcome) -> Vec<u8> {
            bytes(o.reply.expect("a reply"), &self.hs, &self.id)
        }

        fn start(&mut self, now: u64) -> Outcome {
            let o = self.hs.start(&self.cfg, now);
            self.apply(&o);
            o
        }

        fn poll(&mut self, now: u64) -> Outcome {
            let o = self.hs.poll(&self.cfg, now, self.state);
            self.apply(&o);
            o
        }
    }

    /// Writes the reply of `o` from `from` and gives it to `to`.
    fn send(now: u64, from: &Side, o: &Outcome, to: &mut Side) -> Outcome {
        let p = from.wire(o);
        to.receive(now, &p)
    }

    /// The Invitation of A.1 ("MyDev", "8shYe3h5", Capabilities 0).
    const A11: [u8; 24] = [
        0x4D, 0x49, 0x44, 0x49, 0x01, 0x04, 0x02, 0x00, 0x4D, 0x79, 0x44, 0x65, 0x76, 0, 0, 0,
        0x38, 0x73, 0x68, 0x59, 0x65, 0x33, 0x68, 0x35,
    ];

    #[test]
    fn config_defaults_and_checks() {
        // N-22, N-25, N-46.
        let c = HandshakeConfig::default();
        assert_eq!(c.invitation_timeout_ms, Some(10_000));
        assert_eq!(c.pending_timeout_ms, 60_000);
        assert_eq!(c.host_answer_timeout_ms, 60_000);
        assert_eq!(c.failure_limit, 5);
        assert!(c.restart_timeout_on_challenge);
        assert_eq!(c.check(), Ok(()));
        let bad = HandshakeConfig {
            failure_limit: 0,
            ..c
        };
        assert_eq!(bad.check(), Err(HandshakeError::InvalidConfig));
        let p = HostPolicy::default();
        assert_eq!(p.kind, PolicyKind::Accept);
        assert_eq!(p.preferred, Method::User);
        assert!(!p.pending_fallback);
        assert_eq!(p.check(), Ok(()));
        let auth = HostPolicy::new(PolicyKind::SharedSecret).check();
        if cfg!(feature = "auth") {
            assert_eq!(auth, Ok(()));
        } else {
            assert_eq!(auth, Err(HandshakeError::AuthNotAvailable));
        }
    }

    #[test]
    fn accept_policy_establishes_both_sides() {
        // REQ-NET-INV-012, REQ-NET-INV-020.
        let mut c = Side::client(Capabilities::default());
        let mut h = Side::host(HostPolicy::DEFAULT);
        let o = c.start(0);
        assert_eq!(c.state, PendingInvitation);
        assert_eq!(o.repeat, Repeat::Start(Reply::Invitation));
        assert_eq!(c.wire(&o), A11);
        let oh = send(0, &c, &o, &mut h);
        assert_eq!(oh.reply, Some(Reply::Accepted));
        assert_eq!(h.state, EstablishedSession);
        let [HandshakeEvent::Established { peer, method: None }] = events(&oh)[..] else {
            panic!("{oh:?}")
        };
        assert_eq!(peer.name(), b"MyDev");
        assert_eq!(peer.product_instance_id_str(), Ok("8shYe3h5"));
        let oc = send(1, &h, &oh, &mut c);
        assert_eq!(c.state, EstablishedSession);
        assert_eq!(oc.reply, None);
        assert_eq!(oc.repeat, Repeat::Stop);
        assert_eq!(c.hs.repeating(), None);
        let [HandshakeEvent::Established { peer, method: None }] = events(&oc)[..] else {
            panic!("{oc:?}")
        };
        assert_eq!(peer.name_str(), Ok("Host"));
        assert_eq!(peer.product_instance_id(), b"H1");
        assert_eq!(c.hs.next_deadline(c.state), None);
    }

    #[test]
    fn empty_name_or_id_gets_bye_07() {
        // REQ-NET-INV-007, REQ-NET-INV-036.
        let cases = [
            pkt(0x01, 0, 0, b"8shYe3h5"),            // csd1 = 0
            pkt(0x01, 1, 0, b"\0abc8shYe3h5"),       // name starts with 0x00
            pkt(0x01, 2, 0, b"MyDev"),               // pl = csd1
            pkt(0x01, 2, 0, b"MyDev\0\0\0\0id\0\0"), // id starts with 0x00
        ];
        for p in cases {
            let mut h = Side::host(HostPolicy::DEFAULT);
            let o = h.receive(0, &p);
            assert_eq!(o.reply, Some(Reply::Bye(ByeReason::PROTOCOL_ERROR)));
            assert_eq!(o.next, None);
            assert!(o.events.is_empty());
            assert_eq!(h.state, Idle);
            assert_eq!(bye_reason(&h.wire(&o)), 0x07);
        }
    }

    #[test]
    fn client_stays_pending_until_reply_and_repeats() {
        // REQ-NET-INV-008, REQ-NET-INV-009.
        let mut c = Side::client(Capabilities::default());
        let mut h = Side::host(HostPolicy::new(PolicyKind::AskCaller));
        let o = c.start(0);
        let first = c.wire(&o);
        let oh = send(0, &c, &o, &mut h);
        assert_eq!(oh.reply, Some(Reply::Pending));
        let oc = send(0, &h, &oh, &mut c);
        assert_eq!(c.state, PendingInvitation);
        assert_eq!(oc.repeat, Repeat::Keep);
        assert_eq!(c.hs.repeating(), Some(Reply::Invitation));
        // The repeat has the same bytes.
        assert_eq!(bytes(Reply::Invitation, &c.hs, &c.id), first);
        assert!(c.poll(9_999).is_empty());
        assert_eq!(c.state, PendingInvitation);
        // The repeated Invitation gets Pending again; then the caller accepts.
        let oh = h.receive(500, &first);
        assert_eq!(oh.reply, Some(Reply::Pending));
        let oa = h.hs.accept(&h.cfg, 600, h.state);
        h.apply(&oa);
        let oc = send(600, &h, &oa, &mut c);
        assert_eq!(c.state, EstablishedSession);
        assert_eq!(oc.repeat, Repeat::Stop);
    }

    #[test]
    fn cancel_sends_bye_80_or_the_host_reason() {
        // REQ-NET-INV-010.
        let mut c = Side::client(Capabilities::default());
        c.start(0);
        assert_eq!(
            c.hs.cancel(&c.cfg, 1, c.state, Some(ByeReason::USER_DID_NOT_ACCEPT)),
            Err(HandshakeError::ReasonNotAllowed)
        );
        let o = c.hs.cancel(&c.cfg, 1, c.state, None).unwrap();
        c.apply(&o);
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::INVITATION_CANCELED)));
        assert_eq!(c.state, PendingBye);
        assert_eq!(o.repeat, Repeat::Stop);
        assert_eq!(
            events(&o),
            [HandshakeEvent::Ended(ByeReason::INVITATION_CANCELED)]
        );
        assert_eq!(bye_reason(&c.wire(&o)), 0x80);
        assert_eq!(c.hs.next_deadline(c.state), None);
        // Not in Pending Invitation or Authentication Required: nothing.
        assert!(c.hs.cancel(&c.cfg, 2, c.state, None).unwrap().is_empty());

        let mut h = Side::host(HostPolicy::new(PolicyKind::AskCaller));
        h.receive(0, &A11);
        assert_eq!(h.state, PendingInvitation);
        assert_eq!(
            h.hs.cancel(&h.cfg, 1, h.state, Some(ByeReason::INVITATION_CANCELED)),
            Err(HandshakeError::ReasonNotAllowed)
        );
        let o = h.hs.cancel(&h.cfg, 1, h.state, None).unwrap();
        h.apply(&o);
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::USER_DID_NOT_ACCEPT)));
        assert_eq!(h.state, PendingBye);
        assert_eq!(bye_reason(&h.wire(&o)), 0x42);

        let mut h = Side::host(HostPolicy::new(PolicyKind::AskCaller));
        h.receive(0, &A11);
        let o =
            h.hs.cancel(&h.cfg, 1, h.state, Some(ByeReason::TOO_MANY_SESSIONS))
                .unwrap();
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::TOO_MANY_SESSIONS)));
        assert_eq!(o.next, Some(PendingBye));
    }

    #[test]
    fn invitation_timeout_sends_bye_80() {
        // REQ-NET-INV-011.
        let mut c = Side::client(Capabilities::default());
        c.start(1_000);
        assert_eq!(c.hs.next_deadline(c.state), Some(11_000));
        assert!(c.poll(10_999).is_empty());
        let o = c.poll(11_000);
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::INVITATION_CANCELED)));
        assert_eq!(o.next, Some(PendingBye));
        assert_eq!(o.repeat, Repeat::Stop);
        // No timeout: the Client repeats until a reply or a cancel.
        let cfg = HandshakeConfig {
            invitation_timeout_ms: None,
            ..HandshakeConfig::DEFAULT
        };
        let mut hs: Handshake = Handshake::client(Capabilities::default());
        let o = hs.start(&cfg, 0);
        assert_eq!(o.next, Some(PendingInvitation));
        assert_eq!(hs.next_deadline(PendingInvitation), None);
        assert!(hs.poll(&cfg, u64::MAX, PendingInvitation).is_empty());
        assert_eq!(hs.repeating(), Some(Reply::Invitation));
    }

    #[test]
    fn ask_caller_policy_gives_pending_then_accept_or_reject() {
        // REQ-NET-INV-026, REQ-NET-INV-012, REQ-NET-INV-020.
        let mut h = Side::host(HostPolicy::new(PolicyKind::AskCaller));
        let o = h.receive(0, &A11);
        assert_eq!(o.reply, Some(Reply::Pending));
        assert_eq!(h.state, PendingInvitation);
        let [HandshakeEvent::InvitationReceived { peer, capabilities }] = events(&o)[..] else {
            panic!("{o:?}")
        };
        assert_eq!(peer.name(), b"MyDev");
        assert_eq!(capabilities, Capabilities::default());
        // Accept only from Pending Invitation.
        assert!(h.hs.accept(&h.cfg, 1, EstablishedSession).is_empty());
        let o = h.hs.accept(&h.cfg, 1, h.state);
        assert_eq!(o.reply, Some(Reply::Accepted));
        assert_eq!(o.next, Some(EstablishedSession));
        assert!(matches!(
            events(&o)[..],
            [HandshakeEvent::Established { method: None, .. }]
        ));

        let mut h = Side::host(HostPolicy::new(PolicyKind::AskCaller));
        h.receive(0, &A11);
        let o = h.hs.reject(&h.cfg, 1, h.state, None).unwrap();
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::USER_DID_NOT_ACCEPT)));
        assert_eq!(o.next, Some(PendingBye));
    }

    #[test]
    fn pending_fallback_when_no_matching_method() {
        // REQ-NET-INV-014.
        for kind in [
            PolicyKind::SharedSecret,
            PolicyKind::UserList,
            PolicyKind::Both,
        ] {
            let mut policy = HostPolicy::new(kind);
            policy.pending_fallback = true;
            let mut h = Side::host(policy);
            let o = h.receive(0, &A11); // Capabilities 0
            assert_eq!(o.reply, Some(Reply::Pending));
            assert_eq!(h.state, PendingInvitation);
            assert!(matches!(
                events(&o)[..],
                [HandshakeEvent::InvitationReceived { .. }]
            ));
        }
    }

    #[test]
    fn no_matching_method_gets_bye_45_and_no_entry() {
        // REQ-NET-INV-015, REQ-NET-INV-036.
        let mut h = Side::host(HostPolicy::new(PolicyKind::UserList));
        for _ in 0..2 {
            let o = h.receive(0, &A11);
            assert_eq!(o.reply, Some(Reply::Bye(ByeReason::NO_MATCHING_METHOD)));
            assert_eq!(o.next, None);
            assert_eq!(h.state, Idle);
            assert_eq!(h.hs.next_deadline(Idle), None);
            assert_eq!(bye_reason(&h.wire(&o)), 0x45);
        }
    }

    #[test]
    fn invitation_in_established_session_gets_accepted() {
        // REQ-NET-INV-018.
        for s in [EstablishedSession, PendingSessionReset] {
            let mut h = Side::host(HostPolicy::new(PolicyKind::AskCaller));
            h.state = s;
            let o = h.receive(0, &A11);
            assert_eq!(o.reply, Some(Reply::Accepted));
            assert_eq!(o.next, None);
            assert!(o.events.is_empty());
            assert_eq!(h.state, s);
        }
        let mut h = Side::host(HostPolicy::DEFAULT);
        h.state = PendingBye;
        let o = h.receive(0, &A11);
        let (hdr, _) = parse(&A11);
        assert_eq!(
            o.reply,
            Some(Reply::Nak(NakReason::COMMAND_NOT_EXPECTED, hdr))
        );
    }

    #[test]
    fn client_naks_invitation() {
        // REQ-NET-INV-019.
        for s in [Idle, PendingInvitation, EstablishedSession] {
            let mut c = Side::client(Capabilities::default());
            c.state = s;
            let o = c.receive(0, &A11);
            let (hdr, _) = parse(&A11);
            assert_eq!(
                o.reply,
                Some(Reply::Nak(NakReason::COMMAND_NOT_EXPECTED, hdr))
            );
            assert_eq!(o.next, None);
            let p = c.wire(&o);
            let (h2, Command::Nak(n)) = parse(&p) else {
                panic!()
            };
            assert_eq!(h2.csd1, 0x02);
            assert_eq!(n.nak_header, hdr);
        }
    }

    fn host_reply(r: Reply) -> Vec<u8> {
        let hs: Handshake = Handshake::host();
        bytes(r, &hs, &host_id())
    }

    #[test]
    fn client_ignores_accepted_when_established() {
        // REQ-NET-INV-022.
        let mut c = Side::client(Capabilities::default());
        c.state = EstablishedSession;
        let o = c.receive(0, &host_reply(Reply::Accepted));
        assert!(o.is_empty());
        assert_eq!(c.state, EstablishedSession);
    }

    #[test]
    fn client_answers_accepted_without_pending_invitation() {
        // REQ-NET-INV-023.
        let acc = host_reply(Reply::Accepted);
        let mut c = Side::client(Capabilities::default());
        let o = c.receive(0, &acc);
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::NO_PENDING_SESSION)));
        assert_eq!(o.next, None);
        assert_eq!(bye_reason(&c.wire(&o)), 0x06);
        let mut c = Side::client(Capabilities::default());
        c.state = AuthenticationRequired;
        let o = c.receive(0, &acc);
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::NO_PENDING_SESSION)));
        assert_eq!(c.state, PendingBye);
        for s in [PendingSessionReset, PendingBye] {
            let mut c = Side::client(Capabilities::default());
            c.state = s;
            assert!(c.receive(0, &acc).is_empty());
        }
    }

    #[test]
    fn pending_timeout_replaces_invitation_timeout() {
        // REQ-NET-INV-027.
        let pend = host_reply(Reply::Pending);
        let mut c = Side::client(Capabilities::default());
        c.start(0);
        let o = c.receive(5_000, &pend);
        assert_eq!(c.state, PendingInvitation);
        let [HandshakeEvent::Pending { peer }] = events(&o)[..] else {
            panic!("{o:?}")
        };
        assert_eq!(peer.name(), b"Host");
        assert_eq!(peer.product_instance_id(), b"H1");
        assert_eq!(c.hs.next_deadline(c.state), Some(65_000));
        // A later Pending does not start it again and gives no event.
        let o = c.receive(6_000, &pend);
        assert!(o.is_empty());
        assert_eq!(c.hs.next_deadline(c.state), Some(65_000));
        assert!(c.poll(10_000).is_empty());
        assert!(c.poll(64_999).is_empty());
        let o = c.poll(65_000);
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::INVITATION_CANCELED)));
        assert_eq!(c.state, PendingBye);
    }

    #[test]
    fn host_repeats_pending_without_event() {
        // REQ-NET-INV-028.
        let mut h = Side::host(HostPolicy::new(PolicyKind::AskCaller));
        let first = h.receive(0, &A11);
        let p1 = h.wire(&first);
        let o = h.receive(500, &A11);
        assert_eq!(o.reply, Some(Reply::Pending));
        assert!(o.events.is_empty());
        assert_eq!(o.next, None);
        assert_eq!(h.wire(&o), p1);
    }

    #[test]
    fn client_answers_pending_without_pending_invitation() {
        // REQ-NET-INV-029.
        let pend = host_reply(Reply::Pending);
        let mut c = Side::client(Capabilities::default());
        let o = c.receive(0, &pend);
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::NO_PENDING_SESSION)));
        assert_eq!(o.next, None);
        let mut c = Side::client(Capabilities::default());
        c.state = AuthenticationRequired;
        let o = c.receive(0, &pend);
        assert_eq!(o.reply, Some(Reply::Bye(ByeReason::NO_PENDING_SESSION)));
        assert_eq!(c.state, PendingBye);
        for s in [EstablishedSession, PendingSessionReset, PendingBye] {
            let mut c = Side::client(Capabilities::default());
            c.state = s;
            assert!(c.receive(0, &pend).is_empty());
        }
    }

    #[test]
    fn host_naks_invitation_replies() {
        // REQ-NET-INV-030, REQ-NET-AUTH-030.
        let replies = [
            host_reply(Reply::Accepted),
            host_reply(Reply::Pending),
            challenge_pkt(0x12, 0, b"nUWrn*@#$hjfwnkL"),
            challenge_pkt(0x13, 0, b"nUWrn*@#$hjfwnkL"),
            pkt(0x12, 0, 0, &[]),
        ];
        for p in &replies {
            for s in [
                Idle,
                PendingInvitation,
                AuthenticationRequired,
                EstablishedSession,
                PendingSessionReset,
                PendingBye,
            ] {
                let mut h = Side::host(HostPolicy::DEFAULT);
                h.state = s;
                let o = h.receive(0, p);
                let (hdr, _) = parse(p);
                assert_eq!(
                    o.reply,
                    Some(Reply::Nak(NakReason::COMMAND_NOT_EXPECTED, hdr))
                );
                assert_eq!(o.next, None);
                assert!(o.events.is_empty());
            }
        }
    }

    #[test]
    fn host_answer_timeout_deletes_entry() {
        // REQ-NET-SES-002.
        let mut h = Side::host(HostPolicy::new(PolicyKind::AskCaller));
        h.receive(0, &A11);
        assert_eq!(h.hs.next_deadline(h.state), Some(60_000));
        // Each Command of the Client starts it again.
        h.receive(30_000, &A11);
        assert_eq!(h.hs.next_deadline(h.state), Some(90_000));
        h.hs.touch(&h.cfg, 50_000);
        assert_eq!(h.hs.next_deadline(h.state), Some(110_000));
        assert!(h.poll(109_999).is_empty());
        let o = h.poll(110_000);
        assert_eq!(o.reply, None);
        assert_eq!(o.next, Some(Idle));
        assert_eq!(events(&o), [HandshakeEvent::Expired]);
        assert_eq!(h.hs.peer(), None);
        assert_eq!(h.hs.next_deadline(h.state), None);
    }

    #[test]
    fn auth_invitation_from_idle_gets_bye_41_else_nak() {
        // REQ-NET-AUTH-027, REQ-NET-AUTH-012 (with and without the feature).
        let mut user = Vec::from(&[0xAAu8; 32][..]);
        user.extend_from_slice(b"Rosa");
        for p in [pkt(0x02, 0, 0, &[0xAA; 32]), pkt(0x03, 0, 0, &user)] {
            let (hdr, c) = parse(&p);
            assert!(matches!(c, Command::Auth(_)));
            for _ in 0..2 {
                let mut h = Side::host(HostPolicy::DEFAULT);
                let o = h.receive(0, &p);
                assert_eq!(
                    o.reply,
                    Some(Reply::Bye(ByeReason::AUTH_INVITATION_REJECTED))
                );
                assert_eq!(o.next, None);
                assert_eq!(bye_reason(&h.wire(&o)), 0x41);
            }
            for s in [
                PendingInvitation,
                PendingSessionReset,
                PendingBye,
                EstablishedSession,
            ] {
                // Established Session without authentication: NAK (REQ-NET-AUTH-029 needs an
                // accepted digest).
                let mut h = Side::host(HostPolicy::DEFAULT);
                h.state = s;
                let o = h.receive(0, &p);
                assert_eq!(
                    o.reply,
                    Some(Reply::Nak(NakReason::COMMAND_NOT_EXPECTED, hdr))
                );
                assert_eq!(o.next, None);
            }
        }
    }

    #[test]
    fn client_naks_unexpected_auth_replies() {
        // REQ-NET-AUTH-031.
        let caps = Capabilities {
            auth: true,
            user_auth: true,
        };
        for code in [0x12u8, 0x13] {
            let p = challenge_pkt(code, 0, b"nUWrn*@#$hjfwnkL");
            for s in [Idle, EstablishedSession, PendingSessionReset, PendingBye] {
                let mut c = Side::client(caps);
                c.state = s;
                let o = c.receive(0, &p);
                let (hdr, _) = parse(&p);
                assert_eq!(
                    o.reply,
                    Some(Reply::Nak(NakReason::COMMAND_NOT_EXPECTED, hdr))
                );
                assert_eq!(o.next, None);
                assert!(o.events.is_empty());
            }
        }
    }

    #[test]
    fn client_naks_challenge_for_method_not_offered() {
        // REQ-NET-AUTH-032.
        for (caps, code) in [
            (Capabilities::default(), 0x12u8),
            (Capabilities::default(), 0x13),
            (
                Capabilities {
                    auth: false,
                    user_auth: true,
                },
                0x12,
            ),
            (
                Capabilities {
                    auth: true,
                    user_auth: false,
                },
                0x13,
            ),
        ] {
            let mut c = Side::client(caps);
            c.start(0);
            let p = challenge_pkt(code, 0, b"nUWrn*@#$hjfwnkL");
            let o = c.receive(10, &p);
            let (hdr, _) = parse(&p);
            assert_eq!(
                o.reply,
                Some(Reply::Nak(NakReason::COMMAND_NOT_EXPECTED, hdr))
            );
            assert_eq!(o.next, None);
            assert_eq!(c.state, PendingInvitation);
            assert_eq!(c.hs.repeating(), Some(Reply::Invitation));
            assert_eq!(c.hs.next_deadline(c.state), Some(10_000));
        }
    }

    #[test]
    fn capabilities_follow_the_feature() {
        // REQ-NET-AUTH-013.
        let both = Capabilities {
            auth: true,
            user_auth: true,
        };
        let mut c = Side::client(both);
        let o = c.start(0);
        let p = c.wire(&o);
        let (hdr, Command::Invitation(inv)) = parse(&p) else {
            panic!()
        };
        if cfg!(feature = "auth") {
            assert_eq!(inv.capabilities, both);
            assert_eq!(hdr.csd2, 0x03);
        } else {
            assert_eq!(inv.capabilities, Capabilities::default());
            assert_eq!(hdr.csd2, 0x00);
        }
        let mut c = Side::client(Capabilities {
            auth: false,
            user_auth: true,
        });
        let p = {
            let o = c.start(0);
            c.wire(&o)
        };
        let expected = if cfg!(feature = "auth") { 0x02 } else { 0x00 };
        assert_eq!(parse(&p).0.csd2, expected);
    }

    #[cfg(not(feature = "auth"))]
    #[test]
    fn host_without_feature_never_asks_for_authentication() {
        // REQ-NET-AUTH-014, REQ-NET-AUTH-015, REQ-NET-SES-002 (no Authentication Required).
        let inv = pkt(0x01, 2, 0x03, b"MyDev\0\0\08shYe3h5");
        for kind in [
            PolicyKind::SharedSecret,
            PolicyKind::UserList,
            PolicyKind::Both,
        ] {
            let mut h = Side::host(HostPolicy::new(kind));
            h.creds.random = Some([0; 16]);
            let o = h.receive(0, &inv);
            assert_eq!(o.reply, Some(Reply::Bye(ByeReason::NO_MATCHING_METHOD)));
            assert_eq!(h.state, Idle);
        }
        let hs: Handshake = Handshake::host();
        let mut buf = [0u8; 64];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        assert_eq!(
            Reply::AuthRequired { state: 0 }.write(&hs, &host_id(), &mut w),
            Err(HandshakeError::AuthNotAvailable)
        );
    }

    #[test]
    fn prop_never_panics_on_received_commands() {
        let states = [
            Idle,
            PendingInvitation,
            AuthenticationRequired,
            EstablishedSession,
            PendingSessionReset,
            PendingBye,
        ];
        proptest!(|(code in prop::sample::select(std::vec![0x01u8, 0x02, 0x03, 0x10, 0x11, 0x12, 0x13, 0x20]),
                    csd1 in any::<u8>(), csd2 in any::<u8>(),
                    body in prop::collection::vec(any::<u8>(), 0..80),
                    si in 0usize..6, host in any::<bool>(), now in any::<u64>())| {
            let p = pkt(code, csd1, csd2, &body);
            let Some(PacketItem::Command(raw)) = command_packets(&p).unwrap().next() else {
                return Ok(());
            };
            let Ok(cmd) = Command::parse(raw) else { return Ok(()); };
            let mut creds = Creds {
                random: Some([7; 16]),
                secret: Some(Vec::from(&b"s"[..])),
                users: std::vec![(Vec::from(&b"Rosa"[..]), Vec::from(&b"pw"[..]))],
                user: Some((Vec::from(&b"Rosa"[..]), Vec::from(&b"pw"[..]))),
                ..Creds::default()
            };
            let mut delay = FailureDelay::default();
            let mut env = Env::new(&mut creds)
                .with_delay(&mut delay)
                .with_policy(HostPolicy::new(PolicyKind::Both));
            let mut hs: Handshake = if host {
                Handshake::host()
            } else {
                Handshake::client(Capabilities::from_bits(3))
            };
            let cfg = HandshakeConfig::DEFAULT;
            let o = hs.on_command(&cfg, now, states[si], raw.header, &cmd, &mut env);
            if let Some(r) = o.reply {
                let mut buf = [0u8; 1400];
                let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
                let id = if host { host_id() } else { client_id() };
                // A reply may need data that a forced state does not have; no panic.
                let _ = r.write(&hs, &id, &mut w);
            }
            let s = o.next.unwrap_or(states[si]);
            let _ = hs.poll(&cfg, now, s);
            let _ = hs.next_deadline(s);
        });
    }

    #[cfg(feature = "auth")]
    mod auth_flow {
        use super::*;
        use crate::auth::{shared_secret_digest, user_digest, CryptoNonce};

        const NONCE_69: [u8; 16] = *b"nUWrn*@#$hjfwnkL";
        const NONCE_610: [u8; 16] = *b"XI|~=NNRVaD;XCPL";
        /// 6.9 example digest (nonce NONCE_69, secret "5483").
        const DIGEST_69: [u8; 32] = [
            0x67, 0x6E, 0xBE, 0x82, 0x58, 0x7C, 0xEC, 0xA8, 0xF8, 0x2F, 0xC3, 0x33, 0xD7, 0x87,
            0x95, 0x1E, 0xEC, 0x2B, 0x00, 0xAD, 0x31, 0x61, 0x3C, 0xC4, 0xDB, 0x18, 0xCF, 0x27,
            0x37, 0x3A, 0xFB, 0x82,
        ];
        /// 6.10 example digest (nonce NONCE_610, "Rosa", "RPBqBno").
        const DIGEST_610: [u8; 32] = [
            0x3A, 0x2D, 0x5E, 0xBE, 0xEF, 0x92, 0xE8, 0x46, 0x35, 0x35, 0xC2, 0x7F, 0xB4, 0xB7,
            0xB3, 0xE2, 0xD1, 0xAC, 0xF5, 0x7A, 0x84, 0x4C, 0x6D, 0x08, 0x08, 0xE0, 0x41, 0xC1,
            0x02, 0xB7, 0xFF, 0x1A,
        ];

        /// Random bytes that give `nonce` (32 + b mod 95).
        fn random_for(nonce: [u8; 16]) -> [u8; 16] {
            nonce.map(|b| b - 32)
        }

        const SHARED: Capabilities = Capabilities {
            auth: true,
            user_auth: false,
        };
        const USER: Capabilities = Capabilities {
            auth: false,
            user_auth: true,
        };
        const BOTH: Capabilities = Capabilities {
            auth: true,
            user_auth: true,
        };

        fn shared_pair() -> (Side, Side) {
            let mut h = Side::host(HostPolicy::new(PolicyKind::SharedSecret));
            h.creds.random = Some(random_for(NONCE_69));
            h.creds.secret = Some(Vec::from(&b"5483"[..]));
            let mut c = Side::client(SHARED);
            c.creds.secret = Some(Vec::from(&b"5483"[..]));
            (h, c)
        }

        fn user_pair() -> (Side, Side) {
            let mut h = Side::host(HostPolicy::new(PolicyKind::UserList));
            h.creds.random = Some(random_for(NONCE_610));
            h.creds.users = std::vec![(Vec::from(&b"Rosa"[..]), Vec::from(&b"RPBqBno"[..]))];
            let mut c = Side::client(USER);
            c.creds.user = Some((Vec::from(&b"Rosa"[..]), Vec::from(&b"RPBqBno"[..])));
            (h, c)
        }

        /// Invitation, then the challenge of the Host; gives the Host Outcome.
        fn invite(now: u64, h: &mut Side, c: &mut Side) -> Outcome {
            let o = c.start(now);
            send(now, c, &o, h)
        }

        #[test]
        fn shared_secret_flow_with_spec_vector() {
            // REQ-NET-AUTH-014, -016, -017, -020, -021, -033, -034, -036.
            let (mut h, mut c) = shared_pair();
            let oh = invite(0, &mut h, &mut c);
            assert_eq!(oh.reply, Some(Reply::AuthRequired { state: 0 }));
            assert_eq!(h.state, AuthenticationRequired);
            let p = h.wire(&oh);
            let (hdr, Command::Auth(raw)) = parse(&p) else {
                panic!()
            };
            assert_eq!(hdr.csd2, 0x00);
            let ar = AuthRequired::read(&raw).unwrap();
            assert_eq!(ar.nonce.as_bytes(), &NONCE_69);
            assert_eq!(ar.name.as_bytes(), b"Host");
            // The Client answers with the digest of 6.9.
            let oc = c.receive(10, &p);
            assert_eq!(oc.reply, Some(Reply::InvitationWithAuth));
            assert_eq!(c.state, PendingInvitation);
            assert_eq!(oc.repeat, Repeat::Start(Reply::InvitationWithAuth));
            let [HandshakeEvent::AuthRequested { peer, challenge }] = events(&oc)[..] else {
                panic!("{oc:?}")
            };
            assert_eq!(peer.name(), b"Host");
            assert_eq!(
                challenge,
                Challenge {
                    method: Method::SharedSecret,
                    state: 0
                }
            );
            let p2 = c.wire(&oc);
            let (_, Command::Auth(raw2)) = parse(&p2) else {
                panic!()
            };
            assert_eq!(
                InvitationWithAuth::read(&raw2).unwrap().digest.as_bytes(),
                &DIGEST_69
            );
            // The Host verifies with the kept nonce.
            let oh = h.receive(20, &p2);
            assert_eq!(oh.reply, Some(Reply::Accepted));
            assert_eq!(h.state, EstablishedSession);
            assert!(matches!(
                events(&oh)[..],
                [HandshakeEvent::Established {
                    method: Some(Method::SharedSecret),
                    ..
                }]
            ));
            let oc = send(30, &h, &oh, &mut c);
            assert_eq!(c.state, EstablishedSession);
            assert!(matches!(
                events(&oc)[..],
                [HandshakeEvent::Established {
                    method: Some(Method::SharedSecret),
                    ..
                }]
            ));
        }

        #[test]
        fn user_flow_with_spec_vector() {
            // REQ-NET-AUTH-015, -023, -024, -035.
            let (mut h, mut c) = user_pair();
            let oh = invite(0, &mut h, &mut c);
            assert_eq!(oh.reply, Some(Reply::UserAuthRequired { state: 0 }));
            // REQ-NET-AUTH-016: State 0x00 on the wire, with the nonce.
            let wire = h.wire(&oh);
            let (hdr, Command::Auth(raw)) = parse(&wire) else {
                panic!()
            };
            assert_eq!((hdr.code, hdr.csd2), (0x13, 0x00));
            let ur = crate::auth::UserAuthRequired::read(&raw).unwrap();
            assert_eq!(ur.nonce.as_bytes(), &NONCE_610);
            let oc = send(10, &h, &oh, &mut c);
            assert_eq!(oc.reply, Some(Reply::InvitationWithUserAuth));
            assert!(matches!(
                events(&oc)[..],
                [HandshakeEvent::AuthRequested {
                    challenge: Challenge {
                        method: Method::User,
                        state: 0
                    },
                    ..
                }]
            ));
            let p = c.wire(&oc);
            let (_, Command::Auth(raw)) = parse(&p) else {
                panic!()
            };
            let iu = InvitationWithUserAuth::read(&raw).unwrap();
            assert_eq!(iu.digest.as_bytes(), &DIGEST_610);
            assert_eq!(iu.username.as_bytes(), b"Rosa");
            let oh = h.receive(20, &p);
            assert_eq!(oh.reply, Some(Reply::Accepted));
            assert_eq!(h.state, EstablishedSession);
        }

        #[test]
        fn method_follows_capabilities_and_preference() {
            // REQ-NET-AUTH-014, REQ-NET-AUTH-015 (N-44).
            let cases = [
                (PolicyKind::Both, Method::User, BOTH, Some(0x13u8)),
                (PolicyKind::Both, Method::SharedSecret, BOTH, Some(0x12)),
                (PolicyKind::Both, Method::User, SHARED, Some(0x12)),
                (PolicyKind::Both, Method::SharedSecret, USER, Some(0x13)),
                (PolicyKind::SharedSecret, Method::User, USER, None),
                (PolicyKind::UserList, Method::User, SHARED, None),
            ];
            for (kind, preferred, caps, code) in cases {
                let mut policy = HostPolicy::new(kind);
                policy.preferred = preferred;
                let mut h = Side::host(policy);
                h.creds.random = Some([1; 16]);
                let mut c = Side::client(caps);
                let oh = invite(0, &mut h, &mut c);
                match code {
                    Some(code) => {
                        assert_eq!(parse(&h.wire(&oh)).0.code, code);
                        assert_eq!(h.state, AuthenticationRequired);
                    }
                    None => {
                        assert_eq!(oh.reply, Some(Reply::Bye(ByeReason::NO_MATCHING_METHOD)));
                        assert_eq!(h.state, Idle);
                    }
                }
            }
        }

        #[test]
        fn duplicate_invitation_gets_the_same_challenge() {
            // REQ-NET-AUTH-019, REQ-NET-AUTH-016.
            let (mut h, mut c) = shared_pair();
            let o = c.start(0);
            let inv = c.wire(&o);
            let first = h.receive(0, &inv);
            let p1 = h.wire(&first);
            h.creds.random = Some([9; 16]); // a new nonce would differ
            let again = h.receive(500, &inv);
            assert_eq!(again.reply, Some(Reply::AuthRequired { state: 0 }));
            assert_eq!(again.next, None);
            assert_eq!(h.wire(&again), p1);
            assert_eq!(h.state, AuthenticationRequired);
        }

        #[test]
        fn wrong_secret_rechallenges_then_bye_43() {
            // REQ-NET-AUTH-022, REQ-NET-AUTH-017.
            let (mut h, mut c) = shared_pair();
            c.creds.secret = Some(Vec::from(&b"wrong"[..]));
            let oh = invite(0, &mut h, &mut c);
            let mut p = h.wire(&oh);
            for n in 1..=4u32 {
                let oc = c.receive(u64::from(n) * 10, &p);
                let oh = send(u64::from(n) * 10, &c, &oc, &mut h);
                assert_eq!(oh.reply, Some(Reply::AuthRequired { state: 1 }));
                assert_eq!(oh.next, None);
                assert_eq!(h.state, AuthenticationRequired);
                assert_eq!(h.hs.failures(), n);
                p = h.wire(&oh);
                let (hdr, Command::Auth(raw)) = parse(&p) else {
                    panic!()
                };
                assert_eq!(hdr.csd2, 0x01);
                // N-51: the same nonce.
                assert_eq!(
                    AuthRequired::read(&raw).unwrap().nonce.as_bytes(),
                    &NONCE_69
                );
            }
            let oc = c.receive(100, &p);
            assert!(matches!(
                events(&oc)[..],
                [HandshakeEvent::AuthRequested {
                    challenge: Challenge { state: 1, .. },
                    ..
                }]
            ));
            let oh = send(100, &c, &oc, &mut h);
            assert_eq!(oh.reply, Some(Reply::Bye(ByeReason::AUTHENTICATION_FAILED)));
            assert_eq!(h.state, Idle);
            assert_eq!(bye_reason(&h.wire(&oh)), 0x43);
            assert!(events(&oh).contains(&HandshakeEvent::Ended(ByeReason::AUTHENTICATION_FAILED)));
            // The nonce is gone (REQ-NET-AUTH-017).
            let mut buf = [0u8; 128];
            let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
            assert_eq!(
                Reply::AuthRequired { state: 0 }.write(&h.hs, &h.id, &mut w),
                Err(HandshakeError::NothingToWrite)
            );
        }

        #[test]
        fn wrong_password_rechallenges_with_state_01() {
            // REQ-NET-AUTH-046.
            let (mut h, mut c) = user_pair();
            h.hs = Handshake::host();
            h.cfg = HandshakeConfig {
                failure_limit: 2,
                ..HandshakeConfig::DEFAULT
            };
            c.creds.user = Some((Vec::from(&b"Rosa"[..]), Vec::from(&b"bad"[..])));
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            let oh = send(2, &c, &oc, &mut h);
            assert_eq!(oh.reply, Some(Reply::UserAuthRequired { state: 1 }));
            assert_eq!(h.state, AuthenticationRequired);
            // State 0x01 on the wire and the same nonce.
            let wire = h.wire(&oh);
            let (hdr, Command::Auth(raw)) = parse(&wire) else {
                panic!()
            };
            assert_eq!(hdr.csd2, 0x01);
            let ur = crate::auth::UserAuthRequired::read(&raw).unwrap();
            assert_eq!(ur.nonce.as_bytes(), &NONCE_610);
            let oc = send(3, &h, &oh, &mut c);
            let oh = send(4, &c, &oc, &mut h);
            assert_eq!(oh.reply, Some(Reply::Bye(ByeReason::AUTHENTICATION_FAILED)));
            assert_eq!(h.state, Idle);
        }

        #[test]
        fn unknown_user_gets_state_02_then_bye_44() {
            // REQ-NET-AUTH-025, REQ-NET-AUTH-023.
            let (mut h, mut c) = user_pair();
            h.hs = Handshake::host();
            h.cfg = HandshakeConfig {
                failure_limit: 2,
                ..HandshakeConfig::DEFAULT
            };
            c.creds.user = Some((Vec::from(&b"Nobody"[..]), Vec::from(&b"x"[..])));
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            let oh = send(2, &c, &oc, &mut h);
            assert_eq!(oh.reply, Some(Reply::UserAuthRequired { state: 2 }));
            // State 0x02 on the wire and the same nonce.
            let wire = h.wire(&oh);
            let (hdr, Command::Auth(raw)) = parse(&wire) else {
                panic!()
            };
            assert_eq!(hdr.csd2, 0x02);
            let ur = crate::auth::UserAuthRequired::read(&raw).unwrap();
            assert_eq!(ur.nonce.as_bytes(), &NONCE_610);
            assert!(events(&oh).contains(&HandshakeEvent::AuthFailed {
                method: Method::User,
                failures: 1,
                username_not_found: true
            }));
            let oc = send(3, &h, &oh, &mut c);
            assert!(matches!(
                events(&oc)[..],
                [HandshakeEvent::AuthRequested {
                    challenge: Challenge { state: 2, .. },
                    ..
                }]
            ));
            let oh = send(4, &c, &oc, &mut h);
            assert_eq!(oh.reply, Some(Reply::Bye(ByeReason::USERNAME_NOT_FOUND)));
            assert_eq!(h.state, Idle);
            assert_eq!(bye_reason(&h.wire(&oh)), 0x44);
        }

        #[test]
        fn other_method_counts_as_failure() {
            // REQ-NET-AUTH-028.
            let mut user = Vec::from(&[0xAAu8; 32][..]);
            user.extend_from_slice(b"Rosa");
            let (mut h, mut c) = shared_pair();
            invite(0, &mut h, &mut c);
            let o = h.receive(1, &pkt(0x03, 0, 0, &user));
            assert_eq!(o.reply, Some(Reply::AuthRequired { state: 1 }));
            assert_eq!(h.hs.failures(), 1);
            let (mut h, mut c) = user_pair();
            invite(0, &mut h, &mut c);
            let o = h.receive(1, &pkt(0x02, 0, 0, &[0xAA; 32]));
            assert_eq!(o.reply, Some(Reply::UserAuthRequired { state: 1 }));
            assert_eq!(h.state, AuthenticationRequired);
        }

        #[test]
        fn repeated_accepted_digest_gets_accepted_again() {
            // REQ-NET-AUTH-029.
            let (mut h, mut c) = shared_pair();
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            let p = c.wire(&oc);
            h.receive(2, &p);
            assert_eq!(h.state, EstablishedSession);
            let o = h.receive(3, &p);
            assert_eq!(o.reply, Some(Reply::Accepted));
            assert_eq!(o.next, None);
            assert!(o.events.is_empty());
            let bad = pkt(0x02, 0, 0, &[0x55; 32]);
            let o = h.receive(4, &bad);
            assert_eq!(
                o.reply,
                Some(Reply::Nak(NakReason::COMMAND_NOT_EXPECTED, parse(&bad).0))
            );
            assert_eq!(h.state, EstablishedSession);
            // The other method with the same digest: NAK.
            let mut user = Vec::from(&DIGEST_69[..]);
            user.extend_from_slice(b"Rosa");
            let o = h.receive(5, &pkt(0x03, 0, 0, &user));
            assert!(matches!(
                o.reply,
                Some(Reply::Nak(NakReason::COMMAND_NOT_EXPECTED, _))
            ));
        }

        #[test]
        fn client_decline_sends_bye_80() {
            // REQ-NET-AUTH-034, REQ-NET-AUTH-035 (N-54), N-53.
            let (mut h, mut c) = shared_pair();
            c.creds.secret = None;
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            assert_eq!(oc.reply, Some(Reply::Bye(ByeReason::INVITATION_CANCELED)));
            assert_eq!(c.state, PendingBye);
            assert!(matches!(
                events(&oc)[..],
                [
                    HandshakeEvent::AuthRequested { .. },
                    HandshakeEvent::Ended(ByeReason::INVITATION_CANCELED)
                ]
            ));
            assert_eq!(bye_reason(&c.wire(&oc)), 0x80);

            let (mut h, mut c) = user_pair();
            c.creds.user = None;
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            assert_eq!(oc.reply, Some(Reply::Bye(ByeReason::INVITATION_CANCELED)));
            assert_eq!(c.state, PendingBye);

            // Empty secret, empty password, a Username that does not fit: errors.
            type Case = (
                Capabilities,
                Option<Vec<u8>>,
                Option<(Vec<u8>, Vec<u8>)>,
                CredentialsError,
            );
            let cases: [Case; 3] = [
                (
                    SHARED,
                    Some(Vec::new()),
                    None,
                    CredentialsError::EmptySecret,
                ),
                (
                    USER,
                    None,
                    Some((Vec::from(&b"Rosa"[..]), Vec::new())),
                    CredentialsError::EmptyPassword,
                ),
                (
                    USER,
                    None,
                    Some((std::vec![b'u'; 129], Vec::from(&b"pw"[..]))),
                    CredentialsError::UsernameTooLong,
                ),
            ];
            for (caps, secret, user, err) in cases {
                let mut c = Side::client(caps);
                c.creds.secret = secret;
                c.creds.user = user;
                c.start(0);
                let code = if caps.auth { 0x12 } else { 0x13 };
                let o = c.receive(1, &challenge_pkt(code, 0, &NONCE_69));
                assert_eq!(o.reply, Some(Reply::Bye(ByeReason::INVITATION_CANCELED)));
                assert!(events(&o).contains(&HandshakeEvent::CredentialsError(err)));
            }
        }

        #[test]
        fn client_waits_in_authentication_required_for_the_caller() {
            // REQ-NET-AUTH-020, REQ-NET-AUTH-036.
            let (mut h, mut c) = shared_pair();
            c.creds.later = true;
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            assert_eq!(oc.reply, None);
            assert_eq!(c.state, AuthenticationRequired);
            assert_eq!(oc.repeat, Repeat::Stop);
            assert_eq!(c.hs.repeating(), None);
            assert_eq!(c.creds.secret_calls, 0);
            // Only in Authentication Required.
            assert!(c
                .hs
                .answer(&c.cfg, 2, PendingInvitation, &mut c.creds)
                .is_empty());
            let o = c.hs.answer(&c.cfg, 2, c.state, &mut c.creds);
            c.apply(&o);
            assert_eq!(o.reply, Some(Reply::InvitationWithAuth));
            assert_eq!(c.state, PendingInvitation);
            assert_eq!(o.repeat, Repeat::Start(Reply::InvitationWithAuth));
            let oh = send(3, &c, &o, &mut h);
            assert_eq!(oh.reply, Some(Reply::Accepted));
            // Pending Invitation until an Invitation Reply: a Pending keeps it.
            let pend = host_reply(Reply::Pending);
            let (mut h2, mut c2) = shared_pair();
            let oh2 = invite(0, &mut h2, &mut c2);
            send(1, &h2, &oh2, &mut c2);
            assert_eq!(c2.state, PendingInvitation);
            c2.receive(2, &pend);
            assert_eq!(c2.state, PendingInvitation);
            assert_eq!(c2.hs.repeating(), Some(Reply::InvitationWithAuth));
        }

        #[test]
        fn auth_command_repeats_with_the_same_bytes() {
            // REQ-NET-AUTH-037.
            let (mut h, mut c) = shared_pair();
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            let first = c.wire(&oc);
            assert_eq!(c.hs.repeating(), Some(Reply::InvitationWithAuth));
            assert_eq!(bytes(Reply::InvitationWithAuth, &c.hs, &c.id), first);
            assert_eq!(c.creds.secret_calls, 1);
            // A duplicate first challenge (reply to a repeated Invitation): no new prompt.
            let p = h.wire(&oh);
            let o = c.receive(2, &p);
            assert!(o.is_empty());
            assert_eq!(c.creds.secret_calls, 1);
            assert_eq!(bytes(Reply::InvitationWithAuth, &c.hs, &c.id), first);
            // User method: the Username is kept for the repeats.
            let (mut h, mut c) = user_pair();
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            let first = c.wire(&oc);
            c.creds.user = None;
            assert_eq!(bytes(Reply::InvitationWithUserAuth, &c.hs, &c.id), first);
        }

        #[test]
        fn reserved_authentication_state_is_processed_as_first_request() {
            // REQ-NET-AUTH-003, REQ-NET-AUTH-004: a reserved state with the same nonce is a
            // duplicate first request: no new prompt, the same repeat.
            let mut c = Side::client(SHARED);
            c.creds.secret = Some(Vec::from(&b"5483"[..]));
            c.start(0);
            let o = c.receive(1, &challenge_pkt(0x12, 0x00, &NONCE_69));
            let first = c.wire(&o);
            assert_eq!(c.creds.secret_calls, 1);
            let o = c.receive(2, &challenge_pkt(0x12, 0x7E, &NONCE_69));
            assert!(o.is_empty());
            assert_eq!(c.creds.secret_calls, 1);
            assert_eq!(c.hs.repeating(), Some(Reply::InvitationWithAuth));
            assert_eq!(bytes(Reply::InvitationWithAuth, &c.hs, &c.id), first);
            // State 0x01 with the same nonce is not a duplicate: a new answer.
            c.receive(3, &challenge_pkt(0x12, 0x01, &NONCE_69));
            assert_eq!(c.creds.secret_calls, 2);
            // User Authentication: state 0x03 is processed as 0x00.
            let mut c = Side::client(USER);
            c.creds.user = Some((Vec::from(&b"Rosa"[..]), Vec::from(&b"RPBqBno"[..])));
            c.start(0);
            let o = c.receive(1, &challenge_pkt(0x13, 0x00, &NONCE_610));
            let first = c.wire(&o);
            let asked = c.creds.challenges.len();
            let o = c.receive(2, &challenge_pkt(0x13, 0x03, &NONCE_610));
            assert!(o.is_empty());
            assert_eq!(c.creds.challenges.len(), asked);
            assert_eq!(bytes(Reply::InvitationWithUserAuth, &c.hs, &c.id), first);
        }

        #[test]
        fn invitation_timeout_restarts_for_a_new_challenge() {
            // REQ-NET-AUTH-038 (N-55).
            let mut c = Side::client(SHARED);
            c.creds.secret = Some(Vec::from(&b"5483"[..]));
            c.start(0);
            assert_eq!(c.hs.next_deadline(c.state), Some(10_000));
            c.receive(8_000, &challenge_pkt(0x12, 0, &NONCE_69));
            assert_eq!(c.state, PendingInvitation);
            assert_eq!(c.hs.next_deadline(c.state), Some(18_000));
            // A new state: restart.
            c.receive(15_000, &challenge_pkt(0x12, 1, &NONCE_69));
            assert_eq!(c.hs.next_deadline(c.state), Some(25_000));
            // The same nonce and state again: a new answer, no restart.
            c.receive(16_000, &challenge_pkt(0x12, 1, &NONCE_69));
            assert_eq!(c.creds.secret_calls, 3);
            assert_eq!(c.hs.next_deadline(c.state), Some(25_000));
            // A new nonce: restart.
            c.receive(17_000, &challenge_pkt(0x12, 1, b"abcdefghijklmnop"));
            assert_eq!(c.hs.next_deadline(c.state), Some(27_000));
            assert!(c.poll(26_999).is_empty());
            let o = c.poll(27_000);
            assert_eq!(o.reply, Some(Reply::Bye(ByeReason::INVITATION_CANCELED)));
            assert_eq!(o.repeat, Repeat::Stop);
            assert_eq!(c.state, PendingBye);
        }

        #[test]
        fn client_uses_the_nonce_of_the_last_challenge() {
            // REQ-NET-AUTH-033.
            let mut c = Side::client(SHARED);
            c.creds.secret = Some(Vec::from(&b"5483"[..]));
            c.start(0);
            c.receive(1, &challenge_pkt(0x12, 0, &NONCE_69));
            let other = *b"abcdefghijklmnop";
            let o = c.receive(2, &challenge_pkt(0x12, 1, &other));
            let p = c.wire(&o);
            let (_, Command::Auth(raw)) = parse(&p) else {
                panic!()
            };
            let want = shared_secret_digest(&CryptoNonce::new(other).unwrap(), b"5483").unwrap();
            assert_eq!(InvitationWithAuth::read(&raw).unwrap().digest, want);
            // The user digest uses the kept nonce the same way.
            let mut c = Side::client(USER);
            c.creds.user = Some((Vec::from(&b"Rosa"[..]), Vec::from(&b"RPBqBno"[..])));
            c.start(0);
            let o = c.receive(1, &challenge_pkt(0x13, 0, &NONCE_610));
            let p = c.wire(&o);
            let (_, Command::Auth(raw)) = parse(&p) else {
                panic!()
            };
            let want = user_digest(&CryptoNonce::new(NONCE_610).unwrap(), b"Rosa", b"RPBqBno");
            assert_eq!(
                InvitationWithUserAuth::read(&raw).unwrap().digest,
                want.unwrap()
            );
            // clear() discards the nonce and the digest.
            c.hs.clear();
            let mut buf = [0u8; 128];
            let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
            assert_eq!(
                Reply::InvitationWithUserAuth.write(&c.hs, &c.id, &mut w),
                Err(HandshakeError::NothingToWrite)
            );
        }

        #[test]
        fn delayed_failure_reply_is_due_later() {
            // REQ-NET-AUTH-047 (with REQ-NET-AUTH-026 delays).
            let (mut h, mut c) = shared_pair();
            h.use_delay = true;
            h.delay = FailureDelay::default();
            c.creds.secret = Some(Vec::from(&b"wrong"[..]));
            let oh = invite(0, &mut h, &mut c);
            let oc = send(0, &h, &oh, &mut c);
            let wrong = c.wire(&oc);
            let o = h.receive(1_000, &wrong);
            assert_eq!(o.reply, None);
            assert_eq!(o.next, None);
            assert!(matches!(
                events(&o)[..],
                [HandshakeEvent::AuthFailed { failures: 1, .. }]
            ));
            assert_eq!(h.hs.delayed_due(), Some(1_100));
            assert_eq!(h.hs.next_deadline(h.state), Some(1_100));
            // Attempts while the reply is due are ignored.
            assert!(h.receive(1_050, &wrong).is_empty());
            assert_eq!(h.hs.failures(), 1);
            assert!(h.poll(1_099).is_empty());
            let o = h.poll(1_100);
            assert_eq!(o.reply, Some(Reply::AuthRequired { state: 1 }));
            assert_eq!(h.hs.delayed_due(), None);
            // The second failure waits 200 ms.
            h.receive(2_000, &wrong);
            assert_eq!(h.hs.delayed_due(), Some(2_200));
            // A correct digest after the delay: the counter goes back to 0.
            h.poll(2_200);
            c.creds.secret = Some(Vec::from(&b"5483"[..]));
            let oc = c.receive(2_300, &challenge_pkt(0x12, 1, &NONCE_69));
            let oh = send(2_300, &c, &oc, &mut h);
            assert_eq!(oh.reply, Some(Reply::Accepted));
            assert_eq!(h.delay.failures(), 0);
            // clear() discards a delayed reply (REQ-NET-AUTH-017).
            let (mut h, mut c) = shared_pair();
            h.use_delay = true;
            h.delay = FailureDelay::default();
            c.creds.secret = Some(Vec::from(&b"wrong"[..]));
            let oh = invite(0, &mut h, &mut c);
            let oc = send(0, &h, &oh, &mut c);
            h.receive(10, &c.wire(&oc));
            assert!(h.hs.delayed_due().is_some());
            h.hs.clear();
            assert_eq!(h.hs.delayed_due(), None);
        }

        #[test]
        fn host_credentials_errors() {
            // N-53: an empty secret is an error; no random bytes: no challenge.
            let (mut h, mut c) = shared_pair();
            h.creds.secret = Some(Vec::new());
            let oh = invite(0, &mut h, &mut c);
            let oc = send(1, &h, &oh, &mut c);
            let o = send(2, &c, &oc, &mut h);
            assert_eq!(o.reply, Some(Reply::AuthRequired { state: 1 }));
            assert!(events(&o).contains(&HandshakeEvent::CredentialsError(
                CredentialsError::EmptySecret
            )));
            let (mut h, mut c) = shared_pair();
            h.creds.random = None;
            let o = invite(0, &mut h, &mut c);
            assert_eq!(o.reply, Some(Reply::Bye(ByeReason::NO_MATCHING_METHOD)));
            assert!(events(&o).contains(&HandshakeEvent::CredentialsError(
                CredentialsError::NoRandom
            )));
        }

        #[test]
        fn cancel_in_authentication_required() {
            // REQ-NET-INV-010 (also after an authentication reply).
            let (mut h, mut c) = shared_pair();
            c.creds.later = true;
            let oh = invite(0, &mut h, &mut c);
            send(1, &h, &oh, &mut c);
            assert_eq!(c.state, AuthenticationRequired);
            let o = c.hs.cancel(&c.cfg, 2, c.state, None).unwrap();
            assert_eq!(o.reply, Some(Reply::Bye(ByeReason::INVITATION_CANCELED)));
            assert_eq!(o.next, Some(PendingBye));
            let o = h.hs.cancel(&h.cfg, 2, h.state, None).unwrap();
            assert_eq!(o.reply, Some(Reply::Bye(ByeReason::USER_DID_NOT_ACCEPT)));
            assert_eq!(o.next, Some(PendingBye));
            assert_eq!(h.hs.next_deadline(PendingBye), None);
        }

        #[test]
        fn malformed_auth_commands_get_nak_03() {
            // REQ-NET-AUTH-012 with the feature: the payload is read here.
            let (mut h, mut c) = shared_pair();
            invite(0, &mut h, &mut c);
            let short = pkt(0x02, 0, 0, &[1; 8]);
            let o = h.receive(1, &short);
            assert_eq!(
                o.reply,
                Some(Reply::Nak(NakReason::COMMAND_MALFORMED, parse(&short).0))
            );
            assert_eq!(h.hs.failures(), 0);
            // REQ-NET-AUTH-008: an empty Username (pl 8, or a first byte 0x00) gets NAK 0x03 and
            // changes nothing.
            let (mut h, mut c) = user_pair();
            invite(0, &mut h, &mut c);
            let mut zero_name = Vec::from(DIGEST_610);
            zero_name.extend_from_slice(&[0, b'R', b'o', b's']);
            for p in [pkt(0x03, 0, 0, &DIGEST_610), pkt(0x03, 0, 0, &zero_name)] {
                let o = h.receive(1, &p);
                assert_eq!(
                    o.reply,
                    Some(Reply::Nak(NakReason::COMMAND_MALFORMED, parse(&p).0))
                );
                assert_eq!(o.next, None);
                assert_eq!(h.state, AuthenticationRequired);
                assert_eq!(h.hs.failures(), 0);
            }
            let mut c = Side::client(SHARED);
            c.start(0);
            let short = pkt(0x12, 1, 0, &[0x41; 8]);
            let o = c.receive(1, &short);
            assert_eq!(
                o.reply,
                Some(Reply::Nak(NakReason::COMMAND_MALFORMED, parse(&short).0))
            );
            assert_eq!(c.state, PendingInvitation);
        }
    }
}
