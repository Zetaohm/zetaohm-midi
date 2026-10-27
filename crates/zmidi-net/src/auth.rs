//! Authentication of Network MIDI 2.0 (M2-124-UM v1.0.1 sections 6.7 to 6.10), behind the
//! Cargo feature `auth` (DEC-013).
//!
//! - Invitation Reply: Authentication Required (0x12, Table 14) and User Authentication
//!   Required (0x13, Table 16) carry an Authentication State (Tables 15 and 17), a 16-byte
//!   CryptoNonce, the UMP Endpoint Name and the Product Instance Id (REQ-NET-AUTH-001 to -004,
//!   -040, -041).
//! - Invitation with Authentication (0x02, Table 18) and with User Authentication (0x03,
//!   Table 19) carry the SHA-256 Authentication Digest, and 0x03 also the Username
//!   (REQ-NET-AUTH-006 to -008, -042 to -045).
//! - The digest is SHA-256 of the CryptoNonce and the shared secret (6.9), or of the
//!   CryptoNonce, the Username and the password (6.10) (REQ-NET-AUTH-009, -010). SHA-256 comes
//!   from `sha2` with no alloc. The compare is constant-time.
//! - Sans-IO (DEC-012): the caller gives the random bytes, the time and the credentials; the
//!   module keeps no secret and no password (REQ-NET-AUTH-011, N-52). No alloc (DEC-002).
//!   Nothing panics on received bytes.
//! - Readers work on a [`RawCommand`] of the right Command Code (from
//!   [`crate::command::Command::Auth`]); they do not check the code.

use crate::command::{CommandCode, CommandError, Malformed};
use crate::identity::{EndpointName, ProductInstanceId};
use crate::packet::{string_words, NetStr, PacketError, PayloadWriter, RawCommand};
use sha2::{Digest, Sha256};

/// Number of bytes of a CryptoNonce (6.7, REQ-NET-AUTH-005).
pub const NONCE_LEN: usize = 16;
/// Number of bytes of an Authentication Digest (SHA-256, Table 18).
pub const DIGEST_LEN: usize = 32;
/// Largest Username that the writer writes, in bytes: 255 * 4 - 32 (Table 19,
/// REQ-NET-AUTH-044).
pub const MAX_USERNAME: usize = 988;
/// Default initial delay of the reply to a failed authentication, in ms (REQ-NET-AUTH-026).
pub const DEFAULT_INITIAL_DELAY_MS: u64 = 100;
/// Default maximum delay of the reply to a failed authentication, in ms (REQ-NET-AUTH-026,
/// N-45).
pub const DEFAULT_MAX_DELAY_MS: u64 = 3200;

/// Payload words of the CryptoNonce.
const NONCE_WORDS: usize = NONCE_LEN / 4;
/// Payload words of the Authentication Digest.
const DIGEST_WORDS: usize = DIGEST_LEN / 4;

/// Error of the Auth writers and of the digest functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The Command writer refused (does not fit, too large).
    #[error("command: {0}")]
    Command(#[from] CommandError),
    /// A CryptoNonce byte outside ASCII 32 to 126 (REQ-NET-AUTH-005).
    #[error("CryptoNonce byte out of range")]
    InvalidNonce,
    /// An Authentication State that is Reserved in its table (REQ-NET-AUTH-003, -004).
    #[error("reserved Authentication State")]
    ReservedState,
    /// An empty shared secret (REQ-NET-AUTH-011, N-53).
    #[error("empty shared secret")]
    EmptySecret,
    /// An empty password (REQ-NET-AUTH-011, N-53).
    #[error("empty password")]
    EmptyPassword,
    /// An empty Username (6.10, REQ-NET-AUTH-008).
    #[error("empty Username")]
    EmptyUsername,
    /// A Username longer than 988 bytes (REQ-NET-AUTH-044).
    #[error("Username too long")]
    UsernameTooLong,
    /// A Username that is not valid UTF-8, contains 0x00 or starts with a Byte Order Mark
    /// (REQ-NET-AUTH-044, REQ-NET-PKT-007).
    #[error("invalid Username")]
    InvalidUsername,
}

impl From<PacketError> for AuthError {
    fn from(e: PacketError) -> Self {
        AuthError::Command(CommandError::Packet(e))
    }
}

/// A CryptoNonce: exactly 16 bytes with no 0x00 terminator (6.7, REQ-NET-AUTH-005).
///
/// [`CryptoNonce::from_random`] and [`CryptoNonce::new`] give only bytes from 32 to 126. A
/// received CryptoNonce ([`CryptoNonce::from_received`]) keeps any byte values, and the digest
/// uses them as received (N-49).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CryptoNonce([u8; NONCE_LEN]);

impl CryptoNonce {
    /// A new CryptoNonce from 16 random bytes of the caller: each byte `b` gives
    /// `32 + (b mod 95)` (REQ-NET-AUTH-018, N-50). The caller gives new random bytes for each
    /// new session.
    pub const fn from_random(random: [u8; NONCE_LEN]) -> Self {
        let mut out = [0u8; NONCE_LEN];
        let mut i = 0;
        while i < NONCE_LEN {
            out[i] = 32 + random[i] % 95;
            i += 1;
        }
        CryptoNonce(out)
    }

    /// A CryptoNonce from given bytes; `InvalidNonce` for a byte outside 32 to 126
    /// (REQ-NET-AUTH-005).
    pub const fn new(bytes: [u8; NONCE_LEN]) -> Result<Self, AuthError> {
        let n = CryptoNonce(bytes);
        if n.is_writable() {
            Ok(n)
        } else {
            Err(AuthError::InvalidNonce)
        }
    }

    /// A received CryptoNonce: any byte values are accepted (REQ-NET-AUTH-005, N-49).
    pub const fn from_received(bytes: [u8; NONCE_LEN]) -> Self {
        CryptoNonce(bytes)
    }

    /// The 16 bytes.
    pub const fn as_bytes(&self) -> &[u8; NONCE_LEN] {
        &self.0
    }

    /// True when every byte is in 32 to 126, so that the writer may write it.
    pub const fn is_writable(&self) -> bool {
        let mut i = 0;
        while i < NONCE_LEN {
            if self.0[i] < 32 || self.0[i] > 126 {
                return false;
            }
            i += 1;
        }
        true
    }
}

