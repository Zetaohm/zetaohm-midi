//! The GATT data of BLE-MIDI v1.0 section 3, for the BLE host adapter (DEC-016).
//!
//! - MIDI Service: 03B80E5A-EDE8-4B33-A751-6CE34EC4C700.
//! - MIDI Data I/O Characteristic: 7772E5DB-3868-4112-A1A9-F2669D106BF3, with the properties
//!   read, write, write without response and notify (AMB-BLE-MIDI-03, DEC-BLE-001).
//! - A read of the characteristic gives a packet with no payload (sections 3 and 5).
//! - The device requests a connection interval of 15 ms or less (section 4).
//!
//! The adapter registers the service, answers the read and requests the interval; this module
//! only gives the values (DEC-004).

/// The MIDI Service UUID as a 128-bit value (section 3).
pub const SERVICE_UUID: u128 = 0x03B8_0E5A_EDE8_4B33_A751_6CE3_4EC4_C700;

/// The MIDI Data I/O Characteristic UUID as a 128-bit value (section 3).
pub const CHARACTERISTIC_UUID: u128 = 0x7772_E5DB_3868_4112_A1A9_F266_9D10_6BF3;

/// The MIDI Service UUID in the byte order of the attribute protocol (least significant byte
/// first).
pub const SERVICE_UUID_LE: [u8; 16] = SERVICE_UUID.to_le_bytes();

/// The MIDI Data I/O Characteristic UUID in the byte order of the attribute protocol (least
/// significant byte first).
pub const CHARACTERISTIC_UUID_LE: [u8; 16] = CHARACTERISTIC_UUID.to_le_bytes();

/// Characteristic property: read.
pub const PROPERTY_READ: u8 = 0x02;
/// Characteristic property: write without response (section 3: required).
pub const PROPERTY_WRITE_WITHOUT_RESPONSE: u8 = 0x04;
/// Characteristic property: write.
pub const PROPERTY_WRITE: u8 = 0x08;
/// Characteristic property: notify.
pub const PROPERTY_NOTIFY: u8 = 0x10;

/// The properties of the MIDI Data I/O Characteristic: read, write without response, write and
/// notify (0x1E).
pub const CHARACTERISTIC_PROPERTIES: u8 =
    PROPERTY_READ | PROPERTY_WRITE_WITHOUT_RESPONSE | PROPERTY_WRITE | PROPERTY_NOTIFY;

/// The value of a read of the MIDI Data I/O Characteristic: a packet with no payload (sections
/// 3 and 5).
pub const READ_VALUE: &[u8] = &[];

/// The largest connection interval that the device requests, in milliseconds (section 4).
pub const MAX_CONNECTION_INTERVAL_MS: u16 = 15;

/// [`MAX_CONNECTION_INTERVAL_MS`] in the unit of the Bluetooth connection parameters (1.25 ms).
pub const MAX_CONNECTION_INTERVAL_UNITS: u16 = 12;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_of_section_3() {
        assert_eq!(
            SERVICE_UUID_LE,
            [
                0x00, 0xC7, 0xC4, 0x4E, 0xE3, 0x6C, 0x51, 0xA7, 0x33, 0x4B, 0xE8, 0xED, 0x5A, 0x0E,
                0xB8, 0x03
            ]
        );
        assert_eq!(
            CHARACTERISTIC_UUID_LE,
            [
                0xF3, 0x6B, 0x10, 0x9D, 0x66, 0xF2, 0xA9, 0xA1, 0x12, 0x41, 0x68, 0x38, 0xDB, 0xE5,
                0x72, 0x77
            ]
        );
    }

    #[test]
    fn characteristic_properties_are_0x1e() {
        assert_eq!(CHARACTERISTIC_PROPERTIES, 0x1E);
        assert_eq!(
            [
                PROPERTY_READ,
                PROPERTY_WRITE_WITHOUT_RESPONSE,
                PROPERTY_WRITE,
                PROPERTY_NOTIFY
            ],
            [0x02, 0x04, 0x08, 0x10]
        );
    }

    #[test]
    fn read_value_is_empty_and_interval_is_15_ms() {
        assert!(READ_VALUE.is_empty());
        assert_eq!(
            MAX_CONNECTION_INTERVAL_UNITS * 125,
            MAX_CONNECTION_INTERVAL_MS * 100
        );
    }
}
