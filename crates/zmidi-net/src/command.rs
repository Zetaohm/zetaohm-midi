//! The Commands of Network MIDI 2.0 (M2-124-UM v1.0.1 Table 8 and Tables 10 to 32), except
//! the content of the authentication Commands (module Auth, Cargo feature `auth`).
//!
//! - Readers work on a [`RawCommand`]; writers write into a [`PayloadWriter`]. Reserved fields
//!   are written as 0 and ignored when read; extra payload words are ignored
//!   (REQ-NET-PKT-010, -011, -013).
//! - Codes 0x02, 0x03, 0x12 and 0x13 always parse to [`Command::Auth`] with the header only,
//!   with and without the feature `auth`, and never get NAK 0x01 (REQ-NET-AUTH-012, N-06).
//! - No state machine: the Session, Stream and Handshake modules decide what a Command means
//!   in a state. Nothing panics on received bytes.

use crate::identity::{EndpointName, ProductInstanceId};
use crate::packet::{string_words, Header, NetStr, PacketError, PayloadWriter, RawCommand};
use zmidi_ump::MessageType;

/// Error of a Command writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// The payload writer refused (does not fit, too large).
    #[error("packet: {0}")]
    Packet(#[from] PacketError),
    /// A Text Message longer than its limit (1016 bytes for NAK, 1020 for Bye).
    #[error("text too long")]
    TextTooLong,
    /// A Text Message that starts with U+FEFF or contains U+0000 (REQ-NET-NAK-002,
    /// REQ-NET-BYE-002).
    #[error("invalid text")]
    InvalidText,
    /// A reason value that the writer may not write: not in its table, or not for this role
    /// (REQ-NET-BYE-003, -004, REQ-NET-NAK-003).
    #[error("reason not allowed")]
    ReasonNotAllowed,
    /// UMP words that do not end at a message boundary, or more than 64 words
    /// (REQ-NET-DATA-001, -006).
    #[error("invalid UMP data")]
    InvalidUmpData,
}

/// A received Command that does not follow its table: the endpoint replies NAK 0x03
/// (REQ-NET-NAK-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Command Malformed")]
pub struct Malformed;

/// Which side sends a Command (Table 8, REQ-NET-CMD-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Client to Host.
    ClientToHost,
    /// Host to Client.
    HostToClient,
    /// Either side.
    Both,
}

/// The role of an endpoint in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// The UDP Host: it accepts Invitations.
    Host,
    /// The UDP Client: it sends Invitations.
    Client,
}

impl Role {
    /// True when a Command of `direction` may come from a peer of this role.
    pub const fn may_send(self, direction: Direction) -> bool {
        matches!(
            (self, direction),
            (_, Direction::Both)
                | (Role::Client, Direction::ClientToHost)
                | (Role::Host, Direction::HostToClient)
        )
    }
}

/// The states of a session (6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionState {
    /// No session.
    Idle,
    /// The Client sent an Invitation (Client), or the Host replied Pending (Host).
    PendingInvitation,
    /// The Host asked for authentication.
    AuthenticationRequired,
    /// The session is open.
    EstablishedSession,
    /// A Session Reset waits for its reply.
    PendingSessionReset,
    /// A Bye waits for its reply.
    PendingBye,
}

/// The Command Codes of Table 8 (REQ-NET-CMD-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandCode {
    /// 0xFF UMP Data.
    UmpData,
    /// 0x01 Invitation.
    Invitation,
    /// 0x02 Invitation with Authentication.
    InvitationWithAuth,
    /// 0x03 Invitation with User Authentication.
    InvitationWithUserAuth,
    /// 0x10 Invitation Reply: Accepted.
    InvitationAccepted,
    /// 0x11 Invitation Reply: Pending.
    InvitationPending,
    /// 0x12 Invitation Reply: Authentication Required.
    InvitationAuthRequired,
    /// 0x13 Invitation Reply: User Authentication Required.
    InvitationUserAuthRequired,
    /// 0x20 Ping.
    Ping,
    /// 0x21 Ping Reply.
    PingReply,
    /// 0x80 Retransmit Request.
    RetransmitRequest,
    /// 0x81 Retransmit Error.
    RetransmitError,
    /// 0x82 Session Reset.
    SessionReset,
    /// 0x83 Session Reset Reply.
    SessionResetReply,
    /// 0x8F NAK.
    Nak,
    /// 0xF0 Bye.
    Bye,
    /// 0xF1 Bye Reply.
    ByeReply,
    /// Any other code, with its value.
    Unknown(u8),
}

impl CommandCode {
    /// The code of a byte.
    pub const fn from_bits(b: u8) -> Self {
        match b {
            0xFF => CommandCode::UmpData,
            0x01 => CommandCode::Invitation,
            0x02 => CommandCode::InvitationWithAuth,
            0x03 => CommandCode::InvitationWithUserAuth,
            0x10 => CommandCode::InvitationAccepted,
            0x11 => CommandCode::InvitationPending,
            0x12 => CommandCode::InvitationAuthRequired,
            0x13 => CommandCode::InvitationUserAuthRequired,
            0x20 => CommandCode::Ping,
            0x21 => CommandCode::PingReply,
            0x80 => CommandCode::RetransmitRequest,
            0x81 => CommandCode::RetransmitError,
            0x82 => CommandCode::SessionReset,
            0x83 => CommandCode::SessionResetReply,
            0x8F => CommandCode::Nak,
            0xF0 => CommandCode::Bye,
            0xF1 => CommandCode::ByeReply,
            b => CommandCode::Unknown(b),
        }
    }