/// A SHA-256 Authentication Digest (Tables 18 and 19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthDigest([u8; DIGEST_LEN]);

impl AuthDigest {
    /// A digest from its 32 bytes.
    pub const fn from_bytes(bytes: [u8; DIGEST_LEN]) -> Self {
        AuthDigest(bytes)
    }

    /// The 32 bytes.
    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        &self.0
    }
}

/// Compares two digests in constant time: an XOR fold over the 32 bytes with no early exit
/// (REQ-NET-AUTH-009, -010). Use it, not `==`, for a received digest.
pub fn verify(a: &AuthDigest, b: &AuthDigest) -> bool {
    let mut acc = 0u8;
    for (x, y) in a.0.iter().zip(b.0.iter()) {
        acc |= x ^ y;
    }
    core::hint::black_box(acc) == 0
}

/// The digest of `h`, finalized in place (OPT-026). `finalize` takes the state by value: the move
/// leaves a copy of the last block (the secret or the password) in the frame of the caller, which
/// the drop does not clear. In place, the drop of `h` clears the only copy.
fn finish(h: &mut Sha256) -> [u8; DIGEST_LEN] {
    let mut out = [0u8; DIGEST_LEN];
    h.finalize_into_reset((&mut out).into());
    out
}

/// The Authentication Digest of Invitation with Authentication: SHA-256 of the 16 CryptoNonce
/// bytes and then the shared secret bytes, no separator, no terminator (6.9,
/// REQ-NET-AUTH-009, -011). `EmptySecret` for an empty secret. The secret is not copied, and
/// the hash state is cleared when it is dropped.
pub fn shared_secret_digest(nonce: &CryptoNonce, secret: &[u8]) -> Result<AuthDigest, AuthError> {
    if secret.is_empty() {
        return Err(AuthError::EmptySecret);
    }
    let mut h = Sha256::new();
    h.update(nonce.as_bytes());
    h.update(secret);
    Ok(AuthDigest(finish(&mut h)))
}

/// The Authentication Digest of Invitation with User Authentication: SHA-256 of the 16
/// CryptoNonce bytes, then the Username bytes before the first 0x00, then the password bytes
/// (6.10, REQ-NET-AUTH-010, -011). `EmptyUsername` or `EmptyPassword` for an empty value. The
/// password is not copied, and the hash state is cleared when it is dropped.
pub fn user_digest(
    nonce: &CryptoNonce,
    username: &[u8],
    password: &[u8],
) -> Result<AuthDigest, AuthError> {
    let username = NetStr::from_field(username).as_bytes();
    if username.is_empty() {
        return Err(AuthError::EmptyUsername);
    }
    if password.is_empty() {
        return Err(AuthError::EmptyPassword);
    }
    let mut h = Sha256::new();
    h.update(nonce.as_bytes());
    h.update(username);
    h.update(password);
    Ok(AuthDigest(finish(&mut h)))
}

/// True when `received` is the digest of `nonce` and `secret` (constant-time compare).
pub fn verify_shared_secret(
    nonce: &CryptoNonce,
    secret: &[u8],
    received: &AuthDigest,
) -> Result<bool, AuthError> {
    Ok(verify(&shared_secret_digest(nonce, secret)?, received))
}

/// True when `received` is the digest of `nonce`, `username` and `password` (constant-time
/// compare).
pub fn verify_user(
    nonce: &CryptoNonce,
    username: &[u8],
    password: &[u8],
    received: &AuthDigest,
) -> Result<bool, AuthError> {
    Ok(verify(&user_digest(nonce, username, password)?, received))
}

/// Authentication State of Invitation Reply: Authentication Required (Table 15,
/// REQ-NET-AUTH-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthState {
    /// 0x00 First authentication request.
    FirstRequest,
    /// 0x01 Previously provided Authentication Digest is not correct.
    DigestNotCorrect,
    /// 0x02 to 0xFF Reserved: the reader keeps the raw value and processes it as 0x00.
    Other(u8),
}

impl AuthState {
    /// The state of a byte.
    pub const fn from_bits(b: u8) -> Self {
        match b {
            0x00 => AuthState::FirstRequest,
            0x01 => AuthState::DigestNotCorrect,
            b => AuthState::Other(b),
        }
    }

    /// The byte of the state.
    pub const fn into_bits(self) -> u8 {
        match self {
            AuthState::FirstRequest => 0x00,
            AuthState::DigestNotCorrect => 0x01,
            AuthState::Other(b) => b,
        }
    }

    /// True for a value of Table 15 (the writer writes only these).
    pub const fn is_defined(self) -> bool {
        self.into_bits() <= 0x01
    }

    /// True when the Client processes the reply as a first request: 0x00 and every Reserved
    /// value (REQ-NET-AUTH-003).
    pub const fn is_first_request(self) -> bool {
        !matches!(self, AuthState::DigestNotCorrect)
    }
}

/// Authentication State of Invitation Reply: User Authentication Required (Table 17,
/// REQ-NET-AUTH-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserAuthState {
    /// 0x00 First authentication request.
    FirstRequest,
    /// 0x01 Previously provided Authentication Digest is not correct.
    DigestNotCorrect,
    /// 0x02 Previously provided username is not found.
    UsernameNotFound,
    /// 0x03 to 0xFF Reserved: the reader keeps the raw value and processes it as 0x00.
    Other(u8),
}

impl UserAuthState {
    /// The state of a byte.
    pub const fn from_bits(b: u8) -> Self {
        match b {
            0x00 => UserAuthState::FirstRequest,
            0x01 => UserAuthState::DigestNotCorrect,
            0x02 => UserAuthState::UsernameNotFound,
            b => UserAuthState::Other(b),
        }
    }

    /// The byte of the state.
    pub const fn into_bits(self) -> u8 {
        match self {
            UserAuthState::FirstRequest => 0x00,
            UserAuthState::DigestNotCorrect => 0x01,
            UserAuthState::UsernameNotFound => 0x02,
            UserAuthState::Other(b) => b,
        }
    }

    /// True for a value of Table 17 (the writer writes only these).
    pub const fn is_defined(self) -> bool {
        self.into_bits() <= 0x02
    }

