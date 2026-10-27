//! MIDI-CI Property Exchange (M2-103-UM v1.2): the meaning of the Header Data, status codes,
//! Mcoded7, zlib, Resources and Subscriptions (DEC-006). The message framing is in `zmidi-ci`.
//!
//! - The crate is `no_std` and needs no `alloc` (DEC-002, DEC-007): the Header Data is parsed in
//!   a fixed-capacity buffer and the Property Data is streamed to the caller.
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

pub mod encoding;
pub mod header;
#[cfg(feature = "zlib")]
mod inflate;
pub mod json;
pub mod link;
pub mod one_chunk;
pub mod property_data;
pub mod resource_list;
pub mod subscription;
pub mod transaction;
#[cfg(feature = "zlib")]
pub mod zlib;

pub use encoding::{
    mcoded7_len, DataDecoder, DataEncoder, EncodingError, Mcoded7Decoder, Mcoded7Encoder,
};
pub use header::{
    Command, EncodingName, EndedBy, Header, HeaderChars, HeaderError, HeaderKind, HeaderStr,
    Status, StatusClass,
};
pub use json::{JsonError, JsonReader, JsonWriter, Number, Token, Tokens};
pub use link::{ConfirmIcon, ConfirmType, Link, LinkError, LinkRole};
pub use property_data::{
    write_simple, PartialSetItem, PartialSetItems, PartialSetReader, PartialSetWriter,
    PropertyDataError, ScalarValue, SimpleValueReader,
};
pub use resource_list::{
    find_resource, CanSet, Column, ColumnDecl, ColumnKind, Encodings, ListError, ListEvent,
    ListEvents, Operation, Present, ResourceCaps, ResourceDecl, ResourceEntry, ResourceListReader,
    ResourceListWriter,
};
pub use subscription::{
    check_send, start_header, update_command, Role, SubscriptionEntry, SubscriptionError,
    SubscriptionInput, SubscriptionTable,
};
pub use transaction::{
    default_message, schema_ref, CloseReason, EngineConfig, Message, Outgoing, PeEngine, PeEvent,
    PeHost, RequestInfo, TransactionError, TxnKind, DEFAULT_QUEUE,
};
#[cfg(feature = "zlib")]
pub use zlib::{Progress, ZlibInflater, ZlibMcoded7Decoder, ZlibMcoded7Encoder};