    /// The byte of the code.
    pub const fn into_bits(self) -> u8 {
        match self {
            CommandCode::UmpData => 0xFF,
            CommandCode::Invitation => 0x01,
            CommandCode::InvitationWithAuth => 0x02,
            CommandCode::InvitationWithUserAuth => 0x03,
            CommandCode::InvitationAccepted => 0x10,
            CommandCode::InvitationPending => 0x11,
            CommandCode::InvitationAuthRequired => 0x12,
            CommandCode::InvitationUserAuthRequired => 0x13,
            CommandCode::Ping => 0x20,
            CommandCode::PingReply => 0x21,
            CommandCode::RetransmitRequest => 0x80,
            CommandCode::RetransmitError => 0x81,
            CommandCode::SessionReset => 0x82,
            CommandCode::SessionResetReply => 0x83,
            CommandCode::Nak => 0x8F,
            CommandCode::Bye => 0xF0,
            CommandCode::ByeReply => 0xF1,
            CommandCode::Unknown(b) => b,
        }
    }

    /// The direction of Table 8 (REQ-NET-CMD-004).
    pub const fn direction(self) -> Direction {
        match self {
            CommandCode::Invitation
            | CommandCode::InvitationWithAuth
            | CommandCode::InvitationWithUserAuth => Direction::ClientToHost,
            CommandCode::InvitationAccepted
            | CommandCode::InvitationPending
            | CommandCode::InvitationAuthRequired
            | CommandCode::InvitationUserAuthRequired => Direction::HostToClient,
            _ => Direction::Both,
        }
    }
}

/// A 16-bit Sequence Number with serial arithmetic (RFC 1982): it wraps after 0xFFFF and
/// compares in the half range (N-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SeqNum(pub u16);

impl SeqNum {
    /// The next number.
    pub const fn next(self) -> SeqNum {
        SeqNum(self.0.wrapping_add(1))
    }

    /// The number `n` after this one.
    pub const fn add(self, n: u16) -> SeqNum {
        SeqNum(self.0.wrapping_add(n))
    }

    /// The signed distance from `self` to `other`: positive when `other` is after `self`.
    pub const fn distance_to(self, other: SeqNum) -> i32 {
        other.0.wrapping_sub(self.0) as i16 as i32
    }
}

/// The Capabilities of an Invitation (Table 11, REQ-NET-INV-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Capabilities {
    /// D0: the Client supports sending Invitation with Authentication.
    pub auth: bool,
    /// D1: the Client supports sending Invitation with User Authentication.
    pub user_auth: bool,
}

impl Capabilities {
    /// The bits; D2 to D7 are 0.
    pub const fn into_bits(self) -> u8 {
        (self.auth as u8) | ((self.user_auth as u8) << 1)
    }

    /// The flags; D2 to D7 are ignored.
    pub const fn from_bits(b: u8) -> Self {
        Capabilities {
            auth: b & 0x01 != 0,
            user_auth: b & 0x02 != 0,
        }
    }
}

macro_rules! reason {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $c:ident = $v:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub u8);

        impl $name {
            $($(#[$vm])* pub const $c: $name = $name($v);)*

            /// True for a value of the table.
            pub const fn is_defined(self) -> bool {
                matches!(self.0, $($v)|*)
            }
        }
    };
}

reason!(
    /// NAK Reason (Table 25, REQ-NET-NAK-003). The reader keeps any raw value; a value not
    /// in the table counts as Other.
    NakReason {
        /// 0x00 Other.
        OTHER = 0x00,
        /// 0x01 Command Not Supported.
        COMMAND_NOT_SUPPORTED = 0x01,
        /// 0x02 Command Not Expected.
        COMMAND_NOT_EXPECTED = 0x02,
        /// 0x03 Command Malformed.
        COMMAND_MALFORMED = 0x03,
        /// 0x20 Bad Ping Reply.
        BAD_PING_REPLY = 0x20,
    }
);

reason!(
    /// Bye Reason (Table 27, REQ-NET-BYE-003). The reader keeps any raw value; a value not in
    /// the table counts as Unknown or Undefined.
    ByeReason {
        /// 0x00 Unknown or Undefined.
        UNKNOWN = 0x00,
        /// 0x01 User terminated session.
        USER_TERMINATED = 0x01,
        /// 0x02 Power Down.
        POWER_DOWN = 0x02,
        /// 0x03 Too Many Missing UMP Packets.
        TOO_MANY_MISSING = 0x03,
        /// 0x04 Timeout.
        TIMEOUT = 0x04,
        /// 0x05 Session Not Established.
        SESSION_NOT_ESTABLISHED = 0x05,
        /// 0x06 No Pending Session.
        NO_PENDING_SESSION = 0x06,
        /// 0x07 Protocol Error.
        PROTOCOL_ERROR = 0x07,
        /// 0x40 Invitation Failed: too many opened sessions.
        TOO_MANY_SESSIONS = 0x40,
        /// 0x41 Invitation With Authentication Rejected.
        AUTH_INVITATION_REJECTED = 0x41,
        /// 0x42 Invitation Rejected: user did not accept session.
        USER_DID_NOT_ACCEPT = 0x42,
        /// 0x43 Invitation Rejected: authentication failed.
        AUTHENTICATION_FAILED = 0x43,
        /// 0x44 Invitation Rejected: username not found.
        USERNAME_NOT_FOUND = 0x44,
        /// 0x45 No Matching Authentication Method.
        NO_MATCHING_METHOD = 0x45,
        /// 0x80 Invitation Canceled.
        INVITATION_CANCELED = 0x80,
    }
);

reason!(
    /// Retransmit Error Reason (Table 32, REQ-NET-RTX-002, -014). The reader keeps any raw
    /// value; a value not in the table counts as Unknown.
    RetransmitErrorReason {
        /// 0x00 Unknown.
        UNKNOWN = 0x00,
        /// 0x01 The transmit buffer does not hold the requested UMP Data Command.
        NOT_IN_BUFFER = 0x01,
    }
);