    /// True when the Client processes the reply as a first request: 0x00 and every Reserved
    /// value (REQ-NET-AUTH-004).
    pub const fn is_first_request(self) -> bool {
        matches!(self, UserAuthState::FirstRequest | UserAuthState::Other(_))
    }
}

/// The fields of 0x12 and 0x13 after the header, read (REQ-NET-AUTH-041): pl < csd1 + 4 is
/// Malformed; an empty name and an empty id are accepted; pl above 40 is accepted.
fn read_challenge<'a>(
    raw: &RawCommand<'a>,
) -> Result<(CryptoNonce, NetStr<'a>, NetStr<'a>), Malformed> {
    let pl = usize::from(raw.header.len);
    let n = usize::from(raw.header.csd1);
    let id_words = pl.checked_sub(n + NONCE_WORDS).ok_or(Malformed)?;
    let nonce = raw
        .field(0, NONCE_WORDS)
        .and_then(|b| b.first_chunk::<NONCE_LEN>())
        .ok_or(Malformed)?;
    let name = raw.field(NONCE_WORDS, n).ok_or(Malformed)?;
    let id = raw.field(NONCE_WORDS + n, id_words).ok_or(Malformed)?;
    Ok((
        CryptoNonce::from_received(*nonce),
        NetStr::from_field(name),
        NetStr::from_field(id),
    ))
}

/// Writes 0x12 or 0x13 (REQ-NET-AUTH-040): pl = 4 + csd1 + id words, 0x00 fill.
fn write_challenge(
    w: &mut PayloadWriter<'_>,
    code: CommandCode,
    state: u8,
    nonce: &CryptoNonce,
    name: &EndpointName,
    id: &ProductInstanceId,
) -> Result<(), AuthError> {
    if !nonce.is_writable() {
        return Err(AuthError::InvalidNonce);
    }
    // EndpointName is 1 to 98 bytes, so csd1 is 1 to 25.
    let csd1 = string_words(name.as_str().len()) as u8;
    w.command(code.into_bits(), csd1, state, |p| {
        p.padded(nonce.as_bytes())?;
        p.string(name.as_str())?;
        p.padded(id.as_str().as_bytes())
    })?;
    Ok(())
}

/// Invitation Reply: Authentication Required (0x12, Table 14, REQ-NET-AUTH-001, -003, -040,
/// -041).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthRequired<'a> {
    /// Authentication State (raw value kept).
    pub state: AuthState,
    /// The CryptoNonce, as received.
    pub nonce: CryptoNonce,
    /// The UMP Endpoint Name of the Host.
    pub name: NetStr<'a>,
    /// The Product Instance Id of the Host.
    pub product_instance_id: NetStr<'a>,
}

impl<'a> AuthRequired<'a> {
    /// Reads the reply; pl < csd1 + 4 is Malformed (NAK 0x03).
    pub fn read(raw: &RawCommand<'a>) -> Result<Self, Malformed> {
        let (nonce, name, product_instance_id) = read_challenge(raw)?;
        Ok(AuthRequired {
            state: AuthState::from_bits(raw.header.csd2),
            nonce,
            name,
            product_instance_id,
        })
    }

    /// Writes the reply. `ReservedState` for a state not in Table 15, `InvalidNonce` for a
    /// nonce byte outside 32 to 126.
    pub fn write(
        w: &mut PayloadWriter<'_>,
        state: AuthState,
        nonce: &CryptoNonce,
        name: &EndpointName,
        id: &ProductInstanceId,
    ) -> Result<(), AuthError> {
        if !state.is_defined() {
            return Err(AuthError::ReservedState);
        }
        write_challenge(
            w,
            CommandCode::InvitationAuthRequired,
            state.into_bits(),
            nonce,
            name,
            id,
        )
    }
}

/// Invitation Reply: User Authentication Required (0x13, Table 16, REQ-NET-AUTH-002, -004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserAuthRequired<'a> {
    /// Authentication State (raw value kept).
    pub state: UserAuthState,
    /// The CryptoNonce, as received.
    pub nonce: CryptoNonce,
    /// The UMP Endpoint Name of the Host.
    pub name: NetStr<'a>,
    /// The Product Instance Id of the Host.
    pub product_instance_id: NetStr<'a>,
}

impl<'a> UserAuthRequired<'a> {
    /// Reads the reply; pl < csd1 + 4 is Malformed (NAK 0x03).
    pub fn read(raw: &RawCommand<'a>) -> Result<Self, Malformed> {
        let (nonce, name, product_instance_id) = read_challenge(raw)?;
        Ok(UserAuthRequired {
            state: UserAuthState::from_bits(raw.header.csd2),
            nonce,
            name,
            product_instance_id,
        })
    }

    /// Writes the reply. `ReservedState` for a state not in Table 17, `InvalidNonce` for a
    /// nonce byte outside 32 to 126.
    pub fn write(
        w: &mut PayloadWriter<'_>,
        state: UserAuthState,
        nonce: &CryptoNonce,
        name: &EndpointName,
        id: &ProductInstanceId,
    ) -> Result<(), AuthError> {
        if !state.is_defined() {
            return Err(AuthError::ReservedState);
        }
        write_challenge(
            w,
            CommandCode::InvitationUserAuthRequired,
            state.into_bits(),
            nonce,
            name,
            id,
        )
    }
}

/// The digest in payload words 0 to 7; `Malformed` for pl < 8.
fn read_digest(raw: &RawCommand<'_>) -> Result<AuthDigest, Malformed> {
    raw.field(0, DIGEST_WORDS)
        .and_then(|b| b.first_chunk::<DIGEST_LEN>())
        .map(|d| AuthDigest(*d))
        .ok_or(Malformed)
}

/// Invitation with Authentication (0x02, Table 18, REQ-NET-AUTH-006, -042, -043).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvitationWithAuth {
    /// The Authentication Digest.
    pub digest: AuthDigest,
}

impl InvitationWithAuth {
    /// Reads the Invitation: pl < 8 is Malformed (NAK 0x03); the words after word 7 and the
    /// Command Specific Data are ignored.
    pub fn read(raw: &RawCommand<'_>) -> Result<Self, Malformed> {
        Ok(InvitationWithAuth {
            digest: read_digest(raw)?,
        })
    }

