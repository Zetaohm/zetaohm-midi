//! Whole-message byte tests of the replies that a Device writes, from the M2-101 tables (DEC-001).
//!
//! Each expected message is written by hand from its table, not from the output of the code: a round trip
//! (write, then read) cannot find an error that the writer and the reader share. These tests guard the
//! changes of OPT-049 (steps 4 and 7): the reply bytes must not change.
//!
//! MUIDs are 28-bit values, written as four 7-bit bytes, LSB first (Table 5):
//! SRC 0x0123_4567 = 67 0A 0D 09, DST 0x0765_4321 = 21 06 15 3B. Message Version 0x02.

use zmidi_ci::{
    Ack, AckStatus, Categories, Details, DeviceId, DeviceIdentity, DiscoveryReply,
    EndpointInfoReply, EndpointInfoStatus, Framing, InvalidateMuid, ManufacturerId, Muid, Nak,
    NakStatus, PeCapabilities, PiCapabilities, ProfileDetailsReply, ProfileId, ProfileInquiryReply,
};

const SRC: Muid = match Muid::new(0x0123_4567) {
    Ok(m) => m,
    Err(_) => panic!(),
};
const DST: Muid = match Muid::new(0x0765_4321) {
    Ok(m) => m,
    Err(_) => panic!(),
};
const SRC_B: [u8; 4] = [0x67, 0x0A, 0x0D, 0x09];
const DST_B: [u8; 4] = [0x21, 0x06, 0x15, 0x3B];

/// The 13 header bytes of Table 5 (Framing::Bare: no 0xF0).
fn header(device_id: u8, sub_id2: u8, dst: [u8; 4]) -> Vec<u8> {
    let mut v = vec![0x7E, device_id, 0x0D, sub_id2, 0x02];
    v.extend(SRC_B);
    v.extend(dst);
    v
}

fn written(f: impl FnOnce(&mut [u8]) -> Result<usize, zmidi_ci::WriteError>) -> Vec<u8> {
    let mut buf = [0u8; 128];
    let n = f(&mut buf).unwrap();
    buf[..n].to_vec()
}

#[test]
fn reply_to_discovery_table_8() {
    // Table 8, with System Exclusive framing (F0 ... F7); REQ-CI-DISC-012.
    let r = DiscoveryReply {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::OneByte(0x41),
            family: 0x1234,
            model: 0x0201,
            revision: [1, 2, 3, 4],
        },
        categories: Categories {
            profile_configuration: true,
            property_exchange: true,
            process_inquiry: true,
            ..Default::default()
        },
        max_sysex_size: 512,
        output_path_id: Some(3),
        function_block: Some(5),
    };
    let mut want = vec![0xF0];
    want.extend(header(0x7F, 0x71, DST_B));
    want.extend([0x41, 0x00, 0x00]); // Device Manufacturer
    want.extend([0x34, 0x24]); // Device Family 0x1234, LSB first, 7 bits each
    want.extend([0x01, 0x04]); // Model 0x0201
    want.extend([1, 2, 3, 4]); // Software Revision Level
    want.push(0x1C); // Categories: D2 Profiles, D3 PE, D4 Process Inquiry (Table 7)
    want.extend([0x00, 0x04, 0x00, 0x00]); // 512, LSB first
    want.push(0x03); // Initiator's Output Path Instance ID
    want.push(0x05); // Function Block
    want.push(0xF7);
    assert_eq!(written(|b| r.write(SRC, DST, Framing::SysEx, b)), want);

    // No Function Block: 0x7F (5.6.2).
    let r = DiscoveryReply {
        function_block: None,
        ..r
    };
    let got = written(|b| r.write(SRC, DST, Framing::Bare, b));
    assert_eq!(got.last(), Some(&0x7F));
    assert_eq!(got.len(), 13 + 16 + 2);
}

#[test]
fn reply_to_endpoint_information_table_11() {
    // Table 11; REQ-CI-EPI-005, REQ-CI-EPI-008.
    let r = EndpointInfoReply {
        status: EndpointInfoStatus::ProductInstanceId,
        data: b"SN-0001",
    };
    let mut want = header(0x7F, 0x73, DST_B);
    want.push(0x00); // Status
    want.extend([0x07, 0x00]); // Length of Following Information Data
    want.extend(b"SN-0001");
    assert_eq!(written(|b| r.write(SRC, DST, Framing::Bare, b)), want);
}

#[test]
fn invalidate_muid_table_12() {
    // Table 12; REQ-CI-INV-008: to the Broadcast MUID, the target MUID last.
    let m = InvalidateMuid { target: DST };
    let mut want = header(0x7F, 0x7E, [0x7F; 4]);
    want.extend(DST_B);
    assert_eq!(written(|b| m.write(SRC, Framing::Bare, b)), want);
}