impl ByeReason {
    /// True when an endpoint of `role` may write this reason (REQ-NET-BYE-004): 0x40 to 0x45
    /// only from a Host, 0x80 only from a Client.
    pub const fn allowed_for(self, role: Role) -> bool {
        match role {
            Role::Client => !matches!(self.0, 0x40..=0x45),
            Role::Host => self.0 != 0x80,
        }
    }
}

/// Largest NAK Text Message, in bytes (REQ-NET-NAK-002).
pub const MAX_NAK_TEXT: usize = 1016;
/// Largest Bye Text Message, in bytes (REQ-NET-BYE-002).
pub const MAX_BYE_TEXT: usize = 1020;
/// Largest UMP Data payload that the writer writes, in words (REQ-NET-DATA-001).
pub const MAX_UMP_DATA_WORDS: usize = 64;

fn check_text(text: &str, max: usize) -> Result<(), CommandError> {
    if text.len() > max {
        return Err(CommandError::TextTooLong);
    }
    if text.starts_with('\u{FEFF}') || text.contains('\0') {
        return Err(CommandError::InvalidText);
    }
    Ok(())
}

/// Invitation (0x01, Table 10, REQ-NET-INV-001, -031, -032).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invitation<'a> {
    /// The UMP Endpoint Name of the Client.
    pub name: NetStr<'a>,
    /// The Product Instance Id of the Client.
    pub product_instance_id: NetStr<'a>,
    /// The Capabilities.
    pub capabilities: Capabilities,
}

/// The name and id fields of Invitation and of Invitation Reply: Accepted and Pending.
fn name_and_id<'a>(raw: &RawCommand<'a>) -> Result<(NetStr<'a>, NetStr<'a>), Malformed> {
    let pl = usize::from(raw.header.len);
    let n = usize::from(raw.header.csd1);
    if n > pl {
        return Err(Malformed);
    }
    let name = raw.field(0, n).ok_or(Malformed)?;
    let id = raw.field(n, pl - n).ok_or(Malformed)?;
    Ok((NetStr::from_field(name), NetStr::from_field(id)))
}

fn write_name_and_id(
    w: &mut PayloadWriter<'_>,
    code: CommandCode,
    csd2: u8,
    name: &EndpointName,
    id: &ProductInstanceId,
) -> Result<(), CommandError> {
    let n = string_words(name.as_str().len()) as u8;
    w.command(code.into_bits(), n, csd2, |p| {
        p.string(name.as_str())?;
        p.padded(id.as_str().as_bytes())
    })?;
    Ok(())
}

impl<'a> Invitation<'a> {
    /// Reads an Invitation. Values above the Table 10 ranges are accepted; csd1 above pl is
    /// Malformed.
    pub fn read(raw: &RawCommand<'a>) -> Result<Self, Malformed> {
        let (name, product_instance_id) = name_and_id(raw)?;
        Ok(Invitation {
            name,
            product_instance_id,
            capabilities: Capabilities::from_bits(raw.header.csd2),
        })
    }

    /// Writes an Invitation.
    pub fn write(
        w: &mut PayloadWriter<'_>,
        name: &EndpointName,
        id: &ProductInstanceId,
        capabilities: Capabilities,
    ) -> Result<(), CommandError> {
        write_name_and_id(
            w,
            CommandCode::Invitation,
            capabilities.into_bits(),
            name,
            id,
        )
    }
}

/// Invitation Reply: Accepted (0x10) or Pending (0x11) (Tables 12 and 13, REQ-NET-INV-003,
/// -004, -033, -034).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvitationReply<'a> {
    /// True for Pending, false for Accepted.
    pub pending: bool,
    /// The UMP Endpoint Name of the Host.
    pub name: NetStr<'a>,
    /// The Product Instance Id of the Host.
    pub product_instance_id: NetStr<'a>,
}

impl<'a> InvitationReply<'a> {
    /// Reads an Accepted or Pending reply; csd2 is ignored, an empty name or id is accepted,
    /// csd1 above pl is Malformed.
    pub fn read(raw: &RawCommand<'a>, pending: bool) -> Result<Self, Malformed> {
        let (name, product_instance_id) = name_and_id(raw)?;
        Ok(InvitationReply {
            pending,
            name,
            product_instance_id,
        })
    }

    /// Writes an Accepted or Pending reply with csd2 = 0.
    pub fn write(
        w: &mut PayloadWriter<'_>,
        pending: bool,
        name: &EndpointName,
        id: &ProductInstanceId,
    ) -> Result<(), CommandError> {
        let code = if pending {
            CommandCode::InvitationPending
        } else {
            CommandCode::InvitationAccepted
        };
        write_name_and_id(w, code, 0, name, id)
    }
}

/// Writes a Command with no Command Specific Data and one payload word: Ping and Ping Reply.
fn write_one_word(
    w: &mut PayloadWriter<'_>,
    code: CommandCode,
    word: u32,
) -> Result<(), CommandError> {
    w.command(code.into_bits(), 0, 0, |p| p.word(word))?;
    Ok(())
}

/// Writes a Command of one header word with Command Specific Data 0 and no payload.
pub fn write_empty(w: &mut PayloadWriter<'_>, code: CommandCode) -> Result<(), CommandError> {
    w.command(code.into_bits(), 0, 0, |_| Ok(()))?;
    Ok(())
}

/// Reads the Ping Id of Ping or Ping Reply: pl >= 1, extra words ignored (REQ-NET-PING-001,
/// -002).
pub fn read_ping_id(raw: &RawCommand<'_>) -> Result<u32, Malformed> {
    raw.word(0).ok_or(Malformed)
}

/// Writes Ping (0x20) with a Ping Id (REQ-NET-PING-001).
pub fn write_ping(w: &mut PayloadWriter<'_>, id: u32) -> Result<(), CommandError> {
    write_one_word(w, CommandCode::Ping, id)
}

/// Writes Ping Reply (0x21) with the Ping Id of the Ping (REQ-NET-PING-002).
pub fn write_ping_reply(w: &mut PayloadWriter<'_>, id: u32) -> Result<(), CommandError> {
    write_one_word(w, CommandCode::PingReply, id)
}