    /// Writes the Invitation: pl 8, Command Specific Data 0.
    pub fn write(w: &mut PayloadWriter<'_>, digest: &AuthDigest) -> Result<(), AuthError> {
        w.command(CommandCode::InvitationWithAuth.into_bits(), 0, 0, |p| {
            p.padded(digest.as_bytes())
        })?;
        Ok(())
    }
}

/// Invitation with User Authentication (0x03, Table 19, REQ-NET-AUTH-007, -008, -044, -045).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvitationWithUserAuth<'a> {
    /// The Authentication Digest.
    pub digest: AuthDigest,
    /// The Username: the received bytes after the digest, up to the first 0x00 or to the end
    /// of the payload. Never empty.
    pub username: NetStr<'a>,
}

impl<'a> InvitationWithUserAuth<'a> {
    /// Reads the Invitation: pl < 8 is Malformed (NAK 0x03); an empty Username (pl 8, or a
    /// first Username byte 0x00) is also Malformed, so the Host replies NAK 0x03 and keeps
    /// the state of the Client (REQ-NET-AUTH-008, AMB-M2-124-18).
    pub fn read(raw: &RawCommand<'a>) -> Result<Self, Malformed> {
        let digest = read_digest(raw)?;
        let username = NetStr::from_field(raw.payload.get(DIGEST_LEN..).unwrap_or(&[]));
        if username.is_empty() {
            return Err(Malformed);
        }
        Ok(InvitationWithUserAuth { digest, username })
    }

    /// Writes the Invitation: Command Specific Data 0, the Username with 0x00 fill to a word
    /// boundary. `EmptyUsername`, `UsernameTooLong` (above 988 bytes) and `InvalidUsername`
    /// (not UTF-8, a 0x00 byte, a Byte Order Mark at the start) are refused.
    pub fn write(
        w: &mut PayloadWriter<'_>,
        digest: &AuthDigest,
        username: &[u8],
    ) -> Result<(), AuthError> {
        if username.is_empty() {
            return Err(AuthError::EmptyUsername);
        }
        if username.len() > MAX_USERNAME {
            return Err(AuthError::UsernameTooLong);
        }
        let name = core::str::from_utf8(username).map_err(|_| AuthError::InvalidUsername)?;
        if name.contains('\0') || name.starts_with('\u{FEFF}') {
            return Err(AuthError::InvalidUsername);
        }
        w.command(CommandCode::InvitationWithUserAuth.into_bits(), 0, 0, |p| {
            p.padded(digest.as_bytes())?;
            p.string(name)
        })?;
        Ok(())
    }
}

/// The delay of the reply to a failed authentication (6.7, 6.8, REQ-NET-AUTH-026, N-45).
///
/// One counter serves the whole Host. The delay after the n-th failure since the last success
/// is `initial * 2^(n - 1)`, at most `max`. An initial delay of 0 turns the delay off. The
/// time is the caller's clock in ms (DEC-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureDelay {
    initial_ms: u64,
    max_ms: u64,
    failures: u32,
}

impl Default for FailureDelay {
    /// 100 ms, doubled, at most 3200 ms.
    fn default() -> Self {
        FailureDelay::new(DEFAULT_INITIAL_DELAY_MS, DEFAULT_MAX_DELAY_MS)
    }
}

impl FailureDelay {
    /// A delay with the configuration values `initial_ms` and `max_ms`.
    pub const fn new(initial_ms: u64, max_ms: u64) -> Self {
        FailureDelay {
            initial_ms,
            max_ms,
            failures: 0,
        }
    }

    /// Number of failed authentications since the last success.
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// The delay for the current number of failures (0 before the first failure).
    pub const fn delay_ms(&self) -> u64 {
        if self.failures == 0 || self.initial_ms == 0 {
            return 0;
        }
        let shift = self.failures - 1;
        let d = if shift >= 64 {
            u64::MAX
        } else {
            self.initial_ms.saturating_mul(1u64 << shift)
        };
        if d > self.max_ms {
            self.max_ms
        } else {
            d
        }
    }

    /// Records one failed authentication at `now_ms` and gives the time at which the reply
    /// may be sent.
    pub fn fail(&mut self, now_ms: u64) -> u64 {
        self.failures = self.failures.saturating_add(1);
        now_ms.saturating_add(self.delay_ms())
    }