#[test]
fn ack_table_13() {
    // Table 13; REQ-CI-ACK-001 to -004.
    let a = Ack {
        original_sub_id2: 0x34,
        status: AckStatus::TimeoutWait(10),
        details: Details::PropertyExchange {
            request_id: 5,
            chunk: 300,
        },
        text: &[],
    };
    let mut want = header(0x7E, 0x7D, DST_B);
    want.extend([0x34, 0x10, 0x0A]); // Original Sub-ID#2, Status Code, Status Data
    want.extend([5, 0x2C, 0x02, 0, 0]); // details: Request ID, Chunk 300 (LSB first), 0, 0
    want.extend([0x04, 0x00]); // Message Length
    want.extend(b"Wait");
    assert_eq!(
        written(|b| a.write("Wait", DeviceId::Group, SRC, DST, None, Framing::Bare, b)),
        want
    );
}

#[test]
fn nak_table_15() {
    // Table 15; REQ-CI-NAK-001, REQ-CI-NAK-002: the automatic NAK of the Device has no text.
    let k = Nak {
        original_sub_id2: 0x30,
        status: NakStatus::NotSupported,
        details: Details::Reserved,
        text: &[],
    };
    let mut want = header(0x7F, 0x7F, DST_B);
    want.extend([0x30, 0x01, 0x00]); // Original Sub-ID#2, NAK Status Code 0x01, Status Data
    want.extend([0; 5]); // details
    want.extend([0x00, 0x00]); // Message Length 0
    assert_eq!(
        written(|b| k.write(
            "",
            DeviceId::FunctionBlock,
            SRC,
            DST,
            None,
            Framing::Bare,
            b
        )),
        want
    );

    // With text, a Channel Device ID and System Exclusive framing.
    let k = Nak {
        original_sub_id2: 0x22,
        status: NakStatus::ProfileNotSupported,
        details: Details::Profile([0x7E, 0x21, 0x00, 0x01, 0x01]),
        text: &[],
    };
    let mut want = vec![0xF0];
    want.extend(header(0x03, 0x7F, DST_B));
    want.extend([0x22, 0x04, 0x00, 0x7E, 0x21, 0x00, 0x01, 0x01, 0x02, 0x00]);
    want.extend(b"no");
    want.push(0xF7);
    assert_eq!(
        written(|b| k.write(
            "no",
            DeviceId::Channel(3),
            SRC,
            DST,
            Some(512),
            Framing::SysEx,
            b
        )),
        want
    );
}

#[test]
fn reply_to_pe_capabilities_table_32() {
    // Table 32; REQ-CI-PEF-004 to -006.
    let c = PeCapabilities {
        simultaneous: 4,
        major: Some(0),
        minor: Some(1),
    };
    let mut want = header(0x7F, 0x31, DST_B);
    want.extend([0x04, 0x00, 0x01]);
    assert_eq!(written(|b| c.write_reply(SRC, DST, Framing::Bare, b)), want);
}

#[test]
fn reply_to_process_inquiry_capabilities_table_41() {
    // Table 41 and Table 42: D0 MIDI Message Report.
    let c = PiCapabilities {
        midi_message_report: true,
    };
    let mut want = header(0x7F, 0x41, DST_B);
    want.push(0x01);
    assert_eq!(written(|b| c.write(SRC, DST, Framing::Bare, b)), want);
}

#[test]
fn reply_to_profile_inquiry_table_18() {
    // Table 18: enabled count, IDs, disabled count, IDs (counts LSB first).
    let on = ProfileId::from_bytes([0x7E, 0x21, 0x00, 0x01, 0x01]).unwrap();
    let off = ProfileId::from_bytes([0x00, 0x21, 0x09, 0x02, 0x03]).unwrap();
    let mut want = header(0x00, 0x21, DST_B);
    want.extend([0x01, 0x00, 0x7E, 0x21, 0x00, 0x01, 0x01]);
    want.extend([0x01, 0x00, 0x00, 0x21, 0x09, 0x02, 0x03]); // three-byte SysEx ID (7.3.1)
    assert_eq!(
        written(|b| ProfileInquiryReply::write(
            [on],
            [off],
            DeviceId::Channel(0),
            SRC,
            DST,
            Framing::Bare,
            b
        )),
        want
    );
}

#[test]
fn reply_to_profile_details_table_23() {
    // Table 23: Profile ID, Inquiry Target, Target Data Length (LSB first), data.
    let r = ProfileDetailsReply {
        profile: ProfileId::from_bytes([0x7E, 0x21, 0x00, 0x01, 0x01]).unwrap(),
        target: 0x00,
        data: &[0x04, 0x00],
    };
    let mut want = header(0x7E, 0x29, DST_B);
    want.extend([0x7E, 0x21, 0x00, 0x01, 0x01, 0x00, 0x02, 0x00, 0x04, 0x00]);
    assert_eq!(
        written(|b| r.write(DeviceId::Group, SRC, DST, Framing::Bare, b)),
        want
    );
}