/// NAK (0x8F, Table 24, REQ-NET-NAK-001, -007, -009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nak<'a> {
    /// NAK Reason (raw value kept).
    pub reason: NakReason,
    /// The header word of the Command that the peer refused.
    pub nak_header: Header,
    /// The Text Message.
    pub text: NetStr<'a>,
}

impl<'a> Nak<'a> {
    /// Reads a NAK; pl = 0 is Malformed.
    pub fn read(raw: &RawCommand<'a>) -> Result<Self, Malformed> {
        let h = raw.field(0, 1).ok_or(Malformed)?;
        let text = raw.payload.get(4..).unwrap_or(&[]);
        Ok(Nak {
            reason: NakReason(raw.header.csd1),
            nak_header: Header::from_bytes([h[0], h[1], h[2], h[3]]),
            text: NetStr::from_field(text),
        })
    }

    /// Writes a NAK with the received header word unchanged and the caller's text (empty
    /// for a NAK that the crate writes by itself, REQ-NET-NAK-006).
    pub fn write(
        w: &mut PayloadWriter<'_>,
        reason: NakReason,
        nak_header: Header,
        text: &str,
    ) -> Result<(), CommandError> {
        if !reason.is_defined() {
            return Err(CommandError::ReasonNotAllowed);
        }
        check_text(text, MAX_NAK_TEXT)?;
        w.command(CommandCode::Nak.into_bits(), reason.0, 0, |p| {
            p.word(nak_header.word())?;
            p.string(text)
        })?;
        Ok(())
    }
}

/// Bye (0xF0, Table 26, REQ-NET-BYE-001 to -004, -012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bye<'a> {
    /// Bye Reason (raw value kept).
    pub reason: ByeReason,
    /// The Text Message.
    pub text: NetStr<'a>,
}

impl<'a> Bye<'a> {
    /// Reads a Bye: every pl and every reason value is accepted.
    pub fn read(raw: &RawCommand<'a>) -> Self {
        Bye {
            reason: ByeReason(raw.header.csd1),
            text: NetStr::from_field(raw.payload),
        }
    }

    /// Writes a Bye; the reason must be in Table 27 and allowed for `role`.
    pub fn write(
        w: &mut PayloadWriter<'_>,
        role: Role,
        reason: ByeReason,
        text: &str,
    ) -> Result<(), CommandError> {
        if !reason.is_defined() || !reason.allowed_for(role) {
            return Err(CommandError::ReasonNotAllowed);
        }
        check_text(text, MAX_BYE_TEXT)?;
        w.command(CommandCode::Bye.into_bits(), reason.0, 0, |p| {
            p.string(text)
        })?;
        Ok(())
    }
}

/// One UMP message of a UMP Data Command: 1 to 4 words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UmpMessage {
    words: [u32; 4],
    len: usize,
}

impl UmpMessage {
    /// The words of the message.
    pub fn words(&self) -> &[u32] {
        &self.words[..self.len]
    }
}

/// UMP Data (0xFF, Table 29, REQ-NET-DATA-001, -005, -006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UmpData<'a> {
    /// The Sequence Number.
    pub seq: SeqNum,
    payload: &'a [u8],
}

impl<'a> UmpData<'a> {
    /// Reads a UMP Data Command: pl from 0 to 255 is accepted (N-14).
    pub fn read(raw: &RawCommand<'a>) -> Self {
        UmpData {
            seq: SeqNum(raw.header.csd()),
            payload: raw.payload,
        }
    }

    /// True for a zero length Command (an idle declaration, 7.2.1).
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    /// The UMP messages, in payload order.
    pub fn messages(&self) -> UmpMessages<'a> {
        UmpMessages {
            rest: self.payload,
            truncated: false,
        }
    }

    /// The payload words.
    pub fn words(&self) -> impl Iterator<Item = u32> + 'a {
        self.payload
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_be_bytes(*b))
    }

    /// Writes a UMP Data Command with whole UMP messages, at most 64 words; on an error
    /// nothing is written (REQ-NET-DATA-001, -006).
    pub fn write(
        w: &mut PayloadWriter<'_>,
        seq: SeqNum,
        words: &[u32],
    ) -> Result<(), CommandError> {
        if words.len() > MAX_UMP_DATA_WORDS || !whole_messages(words) {
            return Err(CommandError::InvalidUmpData);
        }
        let [a, b] = seq.0.to_be_bytes();
        w.command(CommandCode::UmpData.into_bits(), a, b, |p| {
            words.iter().try_for_each(|x| p.word(*x))
        })?;
        Ok(())
    }
}

/// The size in words of the UMP message that starts with `first` (M2-104, zmidi-ump).
pub fn message_words(first: u32) -> usize {
    MessageType::from_bits((first >> 28) as u8).word_count()
}

/// True when `words` end at a message boundary.
pub fn whole_messages(words: &[u32]) -> bool {
    let mut i = 0;
    while i < words.len() {
        i += message_words(words[i]);
    }
    i == words.len()
}

/// The UMP messages of a UMP Data Command. A payload that ends inside a message gives the
/// whole messages before it; then [`UmpMessages::truncated`] is true and the endpoint sends
/// NAK 0x03 (REQ-NET-DATA-006).
#[derive(Debug, Clone)]
pub struct UmpMessages<'a> {
    rest: &'a [u8],
    truncated: bool,
}

impl UmpMessages<'_> {
    /// True after the iterator found a message cut at the end of the payload.
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

impl Iterator for UmpMessages<'_> {
    type Item = UmpMessage;

    fn next(&mut self) -> Option<UmpMessage> {
        let first = self.rest.first_chunk::<4>()?;
        let len = message_words(u32::from_be_bytes(*first));
        let Some(bytes) = self.rest.get(..len * 4) else {
            self.truncated = true;
            self.rest = &[];
            return None;
        };
        let mut words = [0u32; 4];
        for (w, b) in words.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *w = u32::from_be_bytes(*b);
        }
        self.rest = &self.rest[len * 4..];
        Some(UmpMessage { words, len })
    }
}

