//! The UMP Endpoint Name and the Product Instance Id of a Host or Client instance
//! (M2-124-UM v1.0.1 sections 4.4, 6.4, 6.5; REQ-NET-DISC-011, -012, -017).
//!
//! - The values are checked one time, here, with the same rules as the zmidi-ump Stream text
//!   writers, so that the TXT record, the Invitation and the UMP Notifications carry the same
//!   text (DEC-NET-001 N-60).
//! - Each value copies its text into fixed storage (DEC-002).

use zmidi_ump::ump_stream::StreamTextPackets;

/// Largest UMP Endpoint Name, in bytes (M2-104 7.1.4).
pub const MAX_ENDPOINT_NAME: usize = 98;
/// Largest Product Instance Id, in bytes (M2-104 7.1.5).
pub const MAX_PRODUCT_INSTANCE_ID: usize = 42;

/// Error of the Identity constructors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// The text is empty.
    #[error("empty")]
    Empty,
    /// The text is longer than the limit (98 or 42 bytes).
    #[error("too long")]
    TooLong,
    /// A character that the UMP writer refuses: a Byte Order Mark or a zero byte in a name, a
    /// byte outside ASCII 32 to 126 in a Product Instance Id.
    #[error("invalid character")]
    InvalidCharacter,
}

/// A UMP Endpoint Name: 1 to 98 bytes of UTF-8 that the zmidi-ump writer
/// `StreamTextPackets::endpoint_name` accepts (REQ-NET-DISC-011).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EndpointName {
    buf: [u8; MAX_ENDPOINT_NAME],
    len: u8,
}

impl core::fmt::Debug for EndpointName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("EndpointName").field(&self.as_str()).finish()
    }
}

impl EndpointName {
    /// A checked name.
    pub fn new(name: &str) -> Result<Self, IdentityError> {
        if name.is_empty() {
            return Err(IdentityError::Empty);
        }
        if name.len() > MAX_ENDPOINT_NAME {
            return Err(IdentityError::TooLong);
        }
        if StreamTextPackets::endpoint_name(name).is_err() {
            return Err(IdentityError::InvalidCharacter);
        }
        let mut buf = [0; MAX_ENDPOINT_NAME];
        buf[..name.len()].copy_from_slice(name.as_bytes());
        Ok(EndpointName {
            buf,
            len: name.len() as u8,
        })
    }

    /// The text, for the Notification writer of zmidi-ump.
    pub fn as_str(&self) -> &str {
        // The constructor took a `&str`, so the bytes are UTF-8.
        core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
    }
}

/// A Product Instance Id: 1 to 42 bytes, each in ASCII 32 to 126; the same ids as the
/// zmidi-ump writer `StreamTextPackets::product_instance_id` (REQ-NET-DISC-012, -017).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProductInstanceId {
    buf: [u8; MAX_PRODUCT_INSTANCE_ID],
    len: u8,
}

impl core::fmt::Debug for ProductInstanceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ProductInstanceId")
            .field(&self.as_str())
            .finish()
    }
}

impl ProductInstanceId {
    /// A checked id.
    pub fn new(id: &str) -> Result<Self, IdentityError> {
        if id.is_empty() {
            return Err(IdentityError::Empty);
        }
        if id.len() > MAX_PRODUCT_INSTANCE_ID {
            return Err(IdentityError::TooLong);
        }
        if StreamTextPackets::product_instance_id(id).is_err() {
            return Err(IdentityError::InvalidCharacter);
        }
        let mut buf = [0; MAX_PRODUCT_INSTANCE_ID];
        buf[..id.len()].copy_from_slice(id.as_bytes());
        Ok(ProductInstanceId {
            buf,
            len: id.len() as u8,
        })
    }

    /// The text, for the Notification writer of zmidi-ump.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
    }
}

/// The name and the id of one instance; the Host, its Clients and its TXT record use the same
/// values (N-60).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    /// UMP Endpoint Name.
    pub name: EndpointName,
    /// Product Instance Id.
    pub product_instance_id: ProductInstanceId,
}

impl Identity {
    /// An identity from two texts.
    pub fn new(name: &str, product_instance_id: &str) -> Result<Self, IdentityError> {
        Ok(Identity {
            name: EndpointName::new(name)?,
            product_instance_id: ProductInstanceId::new(product_instance_id)?,
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::string::String;

    #[test]
    fn endpoint_name_rules() {
        // REQ-NET-DISC-011.
        assert_eq!(EndpointName::new("MyDev").unwrap().as_str(), "MyDev");
        // Debug shows the text, not the unused bytes of the array.
        let debug = std::format!("{:?}", EndpointName::new("MyDev").unwrap());
        assert_eq!(debug, r#"EndpointName("MyDev")"#);
        let debug = std::format!("{:?}", ProductInstanceId::new("8shYe3h5").unwrap());
        assert_eq!(debug, r#"ProductInstanceId("8shYe3h5")"#);
        assert_eq!(EndpointName::new("Synthé ♪").unwrap().as_str(), "Synthé ♪");
        assert_eq!(EndpointName::new(""), Err(IdentityError::Empty));
        let n98 = "x".repeat(98);
        assert_eq!(EndpointName::new(&n98).unwrap().as_str(), n98);
        assert_eq!(
            EndpointName::new(&"x".repeat(99)),
            Err(IdentityError::TooLong)
        );
        assert_eq!(
            EndpointName::new("\u{FEFF}Dev"),
            Err(IdentityError::InvalidCharacter)
        );
        assert_eq!(
            EndpointName::new("a\0b"),
            Err(IdentityError::InvalidCharacter)
        );
    }

    #[test]
    fn product_instance_id_rules() {
        // REQ-NET-DISC-012, -017.
        assert_eq!(
            ProductInstanceId::new("8shYe3h5").unwrap().as_str(),
            "8shYe3h5"
        );
        assert_eq!(ProductInstanceId::new(""), Err(IdentityError::Empty));
        assert_eq!(
            ProductInstanceId::new(&"1".repeat(43)),
            Err(IdentityError::TooLong)
        );
        assert!(ProductInstanceId::new(&"1".repeat(42)).is_ok());
        assert_eq!(
            ProductInstanceId::new("a\tb"),
            Err(IdentityError::InvalidCharacter)
        );
        assert_eq!(
            ProductInstanceId::new("é"),
            Err(IdentityError::InvalidCharacter)
        );
        let i = Identity::new("MyDev", "8shYe3h5").unwrap();
        assert_eq!(
            (i.name.as_str(), i.product_instance_id.as_str()),
            ("MyDev", "8shYe3h5")
        );
    }

    #[test]
    fn prop_same_rules_as_midi_ump() {
        // REQ-NET-DISC-011, -017: a non-empty text is accepted exactly when zmidi-ump accepts it.
        proptest!(|(s in any::<String>())| {
            if !s.is_empty() {
                prop_assert_eq!(
                    EndpointName::new(&s).is_ok(),
                    StreamTextPackets::endpoint_name(&s).is_ok()
                );
                prop_assert_eq!(
                    ProductInstanceId::new(&s).is_ok(),
                    StreamTextPackets::product_instance_id(&s).is_ok()
                );
            }
        });
    }
}