    /// Records a successful authentication: the counter goes back to 0.
    pub fn succeed(&mut self) {
        self.failures = 0;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::command::Command;
    use crate::packet::{command_packets, MaxPayload, PacketItem};
    use proptest::prelude::*;
    use std::vec::Vec;

    const NONCE_69: [u8; 16] = *b"nUWrn*@#$hjfwnkL";
    const NONCE_610: [u8; 16] = *b"XI|~=NNRVaD;XCPL";
    /// 6.9 example digest.
    const DIGEST_69: [u8; 32] = [
        0x67, 0x6E, 0xBE, 0x82, 0x58, 0x7C, 0xEC, 0xA8, 0xF8, 0x2F, 0xC3, 0x33, 0xD7, 0x87, 0x95,
        0x1E, 0xEC, 0x2B, 0x00, 0xAD, 0x31, 0x61, 0x3C, 0xC4, 0xDB, 0x18, 0xCF, 0x27, 0x37, 0x3A,
        0xFB, 0x82,
    ];
    /// 6.10 example digest.
    const DIGEST_610: [u8; 32] = [
        0x3A, 0x2D, 0x5E, 0xBE, 0xEF, 0x92, 0xE8, 0x46, 0x35, 0x35, 0xC2, 0x7F, 0xB4, 0xB7, 0xB3,
        0xE2, 0xD1, 0xAC, 0xF5, 0x7A, 0x84, 0x4C, 0x6D, 0x08, 0x08, 0xE0, 0x41, 0xC1, 0x02, 0xB7,
        0xFF, 0x1A,
    ];

    fn write(f: impl FnOnce(&mut PayloadWriter<'_>) -> Result<(), AuthError>) -> Vec<u8> {
        let mut buf = [0u8; 1400];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        f(&mut w).unwrap();
        let n = w.finish().unwrap();
        buf[..n].to_vec()
    }

    fn raw(p: &[u8]) -> RawCommand<'_> {
        match command_packets(p).unwrap().next() {
            Some(PacketItem::Command(c)) => c,
            other => panic!("{other:?}"),
        }
    }

    fn host() -> (EndpointName, ProductInstanceId) {
        (
            EndpointName::new("Host").unwrap(),
            ProductInstanceId::new("P1").unwrap(),
        )
    }

    fn payload(header: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut p = Vec::from(*b"MIDI");
        p.extend(header);
        p.extend(body);
        p
    }

    #[test]
    fn shared_secret_digest_example_6_9() {
        // REQ-NET-AUTH-009: "nUWrn*@#$hjfwnkL" + "5483".
        let n = CryptoNonce::new(NONCE_69).unwrap();
        let d = shared_secret_digest(&n, b"5483").unwrap();
        assert_eq!(d.as_bytes(), &DIGEST_69);
        assert_eq!(
            verify_shared_secret(&n, b"5483", &AuthDigest::from_bytes(DIGEST_69)),
            Ok(true)
        );
        assert_eq!(
            verify_shared_secret(&n, b"5484", &AuthDigest::from_bytes(DIGEST_69)),
            Ok(false)
        );
    }

    #[test]
    fn user_digest_example_6_10() {
        // REQ-NET-AUTH-010: "XI|~=NNRVaD;XCPL" + "Rosa" + "RPBqBno".
        let n = CryptoNonce::new(NONCE_610).unwrap();
        let d = user_digest(&n, b"Rosa", b"RPBqBno").unwrap();
        assert_eq!(d.as_bytes(), &DIGEST_610);
        // The 0x00 fill bytes of a received Username field are not hashed.
        assert_eq!(user_digest(&n, b"Rosa\0\0\0\0", b"RPBqBno").unwrap(), d);
        assert_eq!(
            verify_user(&n, b"Rosa", b"RPBqBno", &AuthDigest::from_bytes(DIGEST_610)),
            Ok(true)
        );
        assert_eq!(
            verify_user(&n, b"Rosa", b"RPBqBnx", &AuthDigest::from_bytes(DIGEST_610)),
            Ok(false)
        );
    }

    #[test]
    fn credentials_are_byte_slices_and_not_empty() {
        // REQ-NET-AUTH-011: any bytes, no terminator; empty secret or password is an error.
        let n = CryptoNonce::new(NONCE_69).unwrap();
        assert_eq!(shared_secret_digest(&n, b""), Err(AuthError::EmptySecret));
        assert_eq!(user_digest(&n, b"Rosa", b""), Err(AuthError::EmptyPassword));
        assert_eq!(user_digest(&n, b"", b"pw"), Err(AuthError::EmptyUsername));
        assert!(shared_secret_digest(&n, &[0xFF, 0x00, 0x80]).is_ok());
        assert!(shared_secret_digest(&n, "Mot de passe é".as_bytes()).is_ok());
        // A secret with a trailing 0x00 is a different secret (no terminator removed).
        assert_ne!(
            shared_secret_digest(&n, b"5483\0").unwrap(),
            shared_secret_digest(&n, b"5483").unwrap()
        );
        // No copy kept: the hash state, which holds secret bytes, is cleared when it is
        // dropped (sha2 feature `zeroize`).
        fn clears_on_drop<T: sha2::digest::zeroize::ZeroizeOnDrop>() {}
        clears_on_drop::<Sha256>();
    }

    #[test]
    fn verify_is_xor_fold_compare() {
        // REQ-NET-AUTH-009, -010: the compare gives equality for every byte position.
        let a = AuthDigest::from_bytes(DIGEST_69);
        assert!(verify(&a, &a));
        for i in 0..32 {
            let mut b = DIGEST_69;
            b[i] ^= 0x01;
            assert!(!verify(&a, &AuthDigest::from_bytes(b)));
        }
    }

    #[test]
    fn nonce_from_random_maps_into_ascii() {
        // REQ-NET-AUTH-018: b -> 32 + (b mod 95).
        let mut r = [0u8; 16];
        r[0] = 0;
        r[1] = 94;
        r[2] = 95;
        r[3] = 255;
        let n = CryptoNonce::from_random(r);
        assert_eq!(&n.as_bytes()[..4], &[32, 126, 32, 32 + 255 % 95]);
        assert!(n.is_writable());
        // Different random bytes give a different nonce for a new session.
        assert_ne!(
            CryptoNonce::from_random([1; 16]),
            CryptoNonce::from_random([2; 16])
        );
    }

    #[test]
    fn nonce_range_writer_and_reader() {
        // REQ-NET-AUTH-005: 16 bytes; the writer refuses bytes out of 32..=126, the reader
        // accepts them and hashes them as received.
        assert_eq!(CryptoNonce::new(NONCE_69).unwrap().as_bytes(), &NONCE_69);
        let mut bad = NONCE_69;
        bad[5] = 0x1F;
        assert_eq!(CryptoNonce::new(bad), Err(AuthError::InvalidNonce));
        bad[5] = 0x7F;
        assert_eq!(CryptoNonce::new(bad), Err(AuthError::InvalidNonce));
        let rx = CryptoNonce::from_received(bad);
        assert!(!rx.is_writable());
        let (name, id) = host();
        let mut buf = [0u8; 64];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        assert_eq!(
            AuthRequired::write(&mut w, AuthState::FirstRequest, &rx, &name, &id),
            Err(AuthError::InvalidNonce)
        );
        // Read a reply with out-of-range nonce bytes: accepted, kept as received.
        let mut body = Vec::from([0x00u8, 0xFF, 0x80, 0x01]);
        body.extend([b'A'; 12]);
        body.extend(*b"Host");
        let p = payload([0x12, 5, 1, 0], &body);
        let r = AuthRequired::read(&raw(&p)).unwrap();
        assert_eq!(&r.nonce.as_bytes()[..4], &[0x00, 0xFF, 0x80, 0x01]);
        let mut h = Sha256::new();
        h.update(&body[..16]);
        h.update(b"s");
        let want: [u8; 32] = h.finalize().into();
        assert_eq!(
            shared_secret_digest(&r.nonce, b"s").unwrap().as_bytes(),
            &want
        );
    }

    #[test]
    fn auth_required_vector_and_layout() {
        // REQ-NET-AUTH-001, -040: the derived test vector.
        let (name, id) = host();
        let n = CryptoNonce::new(NONCE_69).unwrap();
        let p = write(|w| AuthRequired::write(w, AuthState::FirstRequest, &n, &name, &id));
        let want: [u8; 32] = [
            0x4D, 0x49, 0x44, 0x49, 0x12, 0x06, 0x01, 0x00, 0x6E, 0x55, 0x57, 0x72, 0x6E, 0x2A,
            0x40, 0x23, 0x24, 0x68, 0x6A, 0x66, 0x77, 0x6E, 0x6B, 0x4C, 0x48, 0x6F, 0x73, 0x74,
            0x50, 0x31, 0x00, 0x00,
        ];
        assert_eq!(p, want);
        let c = raw(&p);
        assert!(matches!(Command::parse(c), Ok(Command::Auth(_))));
        let r = AuthRequired::read(&c).unwrap();
        assert_eq!(r.state, AuthState::FirstRequest);
        assert_eq!(r.nonce, n);
        assert_eq!(r.name.to_str(), Ok("Host"));
        assert_eq!(r.product_instance_id.to_str(), Ok("P1"));
    }

    #[test]
    fn auth_required_writer_fill_and_length() {
        // REQ-NET-AUTH-040: 0x00 fill, pl = 4 + csd1 + id words, csd1 up to 25, buffer too
        // small is an error.
        let name = EndpointName::new(&"n".repeat(98)).unwrap();
        let id = ProductInstanceId::new(&"i".repeat(42)).unwrap();
        let n = CryptoNonce::new(NONCE_69).unwrap();
        let p = write(|w| AuthRequired::write(w, AuthState::DigestNotCorrect, &n, &name, &id));
        assert_eq!(&p[4..8], &[0x12, 40, 25, 0x01]);
        assert_eq!(p.len(), 8 + 40 * 4);
        assert_eq!(&p[24 + 98..24 + 100], &[0, 0]);
        let name5 = EndpointName::new("MyDev").unwrap();
        let p = write(|w| AuthRequired::write(w, AuthState::FirstRequest, &n, &name5, &id));
        assert_eq!(&p[4..8], &[0x12, 4 + 2 + 11, 2, 0x00]);
        assert_eq!(&p[24..32], b"MyDev\0\0\0");
        let mut small = [0u8; 20];
        let mut w = PayloadWriter::new(&mut small, MaxPayload::default());
        assert_eq!(
            AuthRequired::write(&mut w, AuthState::FirstRequest, &n, &name5, &id),
            Err(AuthError::Command(CommandError::Packet(
                PacketError::TooLarge
            )))
        );
    }

    #[test]
    fn auth_required_reader_lengths() {
        // REQ-NET-AUTH-041: pl < csd1 + 4 is Malformed; csd1 = 0 and an empty id are
        // accepted; pl above 40 is accepted.
        let nonce = NONCE_69.to_vec();
        let p = payload([0x12, 3, 0, 0], &nonce[..12]);
        assert_eq!(AuthRequired::read(&raw(&p)), Err(Malformed));
        let p = payload([0x12, 4, 1, 0], &nonce);
        assert_eq!(AuthRequired::read(&raw(&p)), Err(Malformed));
        let p = payload([0x12, 4, 0, 0], &nonce);
        let r = AuthRequired::read(&raw(&p)).unwrap();
        assert!(r.name.is_empty() && r.product_instance_id.is_empty());
        let mut body = nonce.clone();
        body.extend(*b"Host");
        let p = payload([0x12, 5, 1, 0], &body);
        let r = AuthRequired::read(&raw(&p)).unwrap();
        assert_eq!(r.name.to_str(), Ok("Host"));
        assert!(r.product_instance_id.is_empty());
        let mut body = nonce.clone();
        body.extend(*b"Host");
        body.extend([b'x'; 4 * 50]);
        let p = payload([0x12, 55, 1, 0], &body);
        let r = AuthRequired::read(&raw(&p)).unwrap();
        assert_eq!(r.product_instance_id.as_bytes().len(), 200);
    }

    #[test]
    fn user_auth_required_vector() {
        // REQ-NET-AUTH-002: state 0x02, code 0x13, same layout as 0x12.
        let (name, id) = host();
        let n = CryptoNonce::new(NONCE_69).unwrap();
        let p =
            write(|w| UserAuthRequired::write(w, UserAuthState::UsernameNotFound, &n, &name, &id));
        let want: [u8; 32] = [
            0x4D, 0x49, 0x44, 0x49, 0x13, 0x06, 0x01, 0x02, 0x6E, 0x55, 0x57, 0x72, 0x6E, 0x2A,
            0x40, 0x23, 0x24, 0x68, 0x6A, 0x66, 0x77, 0x6E, 0x6B, 0x4C, 0x48, 0x6F, 0x73, 0x74,
            0x50, 0x31, 0x00, 0x00,
        ];
        assert_eq!(p, want);
        let r = UserAuthRequired::read(&raw(&p)).unwrap();
        assert_eq!(r.state, UserAuthState::UsernameNotFound);
        assert_eq!(r.nonce, n);
        assert_eq!(r.name.to_str(), Ok("Host"));
        assert_eq!(r.product_instance_id.to_str(), Ok("P1"));
        let p = payload([0x13, 3, 0, 0], &NONCE_69[..12]);
        assert_eq!(UserAuthRequired::read(&raw(&p)), Err(Malformed));
    }

    #[test]
    fn auth_state_value_set() {
        // REQ-NET-AUTH-003: Table 15; writer only 0x00, 0x01; reader keeps reserved raw
        // values and treats them as a first request.
        for b in 0..=255u8 {
            assert_eq!(AuthState::from_bits(b).into_bits(), b);
        }
        assert_eq!(AuthState::from_bits(1), AuthState::DigestNotCorrect);
        assert!(AuthState::from_bits(0).is_first_request());
        assert!(!AuthState::DigestNotCorrect.is_first_request());
        assert_eq!(AuthState::from_bits(2), AuthState::Other(2));
        assert!(AuthState::Other(2).is_first_request());
        let (name, id) = host();
        let n = CryptoNonce::new(NONCE_69).unwrap();
        let mut buf = [0u8; 64];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        assert_eq!(
            AuthRequired::write(&mut w, AuthState::Other(2), &n, &name, &id),
            Err(AuthError::ReservedState)
        );
        let mut p = write(|w| AuthRequired::write(w, AuthState::FirstRequest, &n, &name, &id));
        p[7] = 0x7E;
        let r = AuthRequired::read(&raw(&p)).unwrap();
        assert_eq!(r.state.into_bits(), 0x7E);
        assert!(r.state.is_first_request());
    }

    #[test]
    fn user_auth_state_value_set() {
        // REQ-NET-AUTH-004: Table 17; writer only 0x00 to 0x02; reserved values kept raw and
        // treated as a first request.
        for b in 0..=255u8 {
            assert_eq!(UserAuthState::from_bits(b).into_bits(), b);
        }
        assert_eq!(UserAuthState::from_bits(2), UserAuthState::UsernameNotFound);
        assert!(!UserAuthState::UsernameNotFound.is_first_request());
        assert!(!UserAuthState::DigestNotCorrect.is_first_request());
        assert!(UserAuthState::Other(3).is_first_request());
        let (name, id) = host();
        let n = CryptoNonce::new(NONCE_69).unwrap();
        let mut buf = [0u8; 64];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        assert_eq!(
            UserAuthRequired::write(&mut w, UserAuthState::Other(3), &n, &name, &id),
            Err(AuthError::ReservedState)
        );
        let mut p =
            write(|w| UserAuthRequired::write(w, UserAuthState::FirstRequest, &n, &name, &id));
        p[7] = 0x03;
        let r = UserAuthRequired::read(&raw(&p)).unwrap();
        assert_eq!(r.state, UserAuthState::Other(3));
        assert!(r.state.is_first_request());
    }

    #[test]
    fn invitation_with_auth_vector() {
        // REQ-NET-AUTH-006, -042: header 02 08 00 00, then the 6.9 digest.
        let p = write(|w| InvitationWithAuth::write(w, &AuthDigest::from_bytes(DIGEST_69)));
        let mut want = Vec::from(*b"MIDI");
        want.extend([0x02, 0x08, 0x00, 0x00]);
        want.extend(DIGEST_69);
        assert_eq!(p, want);
        let c = raw(&p);
        assert!(matches!(Command::parse(c), Ok(Command::Auth(_))));
        assert_eq!(
            InvitationWithAuth::read(&c).unwrap().digest.as_bytes(),
            &DIGEST_69
        );
    }

    #[test]
    fn invitation_with_auth_reader_rules() {
        // REQ-NET-AUTH-043: pl < 8 Malformed; extra words and csd ignored.
        let p = payload([0x02, 7, 0, 0], &DIGEST_69[..28]);
        assert_eq!(InvitationWithAuth::read(&raw(&p)), Err(Malformed));
        let p = payload([0x02, 0, 0, 0], &[]);
        assert_eq!(InvitationWithAuth::read(&raw(&p)), Err(Malformed));
        let mut body = DIGEST_69.to_vec();
        body.extend([9u8; 8]);
        let p = payload([0x02, 10, 0xAB, 0xCD], &body);
        assert_eq!(
            InvitationWithAuth::read(&raw(&p))
                .unwrap()
                .digest
                .as_bytes(),
            &DIGEST_69
        );
    }

    #[test]
    fn invitation_with_user_auth_vector() {
        // REQ-NET-AUTH-007, -044: header 03 09 00 00, the 6.10 digest, "Rosa".
        let p = write(|w| {
            InvitationWithUserAuth::write(w, &AuthDigest::from_bytes(DIGEST_610), b"Rosa")
        });
        let mut want = Vec::from(*b"MIDI");
        want.extend([0x03, 0x09, 0x00, 0x00]);
        want.extend(DIGEST_610);
        want.extend(*b"Rosa");
        assert_eq!(p, want);
        let c = raw(&p);
        assert!(matches!(Command::parse(c), Ok(Command::Auth(_))));
        let r = InvitationWithUserAuth::read(&c).unwrap();
        assert_eq!(r.digest.as_bytes(), &DIGEST_610);
        assert_eq!(r.username.as_bytes(), b"Rosa");
    }

    #[test]
    fn invitation_with_user_auth_writer_rules() {
        // REQ-NET-AUTH-044, -008: 0x00 fill, csd 0, limits 1..=988 bytes, UTF-8.
        let d = AuthDigest::from_bytes(DIGEST_610);
        let p = write(|w| InvitationWithUserAuth::write(w, &d, "Zoë".as_bytes()));
        assert_eq!(&p[4..8], &[0x03, 9, 0, 0]);
        assert_eq!(&p[40..], &[b'Z', b'o', 0xC3, 0xAB]);
        let p = write(|w| InvitationWithUserAuth::write(w, &d, b"Rosa1"));
        assert_eq!(&p[4..8], &[0x03, 10, 0, 0]);
        assert_eq!(&p[40..], b"Rosa1\0\0\0");
        let long = [b'u'; 988];
        let p = write(|w| InvitationWithUserAuth::write(w, &d, &long));
        assert_eq!(&p[4..8], &[0x03, 255, 0, 0]);
        let mut buf = [0u8; 1400];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        assert_eq!(
            InvitationWithUserAuth::write(&mut w, &d, b""),
            Err(AuthError::EmptyUsername)
        );
        assert_eq!(
            InvitationWithUserAuth::write(&mut w, &d, &[b'u'; 989]),
            Err(AuthError::UsernameTooLong)
        );
        assert_eq!(
            InvitationWithUserAuth::write(&mut w, &d, &[0xFF, b'a']),
            Err(AuthError::InvalidUsername)
        );
        assert_eq!(
            InvitationWithUserAuth::write(&mut w, &d, b"a\0b"),
            Err(AuthError::InvalidUsername)
        );
        assert!(w.is_empty());
    }

    #[test]
    fn invitation_with_user_auth_reader_rules() {
        // REQ-NET-AUTH-045: Username up to the first 0x00 or the end; pl < 8 Malformed.
        // REQ-NET-AUTH-008: an empty Username (pl 8 or first byte 0x00) is Malformed (NAK 0x03).
        let p = payload([0x03, 7, 0, 0], &DIGEST_610[..28]);
        assert_eq!(InvitationWithUserAuth::read(&raw(&p)), Err(Malformed));
        let p = payload([0x03, 8, 0, 0], &DIGEST_610);
        assert_eq!(InvitationWithUserAuth::read(&raw(&p)), Err(Malformed));
        let mut body = DIGEST_610.to_vec();
        body.extend([0, b'a', b'b', b'c']);
        let p = payload([0x03, 9, 0, 0], &body);
        assert_eq!(InvitationWithUserAuth::read(&raw(&p)), Err(Malformed));
        let mut body = DIGEST_610.to_vec();
        body.extend(*b"Ro\0aXXXX");
        body.extend([0u8; 4]);
        let p = payload([0x03, 11, 0xFF, 0xFF], &body);
        let r = InvitationWithUserAuth::read(&raw(&p)).unwrap();
        assert_eq!(r.username.as_bytes(), b"Ro");
        let mut body = DIGEST_610.to_vec();
        body.extend(*b"abcdefgh");
        let p = payload([0x03, 10, 0, 0], &body);
        let r = InvitationWithUserAuth::read(&raw(&p)).unwrap();
        assert_eq!(r.username.as_bytes(), b"abcdefgh");
        // The Username is a slice of the received payload (no copy).
        assert!(core::ptr::eq(r.username.as_bytes().as_ptr(), &p[40]));
    }

    #[test]
    fn failure_delay_doubles_to_maximum() {
        // REQ-NET-AUTH-026, N-45: 100, 200, 400, ..., 3200 ms; one counter; success resets.
        let mut d = FailureDelay::default();
        assert_eq!(d.delay_ms(), 0);
        let mut now = 1_000u64;
        for want in [100u64, 200, 400, 800, 1600, 3200, 3200, 3200] {
            let due = d.fail(now);
            assert_eq!(due, now + want);
            now = due;
        }
        assert_eq!(d.failures(), 8);
        d.succeed();
        assert_eq!((d.failures(), d.delay_ms()), (0, 0));
        assert_eq!(d.fail(5), 105);
        // Initial delay 0: off.
        let mut off = FailureDelay::new(0, 3200);
        assert_eq!(off.fail(7), 7);
        // Many failures and a large time do not overflow.
        let mut big = FailureDelay::new(u64::MAX / 2, u64::MAX);
        for _ in 0..200 {
            big.fail(u64::MAX - 1);
        }
        assert_eq!(big.delay_ms(), u64::MAX);
        assert_eq!(big.fail(u64::MAX - 1), u64::MAX);
    }

    #[test]
    fn failure_delay_keeps_no_host_wide_due_time() {
        // L3: the due time is a return value only; the per-entry due time lives in the
        // handshake. Two failures at different times give two independent due times.
        let mut d = FailureDelay::new(100, 3200);
        let first = d.fail(0);
        let second = d.fail(10);
        assert_eq!((first, second), (100, 210));
        // The value is only the counter and the configuration.
        assert_eq!(d, {
            let mut e = FailureDelay::new(100, 3200);
            e.fail(999);
            e.fail(0);
            e
        });
    }

    #[test]
    fn prop_digests_match_one_shot_sha256() {
        // REQ-NET-AUTH-009, -010: the concatenation, with no separator.
        proptest!(|(r in any::<[u8; 16]>(),
                    s in proptest::collection::vec(any::<u8>(), 1..80),
                    u in proptest::collection::vec(1u8..=255, 1..40),
                    pw in proptest::collection::vec(any::<u8>(), 1..40))| {
            let n = CryptoNonce::from_random(r);
            let mut cat = n.as_bytes().to_vec();
            cat.extend(&s);
            let want: [u8; 32] = Sha256::digest(&cat).into();
            prop_assert_eq!(*shared_secret_digest(&n, &s).unwrap().as_bytes(), want);
            let mut cat = n.as_bytes().to_vec();
            cat.extend(&u);
            cat.extend(&pw);
            let want: [u8; 32] = Sha256::digest(&cat).into();
            prop_assert_eq!(*user_digest(&n, &u, &pw).unwrap().as_bytes(), want);
        });
    }

    #[test]
    fn prop_nonce_from_random_in_range() {
        // REQ-NET-AUTH-018, -005.
        proptest!(|(r in any::<[u8; 16]>())| {
            let n = CryptoNonce::from_random(r);
            prop_assert!(n.is_writable());
            for (b, x) in n.as_bytes().iter().zip(r) {
                prop_assert_eq!(*b, 32 + x % 95);
            }
        });
    }

    #[test]
    fn prop_verify_equals_eq() {
        proptest!(|(a in any::<[u8; 32]>(), b in any::<[u8; 32]>())| {
            let (a, b) = (AuthDigest::from_bytes(a), AuthDigest::from_bytes(b));
            prop_assert_eq!(verify(&a, &b), a == b);
            prop_assert!(verify(&a, &a));
        });
    }

    #[test]
    fn prop_user_auth_round_trip() {
        proptest!(|(d in any::<[u8; 32]>(), u in "[^\u{0}\u{FEFF}]{1,100}")| {
            let digest = AuthDigest::from_bytes(d);
            let p = write(|w| InvitationWithUserAuth::write(w, &digest, u.as_bytes()));
            let r = InvitationWithUserAuth::read(&raw(&p)).unwrap();
            prop_assert_eq!(r.digest, digest);
            prop_assert_eq!(r.username.as_bytes(), u.as_bytes());
        });
    }

    #[test]
    fn prop_readers_never_panic() {
        // Nothing panics on received bytes.
        proptest!(|(code in prop_oneof![Just(2u8), Just(3u8), Just(0x12u8), Just(0x13u8)],
                    csd1 in any::<u8>(), csd2 in any::<u8>(),
                    body in proptest::collection::vec(any::<u8>(), 0..300))| {
            let words = body.len() / 4;
            let p = payload([code, words as u8, csd1, csd2], &body[..words * 4]);
            if let Some(PacketItem::Command(c)) = command_packets(&p).unwrap().next() {
                let _ = AuthRequired::read(&c);
                let _ = UserAuthRequired::read(&c);
                let _ = InvitationWithAuth::read(&c);
                let _ = InvitationWithUserAuth::read(&c);
            }
        });
    }
}