/// Retransmit Request (0x80, Table 30, REQ-NET-RTX-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetransmitRequest {
    /// The first requested Sequence Number.
    pub seq: SeqNum,
    /// Number of UMP Commands; 0 means all from `seq`.
    pub count: u16,
}

impl RetransmitRequest {
    /// Reads a request; pl = 0 is Malformed.
    pub fn read(raw: &RawCommand<'_>) -> Result<Self, Malformed> {
        let w = raw.word(0).ok_or(Malformed)?;
        Ok(RetransmitRequest {
            seq: SeqNum(raw.header.csd()),
            count: (w >> 16) as u16,
        })
    }

    /// Writes the request.
    pub fn write(&self, w: &mut PayloadWriter<'_>) -> Result<(), CommandError> {
        let [a, b] = self.seq.0.to_be_bytes();
        w.command(CommandCode::RetransmitRequest.into_bits(), a, b, |p| {
            p.word(u32::from(self.count) << 16)
        })?;
        Ok(())
    }
}

/// Retransmit Error (0x81, Tables 31 and 32, REQ-NET-RTX-002, -014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetransmitError {
    /// Error Reason (raw value kept).
    pub reason: RetransmitErrorReason,
    /// The Sequence Number field.
    pub seq: SeqNum,
}

impl RetransmitError {
    /// Reads the error; pl = 0 is Malformed.
    pub fn read(raw: &RawCommand<'_>) -> Result<Self, Malformed> {
        let w = raw.word(0).ok_or(Malformed)?;
        Ok(RetransmitError {
            reason: RetransmitErrorReason(raw.header.csd1),
            seq: SeqNum((w >> 16) as u16),
        })
    }

    /// Writes the error with the Reserved fields 0.
    pub fn write(&self, w: &mut PayloadWriter<'_>) -> Result<(), CommandError> {
        if !self.reason.is_defined() {
            return Err(CommandError::ReasonNotAllowed);
        }
        w.command(
            CommandCode::RetransmitError.into_bits(),
            self.reason.0,
            0,
            |p| p.word(u32::from(self.seq.0) << 16),
        )?;
        Ok(())
    }
}

/// One received Command, typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command<'a> {
    /// 0xFF.
    UmpData(UmpData<'a>),
    /// 0x01.
    Invitation(Invitation<'a>),
    /// 0x10 and 0x11.
    InvitationReply(InvitationReply<'a>),
    /// 0x20, with the Ping Id.
    Ping(u32),
    /// 0x21, with the Ping Id.
    PingReply(u32),
    /// 0x80.
    RetransmitRequest(RetransmitRequest),
    /// 0x81.
    RetransmitError(RetransmitError),
    /// 0x82.
    SessionReset,
    /// 0x83.
    SessionResetReply,
    /// 0x8F.
    Nak(Nak<'a>),
    /// 0xF0.
    Bye(Bye<'a>),
    /// 0xF1.
    ByeReply,
    /// 0x02, 0x03, 0x12 and 0x13: the raw Command; the module Auth reads the payload when
    /// the feature `auth` is on (N-06).
    Auth(RawCommand<'a>),
    /// A code that is not in Table 8: the endpoint replies NAK 0x01 (REQ-NET-CMD-002).
    Unknown(RawCommand<'a>),
}

impl<'a> Command<'a> {
    /// Types one received Command.
    pub fn parse(raw: RawCommand<'a>) -> Result<Self, Malformed> {
        Ok(match CommandCode::from_bits(raw.header.code) {
            CommandCode::UmpData => Command::UmpData(UmpData::read(&raw)),
            CommandCode::Invitation => Command::Invitation(Invitation::read(&raw)?),
            CommandCode::InvitationAccepted => {
                Command::InvitationReply(InvitationReply::read(&raw, false)?)
            }
            CommandCode::InvitationPending => {
                Command::InvitationReply(InvitationReply::read(&raw, true)?)
            }
            CommandCode::InvitationWithAuth
            | CommandCode::InvitationWithUserAuth
            | CommandCode::InvitationAuthRequired
            | CommandCode::InvitationUserAuthRequired => Command::Auth(raw),
            CommandCode::Ping => Command::Ping(read_ping_id(&raw)?),
            CommandCode::PingReply => Command::PingReply(read_ping_id(&raw)?),
            CommandCode::RetransmitRequest => {
                Command::RetransmitRequest(RetransmitRequest::read(&raw)?)
            }
            CommandCode::RetransmitError => Command::RetransmitError(RetransmitError::read(&raw)?),
            CommandCode::SessionReset => Command::SessionReset,
            CommandCode::SessionResetReply => Command::SessionResetReply,
            CommandCode::Nak => Command::Nak(Nak::read(&raw)?),
            CommandCode::Bye => Command::Bye(Bye::read(&raw)),
            CommandCode::ByeReply => Command::ByeReply,
            CommandCode::Unknown(_) => Command::Unknown(raw),
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::packet::{command_packets, MaxPayload, PacketItem};
    use proptest::prelude::*;
    use std::vec::Vec;

    fn one(p: &[u8]) -> Command<'_> {
        let mut it = command_packets(p).unwrap();
        let Some(PacketItem::Command(c)) = it.next() else {
            panic!()
        };
        Command::parse(c).unwrap()
    }

    fn write(f: impl FnOnce(&mut PayloadWriter<'_>) -> Result<(), CommandError>) -> Vec<u8> {
        let mut buf = [0u8; 1400];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        f(&mut w).unwrap();
        let n = w.finish().unwrap();
        buf[..n].to_vec()
    }

    fn raw(bytes: &[u8]) -> Vec<u8> {
        let mut p = Vec::from(*b"MIDI");
        p.extend_from_slice(bytes);
        p
    }

    #[test]
    fn command_codes() {
        // REQ-NET-CMD-001, -004.
        for b in 0..=255u8 {
            assert_eq!(CommandCode::from_bits(b).into_bits(), b);
        }
        assert_eq!(CommandCode::from_bits(0x8F), CommandCode::Nak);
        assert_eq!(CommandCode::from_bits(0x30), CommandCode::Unknown(0x30));
        assert_eq!(CommandCode::Invitation.direction(), Direction::ClientToHost);
        assert_eq!(
            CommandCode::InvitationUserAuthRequired.direction(),
            Direction::HostToClient
        );
        assert_eq!(CommandCode::Ping.direction(), Direction::Both);
        assert!(Role::Client.may_send(Direction::ClientToHost));
        assert!(!Role::Host.may_send(Direction::ClientToHost));
    }

    #[test]
    fn invitation_vector_and_rules() {
        // REQ-NET-VEC-001, REQ-NET-INV-001, -002, -031, -032, REQ-NET-PKT-006.
        let a11 = [
            0x4D, 0x49, 0x44, 0x49, 0x01, 0x04, 0x02, 0x00, 0x4D, 0x79, 0x44, 0x65, 0x76, 0, 0, 0,
            0x38, 0x73, 0x68, 0x59, 0x65, 0x33, 0x68, 0x35,
        ];
        let name = EndpointName::new("MyDev").unwrap();
        let id = ProductInstanceId::new("8shYe3h5").unwrap();
        let p = write(|w| Invitation::write(w, &name, &id, Capabilities::default()));
        assert_eq!(p, a11);
        let Command::Invitation(i) = one(&a11) else {
            panic!()
        };
        assert_eq!(i.name.to_str(), Ok("MyDev"));
        assert_eq!(i.product_instance_id.to_str(), Ok("8shYe3h5"));
        assert_eq!(i.capabilities, Capabilities::default());
        // Capabilities D0, D1; D2-D7 ignored (REQ-NET-INV-002).
        let p = write(|w| {
            Invitation::write(
                w,
                &name,
                &id,
                Capabilities {
                    auth: true,
                    user_auth: true,
                },
            )
        });
        assert_eq!(p[7], 0x03);
        let mut q = a11;
        q[7] = 0xFE;
        let Command::Invitation(i) = one(&q) else {
            panic!()
        };
        assert_eq!(
            i.capabilities,
            Capabilities {
                auth: false,
                user_auth: true
            }
        );
        // csd1 above pl is Malformed; values above the ranges are accepted.
        let bad = raw(&[0x01, 0x01, 0x02, 0x00, b'a', 0, 0, 0]);
        let PacketItem::Command(c) = command_packets(&bad).unwrap().next().unwrap() else {
            panic!()
        };
        assert_eq!(Command::parse(c), Err(Malformed));
        let big = raw(&[
            0x01, 0x02, 0x00, 0x00, b'a', b'b', b'c', b'd', b'e', 0, 0, 0,
        ]);
        let Command::Invitation(i) = one(&big) else {
            panic!()
        };
        assert!(i.name.is_empty());
        assert_eq!(i.product_instance_id.to_str(), Ok("abcde"));
    }

    #[test]
    fn invitation_replies() {
        // REQ-NET-INV-003, -004, -033, -034.
        let name = EndpointName::new("Host").unwrap();
        let id = ProductInstanceId::new("H1").unwrap();
        let p = write(|w| InvitationReply::write(w, false, &name, &id));
        assert_eq!(&p[4..8], [0x10, 0x02, 0x01, 0x00]);
        let p2 = write(|w| InvitationReply::write(w, true, &name, &id));
        assert_eq!(&p2[4..8], [0x11, 0x02, 0x01, 0x00]);
        let Command::InvitationReply(r) = one(&p2) else {
            panic!()
        };
        assert!(r.pending);
        assert_eq!(
            (r.name.to_str(), r.product_instance_id.to_str()),
            (Ok("Host"), Ok("H1"))
        );
        // csd2 ignored, empty name accepted.
        let q = raw(&[0x10, 0x00, 0x00, 0x77]);
        let Command::InvitationReply(r) = one(&q) else {
            panic!()
        };
        assert!(r.name.is_empty() && r.product_instance_id.is_empty() && !r.pending);
    }

    #[test]
    fn ump_data_vectors() {
        // REQ-NET-VEC-002, -003, -004, REQ-NET-DATA-001, -005, REQ-NET-PKT-005.
        let a12 = raw(&[0xFF, 0x01, 0x00, 0x10, 0x10, 0xF8, 0x00, 0x00]);
        assert_eq!(
            write(|w| UmpData::write(w, SeqNum(0x10), &[0x10F8_0000])),
            a12
        );
        let a13 = raw(&[
            0xFF, 0x04, 0x00, 0x11, 0x45, 0x90, 0x40, 0x00, 0x12, 0x34, 0x00, 0x00, 0x45, 0x80,
            0x40, 0x00, 0x01, 0x00, 0x00, 0x00,
        ]);
        let words = [0x4590_4000, 0x1234_0000, 0x4580_4000, 0x0100_0000];
        assert_eq!(write(|w| UmpData::write(w, SeqNum(0x11), &words)), a13);
        let Command::UmpData(d) = one(&a13) else {
            panic!()
        };
        assert_eq!(d.seq, SeqNum(0x11));
        let m: Vec<_> = d.messages().map(|m| m.words().to_vec()).collect();
        assert_eq!(m, [[0x4590_4000, 0x1234_0000], [0x4580_4000, 0x0100_0000]]);
        let a14 = raw(&[
            0xFF, 0x02, 0x34, 0x56, 0x45, 0x90, 0x40, 0x00, 0x12, 0x34, 0x00, 0x00, 0xFF, 0x02,
            0x34, 0x57, 0x45, 0x80, 0x40, 0x00, 0x01, 0x00, 0x00, 0x00,
        ]);
        let p = write(|w| {
            UmpData::write(w, SeqNum(0x3456), &words[..2])?;
            UmpData::write(w, SeqNum(0x3457), &words[2..])
        });
        assert_eq!(p, a14);
        let cmds: Vec<_> = command_packets(&a14)
            .unwrap()
            .map(|i| match i {
                PacketItem::Command(c) => Command::parse(c).unwrap(),
                _ => panic!(),
            })
            .collect();
        assert!(matches!(cmds[1], Command::UmpData(d) if d.seq == SeqNum(0x3457)));
    }

    #[test]
    fn ump_data_rules() {
        // REQ-NET-DATA-001, -006.
        assert_eq!(
            write_err(|w| UmpData::write(w, SeqNum(1), &[0x4590_4000])),
            CommandError::InvalidUmpData
        );
        let many = [0x10F8_0000u32; 65];
        assert_eq!(
            write_err(|w| UmpData::write(w, SeqNum(1), &many)),
            CommandError::InvalidUmpData
        );
        assert!(write(|w| UmpData::write(w, SeqNum(1), &many[..64])).len() == 4 + 4 + 256);
        // Zero length; pl above 64 is read.
        let z = raw(&[0xFF, 0x00, 0x00, 0x05]);
        assert!(matches!(one(&z), Command::UmpData(d) if d.is_empty()));
        let mut big = raw(&[0xFF, 65, 0, 0]);
        big.extend((0..65).flat_map(|_| 0x10F8_0000u32.to_be_bytes()));
        let Command::UmpData(d) = one(&big) else {
            panic!()
        };
        assert_eq!(d.messages().count(), 65);
        // A payload that ends inside a message: one 1-word message, then a 2-word message
        // cut after its first word.
        let cut = raw(&[0xFF, 0x02, 0, 1, 0x10, 0xF8, 0, 0, 0x45, 0x90, 0x40, 0]);
        let Command::UmpData(d) = one(&cut) else {
            panic!()
        };
        let mut it = d.messages();
        let it2: Vec<_> = it.by_ref().collect();
        assert!(it.truncated());
        assert_eq!(it2.len(), 1);
    }

    fn write_err(
        f: impl FnOnce(&mut PayloadWriter<'_>) -> Result<(), CommandError>,
    ) -> CommandError {
        let mut buf = [0u8; 1400];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        let e = f(&mut w).unwrap_err();
        assert!(w.is_empty(), "nothing written");
        e
    }

    #[test]
    fn ping_reset_bye_reply() {
        // REQ-NET-PING-001, -002, REQ-NET-RST-001, -002, REQ-NET-BYE-005, REQ-NET-PKT-013.
        assert_eq!(
            write(|w| write_ping(w, 0x0102_0304)),
            raw(&[0x20, 1, 0, 0, 1, 2, 3, 4])
        );
        assert_eq!(
            write(|w| write_ping_reply(w, 7)),
            raw(&[0x21, 1, 0, 0, 0, 0, 0, 7])
        );
        assert_eq!(
            write(|w| write_empty(w, CommandCode::SessionReset)),
            raw(&[0x82, 0, 0, 0])
        );
        assert_eq!(
            write(|w| write_empty(w, CommandCode::SessionResetReply)),
            raw(&[0x83, 0, 0, 0])
        );
        assert_eq!(
            write(|w| write_empty(w, CommandCode::ByeReply)),
            raw(&[0xF1, 0, 0, 0])
        );
        // Readers: extra words and reserved bits ignored; pl 0 for Ping is Malformed.
        assert_eq!(
            one(&raw(&[0x20, 2, 0xAA, 0xBB, 0, 0, 0, 9, 1, 1, 1, 1]).clone()),
            Command::Ping(9)
        );
        assert_eq!(
            one(&raw(&[0x82, 1, 5, 5, 1, 2, 3, 4]).clone()),
            Command::SessionReset
        );
        assert_eq!(one(&raw(&[0xF1, 0, 9, 9]).clone()), Command::ByeReply);
        let p = raw(&[0x21, 0, 0, 0]);
        let PacketItem::Command(c) = command_packets(&p).unwrap().next().unwrap() else {
            panic!()
        };
        assert_eq!(Command::parse(c), Err(Malformed));
        let mut small = [0u8; 6];
        let mut w = PayloadWriter::new(&mut small, MaxPayload::default());
        assert_eq!(
            write_ping(&mut w, 1),
            Err(CommandError::Packet(PacketError::TooLarge))
        );
        assert!(w.is_empty());
    }

    #[test]
    fn nak_rules() {
        // REQ-NET-NAK-001, -002, -003, -006, -007, -009.
        let h = Header::from_bytes([0x30, 0x01, 0xAB, 0xCD]);
        let p = write(|w| Nak::write(w, NakReason::COMMAND_NOT_SUPPORTED, h, ""));
        assert_eq!(p, raw(&[0x8F, 1, 0x01, 0x00, 0x30, 0x01, 0xAB, 0xCD]));
        let p = write(|w| Nak::write(w, NakReason::BAD_PING_REPLY, h, "bad"));
        assert_eq!(
            p,
            raw(&[0x8F, 2, 0x20, 0x00, 0x30, 0x01, 0xAB, 0xCD, b'b', b'a', b'd', 0])
        );
        let Command::Nak(n) = one(&p) else { panic!() };
        assert_eq!(
            (n.reason, n.nak_header, n.text.to_str()),
            (NakReason::BAD_PING_REPLY, h, Ok("bad"))
        );
        assert_eq!(
            write_err(|w| Nak::write(w, NakReason(0x55), h, "")),
            CommandError::ReasonNotAllowed
        );
        assert_eq!(
            write_err(|w| Nak::write(w, NakReason::OTHER, h, &"x".repeat(1017))),
            CommandError::TextTooLong
        );
        assert_eq!(
            write_err(|w| Nak::write(w, NakReason::OTHER, h, "\u{FEFF}x")),
            CommandError::InvalidText
        );
        assert_eq!(
            write_err(|w| Nak::write(w, NakReason::OTHER, h, "a\0")),
            CommandError::InvalidText
        );
        // Reader: unknown reason kept; text not UTF-8 given as bytes; pl 0 Malformed.
        let q = raw(&[0x8F, 2, 0x77, 0x55, 1, 2, 3, 4, 0xFF, b'x', 0, 0]);
        let Command::Nak(n) = one(&q) else { panic!() };
        assert!(!n.reason.is_defined());
        assert_eq!(n.text.as_bytes(), [0xFF, b'x']);
        assert!(n.text.to_str().is_err());
        let z = raw(&[0x8F, 0, 0, 0]);
        let PacketItem::Command(c) = command_packets(&z).unwrap().next().unwrap() else {
            panic!()
        };
        assert_eq!(Command::parse(c), Err(Malformed));
    }

    #[test]
    fn bye_rules() {
        // REQ-NET-BYE-001 to -004, -012.
        let p = write(|w| Bye::write(w, Role::Client, ByeReason::USER_TERMINATED, ""));
        assert_eq!(p, raw(&[0xF0, 0, 0x01, 0]));
        let p = write(|w| Bye::write(w, Role::Host, ByeReason::TOO_MANY_SESSIONS, "full"));
        assert_eq!(p, raw(&[0xF0, 1, 0x40, 0, b'f', b'u', b'l', b'l']));
        assert_eq!(
            write_err(|w| Bye::write(w, Role::Client, ByeReason::TOO_MANY_SESSIONS, "")),
            CommandError::ReasonNotAllowed
        );
        assert_eq!(
            write_err(|w| Bye::write(w, Role::Host, ByeReason::INVITATION_CANCELED, "")),
            CommandError::ReasonNotAllowed
        );
        assert_eq!(
            write_err(|w| Bye::write(w, Role::Host, ByeReason(0x99), "")),
            CommandError::ReasonNotAllowed
        );
        assert_eq!(
            write_err(|w| Bye::write(w, Role::Host, ByeReason::TIMEOUT, &"x".repeat(1021))),
            CommandError::TextTooLong
        );
        assert!(
            write(|w| Bye::write(w, Role::Host, ByeReason::TIMEOUT, &"x".repeat(1020))).len()
                > 1020
        );
        // Reader: any reason, pl 0 and up, reserved ignored.
        let tmp = raw(&[0xF0, 0, 0x99, 0x12]);
        let Command::Bye(b) = one(&tmp) else { panic!() };
        assert_eq!((b.reason, b.text.is_empty()), (ByeReason(0x99), true));
        assert!(!b.reason.is_defined());
    }

    #[test]
    fn retransmit_commands() {
        // REQ-NET-RTX-001, -002, -014.
        let r = RetransmitRequest {
            seq: SeqNum(0x1234),
            count: 3,
        };
        let p = write(|w| r.write(w));
        assert_eq!(p, raw(&[0x80, 1, 0x12, 0x34, 0, 3, 0, 0]));
        assert_eq!(one(&p), Command::RetransmitRequest(r));
        let e = RetransmitError {
            reason: RetransmitErrorReason::NOT_IN_BUFFER,
            seq: SeqNum(0x00FF),
        };
        let p = write(|w| e.write(w));
        assert_eq!(p, raw(&[0x81, 1, 0x01, 0, 0x00, 0xFF, 0, 0]));
        assert_eq!(one(&p), Command::RetransmitError(e));
        // Reader: reserved ignored, unknown reason kept; pl 0 Malformed.
        let q = raw(&[0x81, 2, 0x09, 0x77, 0x00, 0x05, 0xAB, 0xCD, 1, 1, 1, 1]);
        let Command::RetransmitError(e) = one(&q) else {
            panic!()
        };
        assert_eq!((e.reason, e.seq), (RetransmitErrorReason(9), SeqNum(5)));
        for z in [raw(&[0x80, 0, 0, 0]), raw(&[0x81, 0, 0, 0])] {
            let PacketItem::Command(c) = command_packets(&z).unwrap().next().unwrap() else {
                panic!()
            };
            assert_eq!(Command::parse(c), Err(Malformed));
        }
    }

    #[test]
    fn auth_codes_are_known() {
        // REQ-NET-AUTH-012: the four codes parse to Auth with the header only.
        for code in [0x02, 0x03, 0x12, 0x13] {
            let p = raw(&[code, 1, 0, 0, 1, 2, 3, 4]);
            assert!(matches!(one(&p), Command::Auth(r) if r.header.code == code));
        }
        assert!(matches!(one(&raw(&[0x30, 0, 0, 0])), Command::Unknown(_)));
    }

    #[test]
    fn seq_num_serial() {
        assert_eq!(SeqNum(0xFFFF).next(), SeqNum(0));
        assert_eq!(SeqNum(0xFFFE).distance_to(SeqNum(2)), 4);
        assert_eq!(SeqNum(2).distance_to(SeqNum(0xFFFE)), -4);
        assert_eq!(SeqNum(10).add(0xFFFF), SeqNum(9));
    }

    #[test]
    fn prop_parse_never_panics() {
        // Nothing panics on received bytes.
        proptest!(|(data in proptest::collection::vec(any::<u8>(), 0..200))| {
            let mut p = Vec::from(*b"MIDI");
            p.extend(&data);
            for it in command_packets(&p).unwrap() {
                if let PacketItem::Command(c) = it {
                    if let Ok(Command::UmpData(d)) = Command::parse(c) {
                        let _ = d.messages().count();
                    }
                }
            }
        });
    }
}
