//! Rust tests of the C ABI rules (DEC-019): stable status values, no panic on any input.

use zmidi::status::{zmidi_abi_version, ZmidiStatus, ZMIDI_ABI_MAJOR, ZMIDI_ABI_MINOR};

/// REQ-FFI-ABI-001: the values of the status never change.
#[test]
fn status_values_are_stable() {
    assert_eq!(ZmidiStatus::Ok as u32, 0);
    assert_eq!(ZmidiStatus::InvalidArgument as u32, 1);
    assert_eq!(ZmidiStatus::BufferTooSmall as u32, 2);
    assert_eq!(ZmidiStatus::NoPending as u32, 3);
    assert_eq!(ZmidiStatus::Rejected as u32, 4);
    assert_eq!(ZmidiStatus::NotSupported as u32, 5);
    assert_eq!(zmidi_abi_version(), ZMIDI_ABI_MAJOR << 16 | ZMIDI_ABI_MINOR);
}

#[cfg(feature = "io")]
mod io {
    use proptest::prelude::*;
    use zmidi::io::*;
    use zmidi::status::ZmidiStatus;

    /// REQ-FFI-ABI-001, REQ-FFI-IO-002: any byte stream, Group and protocol give a status and
    /// never more words than the buffer holds; no panic.
    #[test]
    fn prop_bytes_to_ump_never_panics() {
        proptest!(|(group in 0u8..20, protocol in 0u32..4, bytes in proptest::collection::vec(any::<u8>(), 0..64), cap in 0usize..8)| {
            let mut r = ZmidiBytesToUmp { _opaque: [0; ZMIDI_BYTES_TO_UMP_WORDS] };
            let st = unsafe { zmidi_bytes_to_ump_init(&mut r, group, protocol) };
            prop_assert_eq!(st == ZmidiStatus::Ok, group < 16 && (1..=2).contains(&protocol));
            if st != ZmidiStatus::Ok { return Ok(()); }
            let mut out = [0u32; 8];
            for b in bytes {
                let mut n = 0usize;
                let st = unsafe { zmidi_bytes_to_ump_push(&mut r, b, out.as_mut_ptr(), cap, &mut n) };
                if cap < ZMIDI_BYTES_TO_UMP_MAX_WORDS {
                    prop_assert_eq!(st, ZmidiStatus::BufferTooSmall);
                } else {
                    prop_assert_eq!(st, ZmidiStatus::Ok);
                    prop_assert!(n <= ZMIDI_BYTES_TO_UMP_MAX_WORDS);
                }
            }
        });
    }

    /// REQ-FFI-IO-007 (OPT-035): one zmidi_bytes_to_ump_feed call over many bytes gives the same
    /// words as one zmidi_bytes_to_ump_push for each byte, for any stream, protocol and buffer;
    /// a call reads every byte when the buffer has room, and never cuts a UMP.
    #[test]
    fn prop_bytes_to_ump_feed_equals_push() {
        proptest!(|(protocol in 1u32..3, bytes in proptest::collection::vec(any::<u8>(), 0..200), cap in 0usize..40)| {
            let mut a = ZmidiBytesToUmp { _opaque: [0; ZMIDI_BYTES_TO_UMP_WORDS] };
            let mut b = ZmidiBytesToUmp { _opaque: [0; ZMIDI_BYTES_TO_UMP_WORDS] };
            prop_assert_eq!(unsafe { zmidi_bytes_to_ump_init(&mut a, 3, protocol) }, ZmidiStatus::Ok);
            prop_assert_eq!(unsafe { zmidi_bytes_to_ump_init(&mut b, 3, protocol) }, ZmidiStatus::Ok);
            let mut want = Vec::new();
            for &x in &bytes {
                let mut out = [0u32; ZMIDI_BYTES_TO_UMP_MAX_WORDS];
                let mut n = 0usize;
                prop_assert_eq!(unsafe { zmidi_bytes_to_ump_push(&mut a, x, out.as_mut_ptr(), out.len(), &mut n) }, ZmidiStatus::Ok);
                want.extend_from_slice(&out[..n]);
            }
            let mut got = Vec::new();
            let mut out = vec![0u32; cap.max(1)];
            let mut at = 0;
            loop {
                let (mut read, mut n) = (99usize, 99usize);
                let st = unsafe { zmidi_bytes_to_ump_feed(&mut b, bytes[at..].as_ptr(), bytes.len() - at, out.as_mut_ptr(), cap, &mut read, &mut n) };
                if cap < ZMIDI_BYTES_TO_UMP_MAX_WORDS {
                    prop_assert_eq!(st, ZmidiStatus::BufferTooSmall);
                    prop_assert_eq!((read, n), (99, 99));
                    return Ok(());
                }
                prop_assert_eq!(st, ZmidiStatus::Ok);
                prop_assert!(n <= cap && read <= bytes.len() - at);
                prop_assert!(read > 0 || at == bytes.len());
                got.extend_from_slice(&out[..n]);
                at += read;
                if at == bytes.len() { break; }
                // Stopped early: the buffer had less than one byte of room left.
                prop_assert!(cap - n < ZMIDI_BYTES_TO_UMP_MAX_WORDS);
            }
            prop_assert_eq!(got, want);
        });
    }

    /// REQ-FFI-ABI-001, REQ-FFI-IO-004: any words and any buffer size give a status and never more
    /// bytes than the buffer holds; no panic.
    #[test]
    fn prop_ump_to_bytes_never_panics() {
        proptest!(|(running in any::<bool>(), words in proptest::collection::vec(any::<u32>(), 0..5), cap in 0usize..16)| {
            let mut w = ZmidiUmpToBytes { _opaque: [0; ZMIDI_UMP_TO_BYTES_WORDS] };
            prop_assert_eq!(unsafe { zmidi_ump_to_bytes_init(&mut w, running) }, ZmidiStatus::Ok);
            let mut out = [0u8; 16];
            let mut n = 0usize;
            let st = unsafe { zmidi_ump_to_bytes_write(&mut w, words.as_ptr(), words.len(), out.as_mut_ptr(), cap, &mut n) };
            prop_assert!(st == ZmidiStatus::Ok || st == ZmidiStatus::BufferTooSmall);
            if st == ZmidiStatus::Ok { prop_assert!(n <= cap); }
        });
    }

    /// REQ-FFI-IO-005: the storage holds the Rust type on this target (the build checks it too).
    #[test]
    fn storage_holds_the_rust_types() {
        assert!(zmidi_bytes_to_ump_size() <= core::mem::size_of::<ZmidiBytesToUmp>());
        assert!(zmidi_ump_to_bytes_size() <= core::mem::size_of::<ZmidiUmpToBytes>());
    }
}

#[cfg(feature = "ump")]
mod ump {
    use proptest::prelude::*;
    use zmidi::status::ZmidiStatus;
    use zmidi::ump::*;

    /// REQ-FFI-ABI-001, REQ-FFI-UMP-002 to -004: any words give a status and no panic; a parse that
    /// succeeds gives the Table 4 word count.
    #[test]
    fn prop_ump_functions_never_panic() {
        proptest!(|(words in proptest::collection::vec(any::<u32>(), 1..6), cap in 0usize..6, g in 0u8..20, c in 0u8..20, n in any::<u8>(), v in any::<u32>())| {
            let mut info = ZmidiUmpInfo { message_type: 0, word_count: 0, has_group: false, group: 0, has_status: false, status: 0, has_channel: false, channel: 0, decoded: false, words: [0; 4] };
            let st = unsafe { zmidi_ump_parse(words.as_ptr(), words.len(), &mut info) };
            let need = zmidi_ump_word_count(words[0]);
            prop_assert_eq!(st == ZmidiStatus::Ok, words.len() >= need);
            if st == ZmidiStatus::Ok { prop_assert_eq!(usize::from(info.word_count), need); }
            let mut out = [0u32; 6];
            let mut count = 0usize;
            let _ = unsafe { zmidi_ump_midi2_to_midi1(words.as_ptr(), words.len(), out.as_mut_ptr(), cap, &mut count) };
            prop_assert!(count <= cap);
            let mut w = [0u32; 2];
            let st = unsafe { zmidi_ump_midi2_control_change(g, c, n, v, w.as_mut_ptr()) };
            prop_assert_eq!(st == ZmidiStatus::Ok, g < 16 && c < 16 && n < 128);
        });
    }
}

#[cfg(feature = "ble")]
mod ble {
    use proptest::prelude::*;
    use zmidi::ble::*;
    use zmidi::status::ZmidiStatus;

    /// REQ-FFI-ABI-001, REQ-FFI-BLE-002, -004: any packet and any UMPs give a status and no panic;
    /// the counts stay inside the buffers.
    #[test]
    fn prop_ble_functions_never_panic() {
        proptest!(|(packet in proptest::collection::vec(any::<u8>(), 0..40), mtu in 0u16..80, words in proptest::collection::vec((any::<u64>(), 1u8..5, any::<[u32; 4]>()), 0..6), cap in 0usize..64)| {
            let mut r = ZmidiBleReader { _opaque: [0; ZMIDI_BLE_READER_WORDS] };
            prop_assert_eq!(unsafe { zmidi_ble_reader_init(&mut r, 0, 2) }, ZmidiStatus::Ok);
            let mut recs = [ZmidiBleUmp { timestamp_ms: 0, word_count: 0, words: [0; 4] }; 40];
            let (mut n, mut d) = (0usize, 0usize);
            let st = unsafe { zmidi_ble_read(&mut r, packet.as_ptr(), packet.len(), recs.as_mut_ptr(), recs.len(), &mut n, &mut d) };
            prop_assert_eq!(st, ZmidiStatus::Ok);
            prop_assert!(n <= packet.len());
            let mut w = ZmidiBleWriter { _opaque: [0; ZMIDI_BLE_WRITER_WORDS] };
            if unsafe { zmidi_ble_writer_init(&mut w, mtu) } != ZmidiStatus::Ok { return Ok(()); }
            let umps: Vec<ZmidiBleTimedUmp> = words.iter().map(|&(millis, word_count, words)| ZmidiBleTimedUmp { millis, word_count, words }).collect();
            let mut out = [0u8; 64];
            let (mut len, mut taken) = (0usize, 0usize);
            let st = unsafe { zmidi_ble_write_packet(&mut w, umps.as_ptr(), umps.len(), out.as_mut_ptr(), cap, &mut len, &mut taken) };
            prop_assert!(st == ZmidiStatus::Ok || st == ZmidiStatus::BufferTooSmall);
            if st == ZmidiStatus::Ok { prop_assert!(len <= cap && taken <= umps.len()); }
        });
    }
}

/// The record size of zmidi_pe_write_pending: a record header (3), the MIDI-CI header and the
/// SysEx framing (2 + 13), the Chunk fields (9) and 256 header bytes.
#[cfg(feature = "ci")]
#[test]
fn pe_pending_buffer_size() {
    assert_eq!(zmidi::pe::ZMIDI_PE_PENDING_MAX, 3 + 2 + 13 + 9 + 256);
}

#[cfg(feature = "file")]
mod file {
    use proptest::prelude::*;
    use zmidi::file::*;
    use zmidi::status::ZmidiStatus;

    /// REQ-FFI-ABI-001, REQ-FFI-FILE-001, -003: any bytes after a File Header read to the end with a
    /// status and no panic; any events give a status and no panic.
    #[test]
    fn prop_clip_functions_never_panic() {
        proptest!(|(body in proptest::collection::vec(any::<u8>(), 0..64), events in proptest::collection::vec((0u64..1000, 1usize..5, any::<[u32; 4]>()), 0..8), cap in 0usize..256)| {
            let mut data = b"SMF2CLIP".to_vec();
            data.extend_from_slice(&body);
            let mut r = ZmidiClipReader { _opaque: [0; ZMIDI_CLIP_READER_WORDS] };
            let mut e = ZmidiClipError::None;
            prop_assert_eq!(unsafe { zmidi_clip_reader_init(&mut r, data.as_ptr(), data.len(), &mut e) }, ZmidiStatus::Ok);
            let mut ev = ZmidiClipEvent { section: 0, ticks: 0, word_count: 0, words: [0; 4] };
            let mut has = false;
            for _ in 0..80 {
                let st = unsafe { zmidi_clip_reader_next(&mut r, &mut ev, &mut has, &mut e) };
                if st != ZmidiStatus::Ok || !has { break; }
            }
            let mut buf = vec![0u8; cap];
            let mut w = ZmidiClipWriter { _opaque: [0; ZMIDI_CLIP_WRITER_WORDS] };
            let c = ZmidiClipConfig { ticks_per_quarter_note: 96, group: 0, has_tempo: false, tempo_10ns: 0, has_time_signature: false, numerator: 0, denominator: 0, thirty_second_notes: 0, profiles: core::ptr::null(), profile_count: 0 };
            if unsafe { zmidi_clip_writer_init(&mut w, buf.as_mut_ptr(), buf.len(), &c, &mut e) } != ZmidiStatus::Ok { return Ok(()); }
            let _ = unsafe { zmidi_clip_writer_start(&mut w, 0, false, 0, 0, &mut e) };
            for (tick, n, words) in events {
                let _ = unsafe { zmidi_clip_writer_event(&mut w, tick, words.as_ptr(), n, &mut e) };
            }
            let mut len = 0usize;
            if unsafe { zmidi_clip_writer_finish(&mut w, 1000, &mut len, &mut e) } == ZmidiStatus::Ok {
                prop_assert!(len <= cap);
            }
        });
    }
}

#[cfg(feature = "ci")]
mod ci {
    use proptest::prelude::*;
    use zmidi::ci::*;
    use zmidi::status::ZmidiStatus;

    fn config() -> ZmidiCiConfig {
        ZmidiCiConfig {
            manufacturer: [0, 0x21, 0x7F],
            family: 1,
            model: 2,
            revision: [0, 0, 1, 0],
            max_sysex_size: 512,
            has_function_block: false,
            function_block: 0,
            first_group: 0,
            group_count: 1,
            channels_in_use: 1,
            product_instance_id: core::ptr::null(),
            profiles: false,
            property_exchange: true,
            process_inquiry: true,
            sysex_framing: false,
            resource_list: b"[]".as_ptr(),
            resource_list_len: 2,
        }
    }

    /// REQ-FFI-ABI-001, REQ-FFI-CI-002: any message (random, or a MIDI-CI header with random
    /// fields) gives a status, replies inside the buffer and no panic.
    #[test]
    fn prop_ci_receive_never_panics() {
        proptest!(|(head in proptest::collection::vec(any::<u8>(), 0..14), body in proptest::collection::vec(any::<u8>(), 0..80), ci_header in any::<bool>(), seed in any::<u32>())| {
            let mut dev = ZmidiCi { _opaque: [0; ZMIDI_CI_WORDS] };
            prop_assert_eq!(unsafe { zmidi_ci_init(&mut dev, &config(), seed) }, ZmidiStatus::Ok);
            let mut msg = if ci_header { vec![0x7E, 0x7F, 0x0D] } else { Vec::new() };
            msg.extend_from_slice(&head);
            msg.extend_from_slice(&body);
            let mut out = [0u8; ZMIDI_CI_REPLY_BUFFER];
            let mut n = 0usize;
            let mut ev = core::mem::MaybeUninit::<ZmidiCiEvent>::uninit();
            let st = unsafe { zmidi_ci_receive(&mut dev, 0, msg.as_ptr(), msg.len(), 0, out.as_mut_ptr(), out.len(), &mut n, ev.as_mut_ptr()) };
            prop_assert_eq!(st, ZmidiStatus::Ok);
            prop_assert!(n <= out.len());
            let mut scratch = [0u8; 256];
            let st = unsafe { zmidi_ci_pe_reply(&mut dev, 200, core::ptr::null(), 0, scratch.as_mut_ptr(), scratch.len(), out.as_mut_ptr(), out.len(), &mut n) };
            prop_assert!(st == ZmidiStatus::Ok || st == ZmidiStatus::NoPending);
        });
    }

    /// REQ-FFI-ABI-001, REQ-FFI-CI-022: zmidi_ci_pe_end gives Ok or invalid-argument for any peer
    /// and Request ID, and a Request ID over 0x7F gives invalid-argument.
    #[test]
    fn prop_ci_pe_end_never_panics() {
        proptest!(|(peer in any::<u32>(), id in any::<u8>())| {
            let mut dev = ZmidiCi { _opaque: [0; ZMIDI_CI_WORDS] };
            prop_assert_eq!(unsafe { zmidi_ci_init(&mut dev, &config(), 5) }, ZmidiStatus::Ok);
            let st = unsafe { zmidi_ci_pe_end(&mut dev, peer, id) };
            prop_assert!(st == ZmidiStatus::Ok || st == ZmidiStatus::InvalidArgument);
            if id > 0x7F {
                prop_assert_eq!(st, ZmidiStatus::InvalidArgument);
            }
        });
    }

    /// REQ-FFI-ABI-001, REQ-FFI-CI-010 to REQ-FFI-CI-013: the Profile and Process Inquiry calls
    /// give a status and records inside the buffer, and do not panic, for any Device ID, Group,
    /// Profile ID, Channel count, bitmap and peer.
    #[test]
    fn prop_ci_profile_and_process_calls_never_panic() {
        proptest!(|(device_id in any::<u8>(), group in 0u8..20, profile in any::<[u8; 5]>(), channels in any::<u16>(), bits in any::<[u8; 4]>(), peer in any::<u32>(), enabled in any::<bool>(), capacity in 0usize..80)| {
            let mut dev = ZmidiCi { _opaque: [0; ZMIDI_CI_WORDS] };
            let mut c = config();
            c.profiles = true;
            prop_assert_eq!(unsafe { zmidi_ci_init(&mut dev, &c, 7) }, ZmidiStatus::Ok);
            let mut out = [0u8; 256];
            let mut n = 0usize;
            let entry = ZmidiCiProfile { device_id, group, profile, enabled, channels, max_channels: channels };
            let report = ZmidiCiReport { data_control: bits[0], system: bits[1], channel: bits[2], note: bits[3] };
            let p = profile.as_ptr();
            let o = out.as_mut_ptr();
            let all = [
                unsafe { zmidi_ci_add_profile(&mut dev, &entry, o, capacity, &mut n) },
                unsafe { zmidi_ci_set_profile_state(&mut dev, device_id, group, p, enabled, channels, o, capacity, &mut n) },
                unsafe { zmidi_ci_remove_profile(&mut dev, device_id, group, p, o, capacity, &mut n) },
                unsafe { zmidi_ci_profile_inquiry(&mut dev, peer, group, device_id, o, capacity, &mut n) },
                unsafe { zmidi_ci_set_profile_on(&mut dev, peer, group, device_id, p, channels, channels, o, capacity, &mut n) },
                unsafe { zmidi_ci_profile_details_inquiry(&mut dev, peer, group, device_id, p, bits[0], o, capacity, &mut n) },
                unsafe { zmidi_ci_profile_details_reply(&mut dev, peer, group, device_id, p, bits[0], p, 5, o, capacity, &mut n) },
                unsafe { zmidi_ci_profile_specific_data(&mut dev, peer, group, device_id, p, p, 5, o, capacity, &mut n) },
                unsafe { zmidi_ci_message_report_inquiry(&mut dev, peer, group, device_id, &report, o, capacity, &mut n) },
                unsafe { zmidi_ci_message_report_reply(&mut dev, peer, group, device_id, &report, &report, o, capacity, &mut n) },
                unsafe { zmidi_ci_message_report_end(&mut dev, peer, group, device_id, o, capacity, &mut n) },
                unsafe { zmidi_ci_abort_report(&mut dev, peer, group, device_id, enabled, o, capacity, &mut n) },
            ];
            for st in all {
                prop_assert!(st as u32 <= ZmidiStatus::NotSupported as u32);
                if st == ZmidiStatus::Ok {
                    prop_assert!(n <= capacity);
                }
                // REQ-FFI-CI-016, -017: a Group above 15 is an invalid argument for each call.
                if group > 15 {
                    prop_assert_eq!(st, ZmidiStatus::InvalidArgument);
                }
            }
            // The status rules of add_profile with a valid Group: channel counts over 14 bits are
            // invalid; otherwise a buffer below ZMIDI_CI_REPORT_BUFFER is too small.
            if group <= 15 && (device_id <= 0x0F || device_id == 0x7E || device_id == 0x7F) {
                let st = all[0];
                if channels > 0x3FFF || profile.iter().any(|b| *b > 0x7F) {
                    // REQ-FFI-CI-016: a Channel count over 14 bits or a Profile ID byte over 0x7F.
                    prop_assert_eq!(st, ZmidiStatus::InvalidArgument);
                } else if capacity < ZMIDI_CI_REPORT_BUFFER {
                    prop_assert_eq!(st, ZmidiStatus::BufferTooSmall);
                }
            }
            let _ = unsafe { zmidi_ci_report_open(&mut dev, peer) };
        });
    }
}

#[cfg(feature = "ci")]
mod pe {
    use proptest::prelude::*;
    use zmidi::ci::*;
    use zmidi::pe::*;
    use zmidi::status::ZmidiStatus;

    fn config() -> ZmidiCiConfig {
        ZmidiCiConfig {
            manufacturer: [0x00, 0x21, 0x7F],
            family: 1,
            model: 2,
            revision: [0, 0, 1, 0],
            max_sysex_size: 512,
            has_function_block: false,
            function_block: 0,
            first_group: 0,
            group_count: 1,
            channels_in_use: 1,
            product_instance_id: core::ptr::null(),
            profiles: false,
            property_exchange: true,
            process_inquiry: false,
            sysex_framing: false,
            resource_list: b"[]".as_ptr(),
            resource_list_len: 2,
        }
    }

    fn resource(name: &'static core::ffi::CStr) -> ZmidiPeResource {
        ZmidiPeResource {
            resource: name.as_ptr().cast(),
            can_get: true,
            can_set: 1,
            can_subscribe: true,
            require_res_id: false,
            fc_on_get: false,
            fc_on_set: false,
            can_paginate: false,
            write_defaults: false,
            encodings: 3,
            media_type: core::ptr::null(),
            schema: core::ptr::null(),
            schema_len: 0,
        }
    }

    /// REQ-FFI-PE-009, REQ-FFI-ABI-001: any Resource text, mediaType, capability values and schema
    /// give Ok or invalid-argument, and no panic.
    #[test]
    fn prop_pe_init_never_panics() {
        proptest!(|(name in proptest::collection::vec(1u8..=255, 0..40), media in proptest::collection::vec(1u8..=255, 0..80), schema in proptest::collection::vec(any::<u8>(), 0..600), can_set in any::<u8>(), encodings in any::<u8>(), flags in any::<[bool; 8]>(), has_media in any::<bool>(), count in 0usize..20)| {
            let mut name = name;
            name.push(0);
            let mut media = media;
            media.push(0);
            let r = ZmidiPeResource {
                resource: name.as_ptr(),
                can_get: flags[0],
                can_set,
                can_subscribe: flags[1],
                require_res_id: flags[2],
                fc_on_get: flags[3],
                fc_on_set: flags[4],
                can_paginate: flags[5],
                write_defaults: flags[6],
                encodings,
                media_type: if has_media { media.as_ptr() } else { core::ptr::null() },
                schema: if flags[7] { schema.as_ptr() } else { core::ptr::null() },
                schema_len: if flags[7] { schema.len() } else { 0 },
            };
            let table = vec![r; count];
            let mut pe = Box::new(ZmidiPe { _opaque: [0; ZMIDI_PE_WORDS] });
            let st = unsafe { zmidi_pe_init(&mut *pe, table.as_ptr(), count) };
            prop_assert!(st == ZmidiStatus::Ok || st == ZmidiStatus::InvalidArgument);
            if count > ZMIDI_PE_RESOURCES_MAX {
                prop_assert_eq!(st, ZmidiStatus::InvalidArgument);
            }
            if st == ZmidiStatus::Ok {
                let mut out = vec![0u8; 16 * 1024];
                let mut n = 0usize;
                prop_assert_eq!(unsafe { zmidi_pe_resource_list(&mut *pe, out.as_mut_ptr(), out.len(), &mut n) }, ZmidiStatus::Ok);
                prop_assert!(n >= 2 && out[0] == b'[' && out[n - 1] == b']');
            }
        });
    }

    /// REQ-FFI-PE-008, REQ-FFI-ABI-001: any received Property Exchange message (a MIDI-CI header
    /// with random fields), and then every engine call with random values, gives a status, records
    /// inside the buffer and no panic.
    #[test]
    fn prop_pe_calls_never_panic() {
        proptest!(|(sub_id2 in 0x34u8..0x40, src in any::<[u8; 4]>(), body in proptest::collection::vec(any::<u8>(), 0..120), peer in any::<u32>(), request_id in any::<u8>(), role in 0u8..3, status in any::<u16>(), value in any::<u8>(), len in 0usize..2000, index in 0usize..6, capacity in 0usize..600, now in any::<u32>())| {
            let mut dev = Box::new(ZmidiCi { _opaque: [0; ZMIDI_CI_WORDS] });
            prop_assert_eq!(unsafe { zmidi_ci_init(&mut *dev, &config(), 5) }, ZmidiStatus::Ok);
            let table = [resource(c"DeviceInfo"), resource(c"X-Sub")];
            let mut pe = Box::new(ZmidiPe { _opaque: [0; ZMIDI_PE_WORDS] });
            prop_assert_eq!(unsafe { zmidi_pe_init(&mut *pe, table.as_ptr(), 2) }, ZmidiStatus::Ok);
            let muid = unsafe { zmidi_ci_muid(&mut *dev) };
            let mut msg = vec![0x7E, 0x7F, 0x0D, sub_id2, 0x02];
            msg.extend_from_slice(&src.map(|b| b & 0x7F));
            msg.extend_from_slice(&[(muid & 0x7F) as u8, ((muid >> 7) & 0x7F) as u8, ((muid >> 14) & 0x7F) as u8, ((muid >> 21) & 0x7F) as u8]);
            msg.extend_from_slice(&body);
            let now = u64::from(now) * 1000;
            let mut out = vec![0u8; 4096];
            let mut n = 0usize;
            let mut ev = core::mem::MaybeUninit::<ZmidiCiEvent>::uninit();
            let st = unsafe { zmidi_ci_receive(&mut *dev, 0, msg.as_ptr(), msg.len(), now, out.as_mut_ptr(), out.len(), &mut n, ev.as_mut_ptr()) };
            prop_assert_eq!(st, ZmidiStatus::Ok);
            let mut scratch = vec![0u8; capacity];
            let mut pev = core::mem::MaybeUninit::<ZmidiPeEvent>::uninit();
            let st = unsafe { zmidi_pe_receive(&mut *pe, &mut *dev, now, scratch.as_mut_ptr(), scratch.len(), pev.as_mut_ptr()) };
            prop_assert!(st == ZmidiStatus::Ok || st == ZmidiStatus::BufferTooSmall || st == ZmidiStatus::Rejected);
            let (p, d) = (&mut *pe as *mut ZmidiPe, &mut *dev as *mut ZmidiCi);
            let o = out.as_mut_ptr();
            let data = vec![b'x'; len];
            let h = ZmidiPeHeader {
                resource: c"DeviceInfo".as_ptr().cast(),
                res_id: core::ptr::null(),
                media_type: core::ptr::null(),
                has_offset: value & 1 != 0,
                offset: u32::from(value),
                limit: u32::from(value),
                set_partial: value & 2 != 0,
                encoding: value % 4,
                length: len,
            };
            let mut id = 0u8;
            let mut missing = false;
            let mut room = 0usize;
            let mut sub = core::mem::MaybeUninit::<ZmidiPeSubscription>::uninit();
            let mut calls = Vec::new();
            for _ in 0..2 {
                calls.extend([
                    unsafe { zmidi_pe_poll(p, d, now + u64::from(value) * 100_000, pev.as_mut_ptr()) },
                    unsafe { zmidi_pe_reply(p, d, peer, request_id, status, value % 4, core::ptr::null(), i64::from(value) - 1, -1, len, now) },
                    unsafe { zmidi_pe_data_room(p, d, peer, request_id, role, &mut room) },
                    unsafe { zmidi_pe_send(p, d, peer, request_id, role, data.as_ptr(), data.len(), value & 1 != 0, now, o, capacity, &mut n) },
                    unsafe { zmidi_pe_reply_status(p, d, peer, request_id, status, now) },
                    unsafe { zmidi_pe_update(p, d, index, value % 6, value % 4, len, now, &mut id) },
                    unsafe { zmidi_pe_request(p, d, peer, value % 3, &h, &table[0], now, &mut id, &mut missing) },
                    unsafe { zmidi_pe_request_resource_list(p, d, peer, now, o, capacity, &mut n, &mut id) },
                    unsafe { zmidi_pe_subscribe(p, d, peer, &h, core::ptr::null(), now, &mut id, &mut missing) },
                    unsafe { zmidi_pe_unsubscribe(p, d, index, now, &mut id) },
                    unsafe { zmidi_pe_refresh(p, index) },
                    unsafe { zmidi_pe_wait(p, peer, request_id, role, u32::from(status)) },
                    unsafe { zmidi_pe_corrupt(p, peer, request_id, role) },
                    unsafe { zmidi_pe_abort(p, d, peer, request_id, role) },
                    unsafe { zmidi_pe_write_pending(p, d, now, o, capacity, &mut n) },
                    unsafe { zmidi_pe_subscription(p, index, sub.as_mut_ptr()) },
                    unsafe { zmidi_pe_take_ended(p, sub.as_mut_ptr()) },
                    unsafe { zmidi_pe_resource_list(p, o, capacity, &mut n) },
                ]);
                let _ = unsafe { zmidi_pe_last_error(p, core::ptr::null_mut()) };
            }
            for st in calls {
                prop_assert!(st as u32 <= ZmidiStatus::NotSupported as u32);
            }
            prop_assert!(n <= capacity.max(out.len()));
        });
    }
}

#[cfg(feature = "net")]
mod net {
    use proptest::prelude::*;
    use zmidi::net::*;
    use zmidi::status::ZmidiStatus;

    /// REQ-FFI-ABI-001, REQ-FFI-NET-002, -003: any payload (random, or with the "MIDI" signature
    /// and random Commands) gives a status, lengths inside the arrays and no panic.
    #[test]
    fn prop_net_receive_never_panics() {
        proptest!(|(body in proptest::collection::vec(any::<u8>(), 0..120), signed in any::<bool>(), ev_cap in 0usize..8)| {
            let mut host = Box::new(ZmidiNetHost { _opaque: [0; ZMIDI_NET_HOST_WORDS] });
            prop_assert_eq!(unsafe { zmidi_net_host_init(&mut *host, c"h".as_ptr().cast(), c"p".as_ptr().cast()) }, ZmidiStatus::Ok);
            let mut payload = if signed { b"MIDI".to_vec() } else { Vec::new() };
            payload.extend_from_slice(&body);
            let from = ZmidiNetAddr { family: 4, ip: [10, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], port: 5000 };
            let mut events = vec![ZmidiNetEvent { kind: ZmidiNetEventKind::None, peer: from, code: 0, value: 0, word_offset: 0, word_count: 0 }; ev_cap];
            let mut words = vec![0u32; 64];
            let mut out = vec![0u8; 4096];
            let mut b = ZmidiNetBuffers { events: events.as_mut_ptr(), events_capacity: ev_cap, events_len: 0, words: words.as_mut_ptr(), words_capacity: 64, words_len: 0, out: out.as_mut_ptr(), out_capacity: out.len(), out_len: 0 };
            let st = unsafe { zmidi_net_host_receive(&mut *host, 0, &from, payload.as_ptr(), payload.len(), &mut b) };
            prop_assert!(st == ZmidiStatus::Ok || st == ZmidiStatus::BufferTooSmall);
            prop_assert!(b.events_len <= ev_cap && b.words_len <= 64 && b.out_len <= out.len());
        });
    }
}

#[cfg(feature = "net-auth")]
mod net_auth {
    use proptest::prelude::*;
    use zmidi::net::*;
    use zmidi::status::ZmidiStatus;

    const HOST: ZmidiNetAddr = ZmidiNetAddr {
        family: 4,
        ip: [10, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        port: 5673,
    };
    const CLIENT: ZmidiNetAddr = ZmidiNetAddr {
        family: 4,
        ip: [10, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        port: 50000,
    };

    fn creds(secret: &[u8], users: &[ZmidiNetUser], random: &[u8]) -> ZmidiNetCredentials {
        ZmidiNetCredentials {
            shared_secret: secret.as_ptr(),
            shared_secret_len: secret.len(),
            users: users.as_ptr(),
            user_count: users.len(),
            random: random.as_ptr(),
            random_len: random.len(),
            random_used: 0,
        }
    }

    fn user(name: &'static [u8], password: &'static [u8]) -> ZmidiNetUser {
        ZmidiNetUser {
            username: name.as_ptr(),
            username_len: name.len(),
            password: password.as_ptr(),
            password_len: password.len(),
        }
    }

    /// One receive into fresh arrays; gives the status and the events.
    fn rx(
        f: impl FnOnce(&mut ZmidiNetBuffers) -> ZmidiStatus,
    ) -> (ZmidiStatus, Vec<ZmidiNetEvent>) {
        let none = ZmidiNetEvent {
            kind: ZmidiNetEventKind::None,
            peer: HOST,
            code: 0,
            value: 0,
            word_offset: 0,
            word_count: 0,
        };
        let mut events = vec![none; 16];
        let mut words = vec![0u32; 512];
        let mut out = vec![0u8; 4096];
        let mut b = ZmidiNetBuffers {
            events: events.as_mut_ptr(),
            events_capacity: events.len(),
            events_len: 0,
            words: words.as_mut_ptr(),
            words_capacity: words.len(),
            words_len: 0,
            out: out.as_mut_ptr(),
            out_capacity: out.len(),
            out_len: 0,
        };
        let st = f(&mut b);
        events.truncate(b.events_len);
        (st, events)
    }

    /// The payloads of one write_pending loop.
    fn pending(
        mut f: impl FnMut(&mut [u8], &mut ZmidiNetAddr, &mut usize) -> ZmidiStatus,
    ) -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        loop {
            let mut buf = vec![0u8; ZMIDI_NET_MAX_PAYLOAD];
            let (mut to, mut len) = (HOST, 0usize);
            assert_eq!(f(&mut buf, &mut to, &mut len), ZmidiStatus::Ok);
            if len == 0 {
                return v;
            }
            buf.truncate(len);
            v.push(buf);
        }
    }

    /// REQ-FFI-NET-006, -007, -008: M2-124 6.9 example through the C ABI. The Host writes
    /// Invitation Reply: Authentication Required with the CryptoNonce "nUWrn*@#$hjfwnkL" (from
    /// the random bytes of the program), the Client writes Invitation with Authentication (0x02,
    /// pl 8, Table 18) with the digest of the example for the shared secret "5483", and both
    /// sides are Established with the shared secret method.
    #[test]
    fn net_auth_shared_secret_example_6_9() {
        const NONCE: &[u8; 16] = b"nUWrn*@#$hjfwnkL";
        const DIGEST: [u8; 32] = [
            0x67, 0x6E, 0xBE, 0x82, 0x58, 0x7C, 0xEC, 0xA8, 0xF8, 0x2F, 0xC3, 0x33, 0xD7, 0x87,
            0x95, 0x1E, 0xEC, 0x2B, 0x00, 0xAD, 0x31, 0x61, 0x3C, 0xC4, 0xDB, 0x18, 0xCF, 0x27,
            0x37, 0x3A, 0xFB, 0x82,
        ];
        let random: Vec<u8> = NONCE.iter().map(|b| b - 32).collect();
        let mut hc = creds(b"5483", &[], &random);
        let mut cc = creds(b"5483", &[], &[]);
        let mut host = Box::new(ZmidiNetHost {
            _opaque: [0; ZMIDI_NET_HOST_WORDS],
        });
        let mut client = Box::new(ZmidiNetClient {
            _opaque: [0; ZMIDI_NET_CLIENT_WORDS],
        });
        unsafe {
            assert_eq!(
                zmidi_net_host_init_auth(
                    &mut *host,
                    c"Host".as_ptr().cast(),
                    c"P1".as_ptr().cast(),
                    ZMIDI_NET_AUTH_SHARED_SECRET
                ),
                ZmidiStatus::Ok
            );
            assert_eq!(
                zmidi_net_client_init_auth(
                    &mut *client,
                    c"C".as_ptr().cast(),
                    c"c1".as_ptr().cast(),
                    ZMIDI_NET_AUTH_SHARED_SECRET
                ),
                ZmidiStatus::Ok
            );
            assert_eq!(
                zmidi_net_client_connect(&mut *client, 0, &HOST),
                ZmidiStatus::Ok
            );
        }
        // Invitation (0x01) with Capabilities D0 = 1.
        let inv = pending(|b, to, n| unsafe {
            zmidi_net_client_write_pending(&mut *client, 0, b.as_mut_ptr(), b.len(), to, n)
        });
        assert_eq!(inv.len(), 1);
        assert_eq!(&inv[0][4..5], &[0x01]);
        assert_eq!(inv[0][7] & 0x03, 0x01);
        let (st, ev) = rx(|b| unsafe {
            zmidi_net_host_receive_auth(
                &mut *host,
                0,
                &CLIENT,
                inv[0].as_ptr(),
                inv[0].len(),
                &mut hc,
                b,
            )
        });
        assert_eq!((st, ev.len()), (ZmidiStatus::Ok, 0));
        assert_eq!(hc.random_used, 16);
        // Invitation Reply: Authentication Required (0x12), state 0x00, the CryptoNonce first.
        let ar = pending(|b, to, n| unsafe {
            zmidi_net_host_write_pending(&mut *host, 0, b.as_mut_ptr(), b.len(), to, n)
        });
        assert_eq!(ar.len(), 1);
        assert_eq!((ar[0][4], ar[0][7]), (0x12, 0x00));
        assert_eq!(&ar[0][8..24], NONCE);
        let (st, ev) = rx(|b| unsafe {
            zmidi_net_client_receive_auth(
                &mut *client,
                10,
                &HOST,
                ar[0].as_ptr(),
                ar[0].len(),
                &mut cc,
                b,
            )
        });
        assert_eq!(st, ZmidiStatus::Ok);
        assert_eq!(
            ev.iter()
                .map(|e| (e.kind, e.code, e.value))
                .collect::<Vec<_>>(),
            [(
                ZmidiNetEventKind::AuthRequested,
                u32::from(ZMIDI_NET_AUTH_SHARED_SECRET),
                0
            )]
        );
        // Invitation with Authentication: 0x02, pl 8, csd 0 0, the digest of the example.
        let ia = pending(|b, to, n| unsafe {
            zmidi_net_client_write_pending(&mut *client, 10, b.as_mut_ptr(), b.len(), to, n)
        });
        assert_eq!(ia.len(), 1);
        assert_eq!(&ia[0][..8], b"MIDI\x02\x08\x00\x00");
        assert_eq!(&ia[0][8..40], &DIGEST);
        let (st, ev) = rx(|b| unsafe {
            zmidi_net_host_receive_auth(
                &mut *host,
                20,
                &CLIENT,
                ia[0].as_ptr(),
                ia[0].len(),
                &mut hc,
                b,
            )
        });
        assert_eq!(st, ZmidiStatus::Ok);
        assert_eq!(
            ev.iter().map(|e| (e.kind, e.code)).collect::<Vec<_>>(),
            [(
                ZmidiNetEventKind::Established,
                u32::from(ZMIDI_NET_AUTH_SHARED_SECRET)
            )]
        );
        let acc = pending(|b, to, n| unsafe {
            zmidi_net_host_write_pending(&mut *host, 20, b.as_mut_ptr(), b.len(), to, n)
        });
        assert_eq!(acc[0][4], 0x10);
        let (_, ev) = rx(|b| unsafe {
            zmidi_net_client_receive_auth(
                &mut *client,
                30,
                &HOST,
                acc[0].as_ptr(),
                acc[0].len(),
                &mut cc,
                b,
            )
        });
        assert_eq!(
            ev.iter().map(|e| (e.kind, e.code)).collect::<Vec<_>>(),
            [(
                ZmidiNetEventKind::Established,
                u32::from(ZMIDI_NET_AUTH_SHARED_SECRET)
            )]
        );
    }

    /// REQ-FFI-NET-006, -007, -008: M2-124 6.10 example through the C ABI. The Client with
    /// Username "Rosa" and password "RPBqBno" answers the CryptoNonce "XI|~=NNRVaD;XCPL" with
    /// Invitation with User Authentication (0x03, Table 19) with the digest of the example and
    /// the Username; the Host finds "Rosa" in its table of two users.
    #[test]
    fn net_auth_user_example_6_10() {
        const NONCE: &[u8; 16] = b"XI|~=NNRVaD;XCPL";
        const DIGEST: [u8; 32] = [
            0x3A, 0x2D, 0x5E, 0xBE, 0xEF, 0x92, 0xE8, 0x46, 0x35, 0x35, 0xC2, 0x7F, 0xB4, 0xB7,
            0xB3, 0xE2, 0xD1, 0xAC, 0xF5, 0x7A, 0x84, 0x4C, 0x6D, 0x08, 0x08, 0xE0, 0x41, 0xC1,
            0x02, 0xB7, 0xFF, 0x1A,
        ];
        let random: Vec<u8> = NONCE.iter().map(|b| b - 32).collect();
        let table = [user(b"Ana", b"other"), user(b"Rosa", b"RPBqBno")];
        let me = [user(b"Rosa", b"RPBqBno")];
        let mut hc = creds(&[], &table, &random);
        hc.shared_secret = core::ptr::null();
        let mut cc = creds(&[], &me, &[]);
        cc.shared_secret = core::ptr::null();
        let mut host = Box::new(ZmidiNetHost {
            _opaque: [0; ZMIDI_NET_HOST_WORDS],
        });
        let mut client = Box::new(ZmidiNetClient {
            _opaque: [0; ZMIDI_NET_CLIENT_WORDS],
        });
        unsafe {
            assert_eq!(
                zmidi_net_host_init_auth(
                    &mut *host,
                    c"Host".as_ptr().cast(),
                    c"P1".as_ptr().cast(),
                    ZMIDI_NET_AUTH_USER
                ),
                ZmidiStatus::Ok
            );
            assert_eq!(
                zmidi_net_client_init_auth(
                    &mut *client,
                    c"C".as_ptr().cast(),
                    c"c1".as_ptr().cast(),
                    ZMIDI_NET_AUTH_USER
                ),
                ZmidiStatus::Ok
            );
            assert_eq!(
                zmidi_net_client_connect(&mut *client, 0, &HOST),
                ZmidiStatus::Ok
            );
        }
        let inv = pending(|b, to, n| unsafe {
            zmidi_net_client_write_pending(&mut *client, 0, b.as_mut_ptr(), b.len(), to, n)
        });
        assert_eq!(inv[0][7] & 0x03, 0x02);
        assert_eq!(
            rx(|b| unsafe {
                zmidi_net_host_receive_auth(
                    &mut *host,
                    0,
                    &CLIENT,
                    inv[0].as_ptr(),
                    inv[0].len(),
                    &mut hc,
                    b,
                )
            })
            .0,
            ZmidiStatus::Ok
        );
        let ar = pending(|b, to, n| unsafe {
            zmidi_net_host_write_pending(&mut *host, 0, b.as_mut_ptr(), b.len(), to, n)
        });
        assert_eq!((ar[0][4], ar[0][7]), (0x13, 0x00));
        assert_eq!(&ar[0][8..24], NONCE);
        let (_, ev) = rx(|b| unsafe {
            zmidi_net_client_receive_auth(
                &mut *client,
                10,
                &HOST,
                ar[0].as_ptr(),
                ar[0].len(),
                &mut cc,
                b,
            )
        });
        assert_eq!(
            (ev[0].kind, ev[0].code),
            (
                ZmidiNetEventKind::AuthRequested,
                u32::from(ZMIDI_NET_AUTH_USER)
            )
        );
        // 0x03, pl 9 (8 digest words and one Username word), the digest, then "Rosa".
        let ia = pending(|b, to, n| unsafe {
            zmidi_net_client_write_pending(&mut *client, 10, b.as_mut_ptr(), b.len(), to, n)
        });
        assert_eq!(&ia[0][..8], b"MIDI\x03\x09\x00\x00");
        assert_eq!(&ia[0][8..40], &DIGEST);
        assert_eq!(&ia[0][40..44], b"Rosa");
        let (_, ev) = rx(|b| unsafe {
            zmidi_net_host_receive_auth(
                &mut *host,
                20,
                &CLIENT,
                ia[0].as_ptr(),
                ia[0].len(),
                &mut hc,
                b,
            )
        });
        assert_eq!(
            ev.iter().map(|e| (e.kind, e.code)).collect::<Vec<_>>(),
            [(
                ZmidiNetEventKind::Established,
                u32::from(ZMIDI_NET_AUTH_USER)
            )]
        );
    }

    /// REQ-FFI-NET-008: a wrong password gives AuthFailed (method user, 1 failure) and no
    /// session; an unknown Username sets ZMIDI_NET_AUTH_USERNAME_NOT_FOUND.
    #[test]
    fn net_auth_wrong_password_fails() {
        for (name, pw, code) in [
            (&b"Rosa"[..], &b"wrong"[..], u32::from(ZMIDI_NET_AUTH_USER)),
            (
                &b"Nobody"[..],
                &b"RPBqBno"[..],
                u32::from(ZMIDI_NET_AUTH_USER) | ZMIDI_NET_AUTH_USERNAME_NOT_FOUND,
            ),
        ] {
            let random = [7u8; 32];
            let table = [user(b"Rosa", b"RPBqBno")];
            let me = [ZmidiNetUser {
                username: name.as_ptr(),
                username_len: name.len(),
                password: pw.as_ptr(),
                password_len: pw.len(),
            }];
            let mut hc = creds(&[], &table, &random);
            let mut cc = creds(&[], &me, &[]);
            let mut host = Box::new(ZmidiNetHost {
                _opaque: [0; ZMIDI_NET_HOST_WORDS],
            });
            let mut client = Box::new(ZmidiNetClient {
                _opaque: [0; ZMIDI_NET_CLIENT_WORDS],
            });
            unsafe {
                assert_eq!(
                    zmidi_net_host_init_auth(
                        &mut *host,
                        c"Host".as_ptr().cast(),
                        c"P1".as_ptr().cast(),
                        ZMIDI_NET_AUTH_USER
                    ),
                    ZmidiStatus::Ok
                );
                assert_eq!(
                    zmidi_net_client_init_auth(
                        &mut *client,
                        c"C".as_ptr().cast(),
                        c"c1".as_ptr().cast(),
                        ZMIDI_NET_AUTH_USER
                    ),
                    ZmidiStatus::Ok
                );
                assert_eq!(
                    zmidi_net_client_connect(&mut *client, 0, &HOST),
                    ZmidiStatus::Ok
                );
            }
            let inv = pending(|b, to, n| unsafe {
                zmidi_net_client_write_pending(&mut *client, 0, b.as_mut_ptr(), b.len(), to, n)
            });
            assert_eq!(
                rx(|b| unsafe {
                    zmidi_net_host_receive_auth(
                        &mut *host,
                        0,
                        &CLIENT,
                        inv[0].as_ptr(),
                        inv[0].len(),
                        &mut hc,
                        b,
                    )
                })
                .0,
                ZmidiStatus::Ok
            );
            let ar = pending(|b, to, n| unsafe {
                zmidi_net_host_write_pending(&mut *host, 0, b.as_mut_ptr(), b.len(), to, n)
            });
            assert_eq!(
                rx(|b| unsafe {
                    zmidi_net_client_receive_auth(
                        &mut *client,
                        10,
                        &HOST,
                        ar[0].as_ptr(),
                        ar[0].len(),
                        &mut cc,
                        b,
                    )
                })
                .0,
                ZmidiStatus::Ok
            );
            let ia = pending(|b, to, n| unsafe {
                zmidi_net_client_write_pending(&mut *client, 10, b.as_mut_ptr(), b.len(), to, n)
            });
            let (st, ev) = rx(|b| unsafe {
                zmidi_net_host_receive_auth(
                    &mut *host,
                    20,
                    &CLIENT,
                    ia[0].as_ptr(),
                    ia[0].len(),
                    &mut hc,
                    b,
                )
            });
            assert_eq!(st, ZmidiStatus::Ok);
            assert_eq!(
                ev.iter()
                    .map(|e| (e.kind, e.code, e.value))
                    .collect::<Vec<_>>(),
                [(ZmidiNetEventKind::AuthFailed, code, 1)]
            );
        }
    }

    /// REQ-FFI-ABI-001, REQ-FFI-NET-007: any payload to an authenticating Host, with any part
    /// of the random bytes used, gives a status, `random_used` inside the random bytes and no
    /// panic; a null pointer with a length gives invalid argument.
    #[test]
    fn prop_net_receive_auth_never_panics() {
        proptest!(|(body in proptest::collection::vec(any::<u8>(), 0..120), methods in 0u8..5, code in prop_oneof![Just(0x01u8), Just(0x02), Just(0x03), any::<u8>()], used in 0usize..40, null_secret in any::<bool>())| {
            let random = [0x41u8; 32];
            let table = [user(b"Rosa", b"RPBqBno")];
            let mut hc = creds(b"5483", &table, &random);
            hc.random_used = used;
            if null_secret {
                hc.shared_secret = core::ptr::null();
            }
            let mut host = Box::new(ZmidiNetHost { _opaque: [0; ZMIDI_NET_HOST_WORDS] });
            let st = unsafe { zmidi_net_host_init_auth(&mut *host, c"h".as_ptr().cast(), c"p".as_ptr().cast(), methods) };
            prop_assert_eq!(st == ZmidiStatus::Ok, (1..=3).contains(&methods));
            if st != ZmidiStatus::Ok {
                return Ok(());
            }
            let mut payload = b"MIDI".to_vec();
            payload.push(code);
            payload.extend_from_slice(&body);
            let (st, _) = rx(|b| unsafe { zmidi_net_host_receive_auth(&mut *host, 0, &CLIENT, payload.as_ptr(), payload.len(), &mut hc, b) });
            if null_secret {
                prop_assert_eq!(st, ZmidiStatus::InvalidArgument);
            } else {
                prop_assert!(st == ZmidiStatus::Ok || st == ZmidiStatus::BufferTooSmall);
                prop_assert!(hc.random_used == used || (hc.random_used == used + 16 && used + 16 <= random.len()));
            }
        });
    }
}

#[cfg(feature = "usb")]
mod usb {
    use proptest::prelude::*;
    use zmidi::status::ZmidiStatus;
    use zmidi::usb::*;

    /// REQ-FFI-ABI-001, REQ-FFI-USB-003, -004: any transfer, setting and UMPs give a status, counts
    /// inside the buffers and no panic.
    #[test]
    fn prop_usb_link_never_panics() {
        proptest!(|(data in proptest::collection::vec(any::<u8>(), 0..64), setting in 0u8..3, mask in any::<u16>(), umps in proptest::collection::vec((1u8..5, any::<[u32; 4]>()), 0..6), cap in 0usize..80)| {
            let mut link = ZmidiUsbLink { _opaque: [0; ZMIDI_USB_LINK_WORDS] };
            let c = ZmidiUsbLinkConfig { host_side: false, group_mask: 0xFFFF, max_packet: 64, protocol: 2 };
            prop_assert_eq!(unsafe { zmidi_usb_link_init(&mut link, &c) }, ZmidiStatus::Ok);
            prop_assert_eq!(unsafe { zmidi_usb_link_set_alternate_setting(&mut link, setting, mask, 64) }, ZmidiStatus::Ok);
            let mut recs = [ZmidiUsbUmp { word_count: 0, words: [0; 4] }; 17];
            let (mut n, mut e) = (0usize, 0usize);
            prop_assert_eq!(unsafe { zmidi_usb_link_read(&mut link, data.as_ptr(), data.len(), recs.as_mut_ptr(), recs.len(), &mut n, &mut e) }, ZmidiStatus::Ok);
            prop_assert!(n <= recs.len());
            let items: Vec<ZmidiUsbUmp> = umps.iter().map(|&(word_count, words)| ZmidiUsbUmp { word_count, words }).collect();
            let mut out = [0u8; 80];
            let (mut len, mut taken) = (0usize, 0usize);
            let st = unsafe { zmidi_usb_link_write_packet(&mut link, items.as_ptr(), items.len(), out.as_mut_ptr(), cap, &mut len, &mut taken) };
            if st == ZmidiStatus::Ok { prop_assert!(len <= cap && taken <= items.len()); }
        });
    }

    fn function() -> ZmidiUsbFunction {
        ZmidiUsbFunction {
            _opaque: [0; ZMIDI_USB_FUNCTION_WORDS],
        }
    }

    /// REQ-FFI-ABI-001, REQ-FFI-USB-006, -010: any configuration, endpoints and blocks give a
    /// status and no panic; a length that the writer accepts is the length that it writes, and a
    /// mask comes only from the blocks.
    #[test]
    fn prop_usb_function_never_panics() {
        proptest!(|(cfg in any::<[u8; 9]>(), high in any::<bool>(), packet in any::<u16>(), eps in proptest::collection::vec((0u8..3, 0u8..17, 0u8..5, any::<u8>(), any::<u16>(), 0u8..18, any::<[u8; 16]>()), 0..6), blocks in proptest::collection::vec(any::<[u8; 8]>(), 0..18), alt in 0u8..4, cap in 0usize..300)| {
            let mut f = function();
            let c = ZmidiUsbFunctionConfig { ac_interface: cfg[0] % 3, ms_interface: cfg[1] % 3, high_speed: high, ac_string: cfg[2], alt0_string: cfg[3], n_out: cfg[4] % 18, n_in: cfg[5] % 18, out_endpoint: cfg[6] % 17, in_endpoint: cfg[7] % 17, max_packet: if cfg[8] % 2 == 0 { 0 } else { packet } };
            prop_assert_eq!(unsafe { zmidi_usb_function_init(&mut f, &c) }, ZmidiStatus::Ok);
            let mut a = 0u8;
            for _ in 0..cfg[8] % 4 {
                let _ = unsafe { zmidi_usb_function_add_setting(&mut f, 0, &mut a) };
            }
            for &(direction, number, transfer, interval, max_packet, block_count, block_ids) in &eps {
                let e = ZmidiUsbEndpoint { direction, number, transfer, interval, max_packet, block_count, block_ids };
                let _ = unsafe { zmidi_usb_function_add_endpoint(&mut f, alt, &e) };
            }
            let bs: Vec<ZmidiUsbBlock> = blocks.iter().map(|b| ZmidiUsbBlock { id: b[0] % 20, kind: b[1] % 4, first_group: b[2] % 18, num_groups: b[3] % 18, string_index: b[4], protocol: b[5], max_input_bandwidth: u16::from(b[6]), max_output_bandwidth: u16::from(b[7]) }).collect();
            let set = unsafe { zmidi_usb_function_set_blocks(&mut f, alt, bs.as_ptr(), bs.len()) } == ZmidiStatus::Ok;
            let mut out = vec![0u8; cap];
            let (mut len, mut written) = (0usize, 0usize);
            let ok = unsafe { zmidi_usb_function_len(&mut f, &mut len) } == ZmidiStatus::Ok;
            let st = unsafe { zmidi_usb_function_write(&mut f, out.as_mut_ptr(), cap, &mut written) };
            if ok && cap >= len { prop_assert_eq!((st, written), (ZmidiStatus::Ok, len)); }
            if !ok { prop_assert_ne!(st, ZmidiStatus::Ok); }
            for i in 0..unsafe { zmidi_usb_function_endpoint_count(&mut f) } {
                let mut e = ZmidiUsbSettingEndpoint::default();
                prop_assert_eq!(unsafe { zmidi_usb_function_endpoint(&mut f, i, &mut e) }, ZmidiStatus::Ok);
            }
            for d in 0..3 {
                let mask = unsafe { zmidi_usb_function_group_mask(&mut f, alt, d) };
                if alt == 0 && d < 2 { prop_assert_eq!(mask, 0xFFFF); }
            }
            let _ = unsafe { zmidi_usb_gtb_set_write(&mut f, alt, out.as_mut_ptr(), cap, &mut written) };
            if unsafe { zmidi_usb_gtb_set_len(&mut f, alt, &mut len) } == ZmidiStatus::Ok {
                // A refused list leaves the default block (choice U-09).
                prop_assert_eq!(len, 5 + 13 * if set { bs.len() } else { 1 });
            }
        });
    }

    /// REQ-FFI-ABI-001, REQ-FFI-USB-008, -009: any setup packet and any answer bytes give a status,
    /// counts inside the buffers and no panic.
    /// REQ-FFI-USB-011: the fixed handler gives the same status, response and bytes as the
    /// run-time handler for the sets of the same function, for any setup packet and buffer.
    #[test]
    fn prop_usb_fixed_gtb_setup_equals_run_time() {
        proptest!(|(setup in any::<[u8; 8]>(), gtb in any::<bool>(), ms in 0u8..3, cap in 0usize..64)| {
            // The default function on the MIDI Streaming interface `ms`.
            let mut f = function();
            prop_assert_eq!(unsafe { zmidi_usb_function_init_default(&mut f, if ms == 0 { 1 } else { 0 }, ms, false) }, ZmidiStatus::Ok);
            let mut set = [0u8; 64];
            let mut len = 0usize;
            prop_assert_eq!(unsafe { zmidi_usb_gtb_set_write(&mut f, 1, set.as_mut_ptr(), set.len(), &mut len) }, ZmidiStatus::Ok);
            let mut setup = setup;
            if gtb { let alt = setup[2] % 3; setup[..4].copy_from_slice(&[0x81, 0x06, alt, 0x26]); setup[5] = 0; setup[4] %= 3; }
            let (mut a, mut b) = (vec![0xEEu8; cap], vec![0xEEu8; cap]);
            let (mut ra, mut rb) = (ZmidiUsbGtbResponse::NotHandled, ZmidiUsbGtbResponse::NotHandled);
            let (mut na, mut nb) = (99usize, 99usize);
            let sa = unsafe { zmidi_usb_gtb_setup(&mut f, setup.as_ptr(), a.as_mut_ptr(), cap, &mut ra, &mut na) };
            let sets = [set.as_ptr()];
            let lens = [len];
            let sb = unsafe { zmidi_usb_fixed_gtb_setup(sets.as_ptr(), lens.as_ptr(), 1, ms, setup.as_ptr(), b.as_mut_ptr(), cap, &mut rb, &mut nb) };
            prop_assert_eq!(sa, sb);
            prop_assert_eq!((ra, na), (rb, nb));
            prop_assert_eq!(a, b);
        });
    }

    #[test]
    fn prop_usb_gtb_requests_never_panic() {
        proptest!(|(setup in any::<[u8; 8]>(), gtb in any::<bool>(), answer in proptest::collection::vec(any::<u8>(), 0..80), index in 0usize..8, cap in 0usize..64)| {
            let mut f = function();
            prop_assert_eq!(unsafe { zmidi_usb_function_init_default(&mut f, 0, 1, false) }, ZmidiStatus::Ok);
            let mut setup = setup;
            if gtb { let alt = setup[2] % 3; setup[..4].copy_from_slice(&[0x81, 0x06, alt, 0x26]); setup[5] = 0; setup[4] %= 3; }
            let mut out = vec![0u8; cap];
            let mut r = ZmidiUsbGtbResponse::NotHandled;
            let mut n = 99usize;
            let st = unsafe { zmidi_usb_gtb_setup(&mut f, setup.as_ptr(), out.as_mut_ptr(), cap, &mut r, &mut n) };
            if st == ZmidiStatus::Ok {
                prop_assert!(n <= cap.min(18) && n <= usize::from(u16::from_le_bytes([setup[6], setup[7]])));
                if r != ZmidiUsbGtbResponse::Answered { prop_assert_eq!(n, 0); }
            }
            let mut total = 0u16;
            let _ = unsafe { zmidi_usb_gtb_total_length(answer.as_ptr(), answer.len(), &mut total) };
            let mut b = ZmidiUsbBlock::default();
            let _ = unsafe { zmidi_usb_gtb_block(answer.as_ptr(), answer.len(), index, &mut b) };
            let mut req = [0u8; 8];
            prop_assert_eq!(unsafe { zmidi_usb_gtb_request(setup[2], setup[4], total, req.as_mut_ptr()) }, ZmidiStatus::Ok);
        });
    }
}

#[cfg(feature = "midi1")]
mod midi1 {
    use proptest::prelude::*;
    use zmidi::midi1::*;
    use zmidi::status::ZmidiStatus;
    use zmidi_io::{
        ControllerState, Midi1Event, Midi1Message, Midi1Reader, Midi1Writer, ModeConfig, ModeState,
        ParameterState,
    };
    use zmidi_ump::{Channel, Midi1ChannelVoiceMessage};

    /// The C message of a MIDI 1.0 word (Message Type 0x2 or 0x1): bytes 2 to 4.
    fn of_word(kind: ZmidiMidi1Kind, w: u32) -> ZmidiMidi1Message {
        ZmidiMidi1Message {
            kind: kind as u32,
            status: (w >> 16) as u8,
            data1: (w >> 8) as u8 & 0x7F,
            data2: w as u8 & 0x7F,
            length: 0,
        }
    }

    fn cv(m: Midi1ChannelVoiceMessage) -> ZmidiMidi1Message {
        of_word(ZmidiMidi1Kind::ChannelVoice, m.into_bits())
    }

    /// Any Channel Voice message (status 80H to EFH, 7-bit data).
    fn any_cv() -> impl Strategy<Value = Midi1ChannelVoiceMessage> {
        (0x80u32..=0xEF, 0u32..0x80, 0u32..0x80).prop_map(|(s, a, b)| {
            Midi1ChannelVoiceMessage::parse(&[0x2000_0000 | s << 16 | a << 8 | b]).unwrap()
        })
    }

    /// Control Changes of the controller and parameter controllers, Program Change and Note On.
    fn any_control() -> impl Strategy<Value = Midi1ChannelVoiceMessage> {
        prop_oneof![
            (
                prop_oneof![
                    0u32..=63,
                    64u32..=127,
                    Just(6),
                    Just(38),
                    96u32..=101,
                    Just(121)
                ],
                0u32..0x80
            )
                .prop_map(
                    |(c, v)| Midi1ChannelVoiceMessage::parse(&[0x20B0_0000 | c << 8 | v]).unwrap()
                ),
            any_cv(),
        ]
    }

    /// REQ-FFI-MIDI1-001: a Channel Voice message goes to C and back; a bad status or data byte
    /// gives invalid-argument.
    #[test]
    fn prop_midi1_message_round_trip() {
        proptest!(|(m in any_cv(), bad_status in 0u8..0x80, bad_data in 0x80u8..=0xFF)| {
            let mut s = ZmidiControllerState { _opaque: [0; ZMIDI_CONTROLLER_STATE_WORDS] };
            prop_assert_eq!(unsafe { zmidi_controllers_init(&mut s) }, ZmidiStatus::Ok);
            let mut e = ZmidiControllerEvent { kind: 9, controller: 0, program: 0, source: 0, has_bank: false, value: 0, bank: 0 };
            let c = cv(m);
            prop_assert_eq!(unsafe { zmidi_controllers_receive(&mut s, &c, &mut e) }, ZmidiStatus::Ok);
            for bad in [
                ZmidiMidi1Message { status: bad_status, ..c },
                ZmidiMidi1Message { data1: bad_data, ..c },
                ZmidiMidi1Message { kind: ZmidiMidi1Kind::System as u32, ..c },
            ] {
                prop_assert_eq!(unsafe { zmidi_controllers_receive(&mut s, &bad, &mut e) }, ZmidiStatus::InvalidArgument);
            }
            // The writer gives the bytes of the message.
            let mut w = ZmidiMidi1Writer { _opaque: [0; ZMIDI_MIDI1_WRITER_WORDS] };
            prop_assert_eq!(unsafe { zmidi_midi1_writer_init(&mut w, false) }, ZmidiStatus::Ok);
            let (mut out, mut n) = ([0u8; 4], 0usize);
            prop_assert_eq!(unsafe { zmidi_midi1_writer_write(&mut w, &c, core::ptr::null(), out.as_mut_ptr(), 4, &mut n) }, ZmidiStatus::Ok);
            let len = if (0xC0..=0xDF).contains(&c.status) { 2 } else { 3 };
            prop_assert_eq!(&out[..n], &[c.status, c.data1, c.data2][..len]);
        });
    }

    /// REQ-FFI-MIDI1-002: the C reader gives the messages of the Rust reader for any bytes.
    #[test]
    fn prop_midi1_reader_equals_rust() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..96), sysex_cap in 0usize..12, cap in 0usize..4, reset_at in 0usize..96)| {
            let mut c = ZmidiMidi1Reader { _opaque: [0; ZMIDI_MIDI1_READER_WORDS] };
            prop_assert_eq!(unsafe { zmidi_midi1_reader_init(&mut c) }, ZmidiStatus::Ok);
            let mut r = Midi1Reader::new();
            let (mut cbuf, mut rbuf) = ([0u8; 12], [0u8; 12]);
            for (i, b) in bytes.into_iter().enumerate() {
                if i == reset_at {
                    prop_assert_eq!(unsafe { zmidi_midi1_reader_reset(&mut c) }, ZmidiStatus::Ok);
                    r.reset();
                }
                let mut want = Vec::new();
                r.feed(b, &mut rbuf[..sysex_cap], |e| want.push(match e {
                    Midi1Event::Message(Midi1Message::ChannelVoice(m)) => (cv(m), Vec::new()),
                    Midi1Event::Message(Midi1Message::System(m)) => (of_word(ZmidiMidi1Kind::System, m.into_bits()), Vec::new()),
                    Midi1Event::Message(Midi1Message::SysEx(s)) => (ZmidiMidi1Message { kind: ZmidiMidi1Kind::SysEx as u32, status: 0xF0, data1: 0, data2: 0, length: s.len() as u32 }, s.to_vec()),
                    Midi1Event::SysExTooLong { length } => (ZmidiMidi1Message { kind: ZmidiMidi1Kind::SysExTooLong as u32, status: 0xF0, data1: 0, data2: 0, length: length as u32 }, Vec::new()),
                }));
                let mut out = [ZmidiMidi1Message { kind: 0, status: 0, data1: 0, data2: 0, length: 0 }; 4];
                let mut n = 0usize;
                let st = unsafe { zmidi_midi1_reader_push(&mut c, b, cbuf.as_mut_ptr(), sysex_cap, out.as_mut_ptr(), cap, &mut n) };
                if cap < ZMIDI_MIDI1_READER_MAX_EVENTS {
                    // No byte read: give it to the C reader again with room.
                    prop_assert_eq!(st, ZmidiStatus::BufferTooSmall);
                    let st = unsafe { zmidi_midi1_reader_push(&mut c, b, cbuf.as_mut_ptr(), sysex_cap, out.as_mut_ptr(), 4, &mut n) };
                    prop_assert_eq!(st, ZmidiStatus::Ok);
                }
                prop_assert_eq!(n, want.len());
                for (got, (m, sysex)) in out[..n].iter().zip(&want) {
                    prop_assert_eq!(got, m);
                    if got.kind == ZmidiMidi1Kind::SysEx as u32 {
                        prop_assert_eq!(&cbuf[..got.length as usize], &sysex[..]);
                    }
                }
            }
        });
    }

    /// REQ-FFI-MIDI1-003: the C writer gives the bytes of the Rust writer, and a small buffer or a
    /// bad SysEx changes no state.
    #[test]
    fn prop_midi1_writer_equals_rust() {
        proptest!(|(ms in proptest::collection::vec(any_cv(), 1..12), rs in any::<bool>(), small in 0usize..12, sysex in proptest::collection::vec(any::<u8>(), 0..6))| {
            let mut c = ZmidiMidi1Writer { _opaque: [0; ZMIDI_MIDI1_WRITER_WORDS] };
            prop_assert_eq!(unsafe { zmidi_midi1_writer_init(&mut c, rs) }, ZmidiStatus::Ok);
            let mut r = Midi1Writer::new(rs);
            for (i, m) in ms.into_iter().enumerate() {
                let (mut want, mut got) = ([0u8; 4], [0u8; 4]);
                let k = r.write(Midi1Message::ChannelVoice(m), &mut want).unwrap();
                let msg = cv(m);
                let mut n = 0usize;
                if i == small {
                    prop_assert_eq!(unsafe { zmidi_midi1_writer_write(&mut c, &msg, core::ptr::null(), got.as_mut_ptr(), 0, &mut n) }, ZmidiStatus::BufferTooSmall);
                }
                prop_assert_eq!(unsafe { zmidi_midi1_writer_write(&mut c, &msg, core::ptr::null(), got.as_mut_ptr(), 4, &mut n) }, ZmidiStatus::Ok);
                prop_assert_eq!(&got[..n], &want[..k]);
            }
            // A SysEx: F0H, data, F7H is written; anything else is invalid.
            let mut bytes = vec![0xF0u8];
            bytes.extend(&sysex);
            bytes.push(0xF7);
            let ok = sysex.iter().all(|b| *b < 0x80);
            let msg = ZmidiMidi1Message { kind: ZmidiMidi1Kind::SysEx as u32, status: 0xF0, data1: 0, data2: 0, length: bytes.len() as u32 };
            let (mut out, mut n) = ([0u8; 16], 0usize);
            let st = unsafe { zmidi_midi1_writer_write(&mut c, &msg, bytes.as_ptr(), out.as_mut_ptr(), 16, &mut n) };
            prop_assert_eq!(st, if ok { ZmidiStatus::Ok } else { ZmidiStatus::InvalidArgument });
            if ok { prop_assert_eq!(&out[..n], &bytes[..]); }
            prop_assert_eq!(unsafe { zmidi_midi1_writer_write(&mut c, &msg, core::ptr::null(), out.as_mut_ptr(), 16, &mut n) }, ZmidiStatus::InvalidArgument);
        });
    }

    /// REQ-FFI-MIDI1-004, -009: the C mode state gives the actions and information of the Rust
    /// state for any configuration and messages.
    #[test]
    fn prop_mode_state_equals_rust() {
        proptest!(|(basic in 0u8..20, mask in 0u8..0x14, alternate in any::<bool>(), voices in 0u8..20, global in any::<bool>(), ms in proptest::collection::vec(any_cv(), 0..24), reset_at in 0usize..24)| {
            let config = ZmidiModeConfig { basic_channel: basic, implemented: mask, alternate, voices, global_controllers: global };
            let mut c = ZmidiModeState { _opaque: [0; ZMIDI_MODE_STATE_WORDS] };
            let st = unsafe { zmidi_mode_init(&mut c, &config) };
            prop_assert_eq!(st == ZmidiStatus::Ok, basic < 16 && mask < 0x10);
            if st != ZmidiStatus::Ok { return Ok(()); }
            let mut r = ModeState::new(ModeConfig {
                basic_channel: Channel::new(basic).unwrap(),
                implemented: [0, 1, 2, 3].map(|b| mask >> b & 1 != 0),
                alternate,
                voices,
                global_controllers: global,
            });
            for (i, m) in ms.into_iter().enumerate() {
                if i == reset_at {
                    prop_assert_eq!(unsafe { zmidi_mode_system_reset(&mut c) }, ZmidiStatus::Ok);
                    r.system_reset();
                }
                let mut a = ZmidiModeAction { kind: 99, channel: 0, voice: 0, on: false, changed: false, alternate: false };
                prop_assert_eq!(unsafe { zmidi_mode_receive(&mut c, &cv(m), &mut a) }, ZmidiStatus::Ok);
                let want = r.receive(&m);
                prop_assert_eq!(format!("{:?}", kind_of(&a)), format!("{want:?}"));
                let mut info = ZmidiModeInfo { mode: 0, requested_mode: 0, alternate: false, basic_channel: 0, m: 0, local: false };
                prop_assert_eq!(unsafe { zmidi_mode_info(&c, &mut info) }, ZmidiStatus::Ok);
                prop_assert_eq!(
                    (info.mode, info.requested_mode, info.alternate, info.basic_channel, info.m, info.local),
                    (r.mode().number(), r.requested_mode().number(), r.is_alternate(), r.basic_channel().index(), r.m(), r.local())
                );
            }
            // The setters.
            prop_assert_eq!(unsafe { zmidi_mode_set_basic_channel(&mut c, 16) }, ZmidiStatus::InvalidArgument);
            prop_assert_eq!(unsafe { zmidi_mode_set_mono(&mut c, 0x80) }, ZmidiStatus::InvalidArgument);
            prop_assert_eq!(unsafe { zmidi_mode_set_basic_channel(&mut c, 3) }, ZmidiStatus::Ok);
            prop_assert_eq!(unsafe { zmidi_mode_set_omni(&mut c, false) }, ZmidiStatus::Ok);
            prop_assert_eq!(unsafe { zmidi_mode_set_mono(&mut c, 2) }, ZmidiStatus::Ok);
            prop_assert_eq!(unsafe { zmidi_mode_set_local(&mut c, false) }, ZmidiStatus::Ok);
            r.set_basic_channel(Channel::new(3).unwrap());
            r.set_omni(false);
            r.set_mono(2);
            r.set_local(false);
            let mut info = ZmidiModeInfo { mode: 0, requested_mode: 0, alternate: false, basic_channel: 0, m: 0, local: true };
            prop_assert_eq!(unsafe { zmidi_mode_info(&c, &mut info) }, ZmidiStatus::Ok);
            prop_assert_eq!((info.mode, info.basic_channel, info.m, info.local), (r.mode().number(), 3, 2, false));
            prop_assert_eq!(unsafe { zmidi_mode_set_poly(&mut c) }, ZmidiStatus::Ok);
            r.set_poly();
            prop_assert_eq!(unsafe { zmidi_mode_info(&c, &mut info) }, ZmidiStatus::Ok);
            prop_assert_eq!(info.requested_mode, r.requested_mode().number());
        });
    }

    /// The Rust `ModeAction` of a C action, to compare with `Debug`.
    fn kind_of(a: &ZmidiModeAction) -> zmidi_io::ModeAction {
        use zmidi_io::ModeAction as A;
        let ch = |c: u8| (c != 0xFF).then(|| Channel::new(c).unwrap());
        match a.kind {
            0 => A::Ignore,
            1 => A::Voice {
                voice: (a.voice != 0xFF).then_some(a.voice),
            },
            2 => A::Global,
            3 => A::AllNotesOff {
                channel: ch(a.channel),
            },
            4 => A::AllSoundOff {
                channel: ch(a.channel),
            },
            5 => A::ResetAllControllers {
                channel: ch(a.channel),
            },
            6 => A::LocalControl { on: a.on },
            _ => A::ModeChange {
                changed: a.changed,
                alternate: a.alternate,
            },
        }
    }

    /// REQ-FFI-MIDI1-005: the C controller state gives the events and bank of the Rust state.
    #[test]
    fn prop_controller_state_equals_rust() {
        proptest!(|(ms in proptest::collection::vec(any_control(), 0..32), reset_at in 0usize..32)| {
            let mut c = ZmidiControllerState { _opaque: [0; ZMIDI_CONTROLLER_STATE_WORDS] };
            prop_assert_eq!(unsafe { zmidi_controllers_init(&mut c) }, ZmidiStatus::Ok);
            let mut r = ControllerState::new();
            for (i, m) in ms.into_iter().enumerate() {
                if i == reset_at {
                    prop_assert_eq!(unsafe { zmidi_controllers_reset(&mut c) }, ZmidiStatus::Ok);
                    r.reset_all_controllers();
                }
                let mut e = ZmidiControllerEvent { kind: 99, controller: 0, program: 0, source: 0, has_bank: false, value: 0, bank: 0 };
                prop_assert_eq!(unsafe { zmidi_controllers_receive(&mut c, &cv(m), &mut e) }, ZmidiStatus::Ok);
                let want = r.receive(&m);
                use zmidi_io::ControllerEvent as E;
                let got = match e.kind {
                    0 => None,
                    1 => Some(E::Controller14 { controller: e.controller, value: e.value }),
                    2 => Some(E::Controller7 { controller: e.controller, value: e.value as u8 }),
                    3 => Some(E::Program { program: e.program, bank: e.has_bank.then_some(e.bank) }),
                    4 => Some(E::Portamento { source: e.source }),
                    _ => Some(E::ResetAllControllers),
                };
                prop_assert_eq!(got, want);
                let (mut has, mut bank) = (false, 0u16);
                prop_assert_eq!(unsafe { zmidi_controllers_bank(&c, &mut has, &mut bank) }, ZmidiStatus::Ok);
                prop_assert_eq!(has.then_some(bank), r.bank());
            }
        });
    }

    /// REQ-FFI-MIDI1-010: the reset value and the controller writers give the zmidi-io results.
    #[test]
    fn prop_controller_writers_equal_rust() {
        proptest!(|(ch in 0u8..18, controller in any::<u8>(), value in 0u16..0x4100, program in 0u8..0x90, cap in 0usize..5)| {
            let mut v = 0u8;
            prop_assert_eq!(unsafe { zmidi_controller_reset_value(controller, &mut v) }, ZmidiStatus::Ok);
            prop_assert_eq!(v, zmidi_io::reset_value(controller).unwrap_or(ZMIDI_CONTROLLER_KEEP));
            let mut out = [ZmidiMidi1Message { kind: 0, status: 0, data1: 0, data2: 0, length: 0 }; 4];
            let mut n = 0usize;
            let want = Channel::new(ch).and_then(|c| zmidi_io::controller14_messages(c, controller, value));
            let st = unsafe { zmidi_controller14_messages(ch, controller, value, out.as_mut_ptr(), cap, &mut n) };
            match want {
                Err(_) => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
                Ok(_) if cap < 2 => prop_assert_eq!(st, ZmidiStatus::BufferTooSmall),
                Ok(w) => { prop_assert_eq!(st, ZmidiStatus::Ok); prop_assert_eq!(&out[..n], &w.map(cv)[..]); }
            }
            let want = Channel::new(ch).and_then(|c| zmidi_io::bank_program_messages(c, value, program));
            let st = unsafe { zmidi_bank_program_messages(ch, value, program, out.as_mut_ptr(), cap, &mut n) };
            match want {
                Err(_) => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
                Ok(_) if cap < 3 => prop_assert_eq!(st, ZmidiStatus::BufferTooSmall),
                Ok(w) => { prop_assert_eq!(st, ZmidiStatus::Ok); prop_assert_eq!(&out[..n], &w.map(cv)[..]); }
            }
        });
    }

    /// REQ-FFI-MIDI1-006: the C parameter receiver gives the events, selection and reception of
    /// the Rust receiver.
    #[test]
    fn prop_parameter_state_equals_rust() {
        proptest!(|(ms in proptest::collection::vec(any_control(), 0..32), switch_at in 0usize..32, rpn in any::<bool>(), nrpn in any::<bool>(), reset_at in 0usize..32)| {
            let mut c = ZmidiParameterState { _opaque: [0; ZMIDI_PARAMETER_STATE_WORDS] };
            prop_assert_eq!(unsafe { zmidi_parameters_init(&mut c) }, ZmidiStatus::Ok);
            let mut r = ParameterState::new();
            for (i, m) in ms.into_iter().enumerate() {
                if i == switch_at {
                    prop_assert_eq!(unsafe { zmidi_parameters_set_reception(&mut c, rpn, nrpn) }, ZmidiStatus::Ok);
                    r.set_rpn_reception(rpn);
                    r.set_nrpn_reception(nrpn);
                }
                if i == reset_at {
                    prop_assert_eq!(unsafe { zmidi_parameters_reset(&mut c) }, ZmidiStatus::Ok);
                    r.reset_all_controllers();
                }
                let mut e = ZmidiParameterEvent { change: 99, parameter: ZmidiParameter { registered: false, msb: 0, lsb: 0 }, value: 0 };
                prop_assert_eq!(unsafe { zmidi_parameters_receive(&mut c, &cv(m), &mut e) }, ZmidiStatus::Ok);
                let want = r.receive(&m);
                prop_assert_eq!(e.change == 0, want.is_none());
                if let Some(w) = want {
                    // The event gives the same new value as the Rust event.
                    let mut out = 0u16;
                    prop_assert_eq!(unsafe { zmidi_parameter_apply(&e, 0x1234, &mut out) }, ZmidiStatus::Ok);
                    prop_assert_eq!(out, w.apply(0x1234));
                }
                let (mut sel, mut p) = (false, ZmidiParameter { registered: false, msb: 0, lsb: 0 });
                prop_assert_eq!(unsafe { zmidi_parameters_selection(&c, &mut sel, &mut p) }, ZmidiStatus::Ok);
                prop_assert_eq!(sel, r.selection().is_some());
                let (mut a, mut b) = (false, false);
                prop_assert_eq!(unsafe { zmidi_parameters_reception(&c, &mut a, &mut b) }, ZmidiStatus::Ok);
                prop_assert_eq!((a, b), (r.rpn_reception(), r.nrpn_reception()));
            }
        });
    }

    /// REQ-FFI-MIDI1-011: the apply function and the parameter writers give the zmidi-io results.
    #[test]
    fn prop_parameter_writers_equal_rust() {
        proptest!(|(ch in 0u8..18, registered in any::<bool>(), msb in 0u8..0x90, lsb in 0u8..0x90, value in 0u16..0x4100, change in 0u32..6, data in 0u8..0x90, cap in 0usize..5)| {
            let p = ZmidiParameter { registered, msb, lsb };
            let rp = (msb < 0x80 && lsb < 0x80).then(|| if registered {
                zmidi_io::Parameter::Registered(zmidi_ump::controller::Rpn::from_parts(msb, lsb))
            } else {
                zmidi_io::Parameter::NonRegistered { msb, lsb }
            });
            let rc = if data >= 0x80 { None } else { match change {
                1 => Some(zmidi_io::ParameterChange::DataEntryMsb(data)),
                2 => Some(zmidi_io::ParameterChange::DataEntryLsb(data)),
                3 => Some(zmidi_io::ParameterChange::Increment),
                4 => Some(zmidi_io::ParameterChange::Decrement),
                _ => None,
            } };
            let mut out16 = 0u16;
            let st = unsafe { zmidi_parameter_apply(&ZmidiParameterEvent { change, parameter: p, value: data }, value, &mut out16) };
            match (rp, rc, value < 0x4000) {
                (Some(parameter), Some(change), true) => {
                    prop_assert_eq!(st, ZmidiStatus::Ok);
                    prop_assert_eq!(out16, zmidi_io::ParameterEvent { parameter, change }.apply(value));
                }
                _ => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
            }
            let mut out = [ZmidiMidi1Message { kind: 0, status: 0, data1: 0, data2: 0, length: 0 }; 4];
            let mut n = 0usize;
            let c = Channel::new(ch).ok();
            let st = unsafe { zmidi_parameter_select_messages(ch, &p, out.as_mut_ptr(), cap, &mut n) };
            match (c, rp) {
                (Some(c), Some(rp)) if cap >= 2 => { prop_assert_eq!(st, ZmidiStatus::Ok); prop_assert_eq!(&out[..n], &zmidi_io::select_messages(c, rp).unwrap().map(cv)[..]); }
                (Some(_), Some(_)) => prop_assert_eq!(st, ZmidiStatus::BufferTooSmall),
                _ => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
            }
            let st = unsafe { zmidi_parameter_value_messages(ch, &p, value, out.as_mut_ptr(), cap, &mut n) };
            match (c, rp, value < 0x4000) {
                (Some(c), Some(rp), true) if cap >= 4 => { prop_assert_eq!(st, ZmidiStatus::Ok); prop_assert_eq!(&out[..n], &zmidi_io::value_messages(c, rp, value).unwrap().map(cv)[..]); }
                (Some(_), Some(_), true) => prop_assert_eq!(st, ZmidiStatus::BufferTooSmall),
                _ => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
            }
            let mut one = out[0];
            let st = unsafe { zmidi_parameter_change_message(ch, change, data, &mut one) };
            match (c, rc) {
                (Some(c), Some(rc)) => { prop_assert_eq!(st, ZmidiStatus::Ok); prop_assert_eq!(one, cv(zmidi_io::change_message(c, rc).unwrap())); }
                _ => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
            }
        });
    }

    /// REQ-FFI-MIDI1-012: the C sync receiver gives the events and information of the Rust
    /// receiver for any System messages and commands.
    #[test]
    fn prop_sync_receiver_equals_rust() {
        use zmidi_io::{SyncCommand, SyncEvent, SyncReceiver, SyncRun};
        proptest!(|(steps in proptest::collection::vec((0u8..9, 0u8..0x80, 0u8..0x80), 0..40), off_at in 0usize..40)| {
            let mut c = ZmidiSyncReceiver { _opaque: [0; ZMIDI_SYNC_RECEIVER_WORDS] };
            prop_assert_eq!(unsafe { zmidi_sync_init(&mut c) }, ZmidiStatus::Ok);
            let mut r = SyncReceiver::new();
            for (i, (op, a, b)) in steps.into_iter().enumerate() {
                if i == off_at {
                    prop_assert_eq!(unsafe { zmidi_sync_set_mode(&mut c, false) }, ZmidiStatus::Ok);
                    r.set_sync_mode(false);
                }
                let mut e = ZmidiSyncEvent { kind: 99, position: 0, song: 0 };
                let want = if op < 3 {
                    let cmd = [SyncCommand::Start, SyncCommand::Continue, SyncCommand::Stop][op as usize];
                    prop_assert_eq!(unsafe { zmidi_sync_command(&mut c, u32::from(op) + 1, &mut e) }, ZmidiStatus::Ok);
                    r.command(cmd)
                } else {
                    let status: u8 = [0xF8, 0xFA, 0xFB, 0xFC, 0xF2, 0xF3][op as usize - 3];
                    let m = of_word(ZmidiMidi1Kind::System, 0x1000_0000 | u32::from(status) << 16 | u32::from(a) << 8 | u32::from(b));
                    prop_assert_eq!(unsafe { zmidi_sync_receive(&mut c, &m, &mut e) }, ZmidiStatus::Ok);
                    r.receive(&zmidi_ump::SystemMessage::parse(&[0x1000_0000 | u32::from(status) << 16 | u32::from(a) << 8 | u32::from(b)]).unwrap())
                };
                let got = match e.kind {
                    0 => None,
                    1 => Some(SyncEvent::Start),
                    2 => Some(SyncEvent::Continue),
                    3 => Some(SyncEvent::Stop),
                    4 => Some(SyncEvent::Clock { position: e.position }),
                    5 => Some(SyncEvent::Locate { position: e.position }),
                    _ => Some(SyncEvent::Song { number: e.song }),
                };
                prop_assert_eq!(got, want);
                let mut info = ZmidiSyncInfo { position: 9, run: 9, song: 9, sync_mode: true };
                prop_assert_eq!(unsafe { zmidi_sync_info(&c, &mut info) }, ZmidiStatus::Ok);
                let run = match r.run() { SyncRun::Stopped => 0, SyncRun::Waiting => 1, SyncRun::Playing => 2 };
                prop_assert_eq!((info.position, info.run, info.song, info.sync_mode), (r.position(), run, r.song(), r.sync_mode()));
            }
            prop_assert_eq!(unsafe { zmidi_sync_system_reset(&mut c) }, ZmidiStatus::Ok);
            let mut e = ZmidiSyncEvent { kind: 0, position: 0, song: 0 };
            prop_assert_eq!(unsafe { zmidi_sync_command(&mut c, 0, &mut e) }, ZmidiStatus::InvalidArgument);
            let bad = cv(Midi1ChannelVoiceMessage::parse(&[0x2090_3C40]).unwrap());
            prop_assert_eq!(unsafe { zmidi_sync_receive(&mut c, &bad, &mut e) }, ZmidiStatus::InvalidArgument);
        });
    }

    /// REQ-FFI-MIDI1-013: the C Active Sensing monitor and sender give the results of the Rust
    /// types for any bytes and times.
    #[test]
    fn prop_active_sensing_equals_rust() {
        use zmidi_io::{ActiveSensingMonitor, ActiveSensingSender};
        proptest!(|(timeout in 1u32..500_000, interval in 1u32..500_000, steps in proptest::collection::vec((0u64..400_000, any::<u8>(), 0u8..4), 1..40))| {
            let mut cm = ZmidiActiveSensingMonitor { _opaque: [0; ZMIDI_ACTIVE_SENSING_MONITOR_WORDS] };
            let mut cs = ZmidiActiveSensingSender { _opaque: [0; ZMIDI_ACTIVE_SENSING_SENDER_WORDS] };
            prop_assert_eq!(unsafe { zmidi_active_sensing_monitor_init(&mut cm, timeout) }, ZmidiStatus::Ok);
            prop_assert_eq!(unsafe { zmidi_active_sensing_sender_init(&mut cs, interval) }, ZmidiStatus::Ok);
            let (mut rm, mut rs) = (ActiveSensingMonitor::new(timeout), ActiveSensingSender::new(interval));
            let mut now = 0u64;
            for (dt, byte, op) in steps {
                now += dt;
                match op {
                    0 => {
                        prop_assert_eq!(unsafe { zmidi_active_sensing_monitor_receive_byte(&mut cm, byte, now) }, ZmidiStatus::Ok);
                        rm.receive_byte(byte, now);
                    }
                    1 => {
                        prop_assert_eq!(unsafe { zmidi_active_sensing_monitor_receive(&mut cm, byte & 1 == 1, now) }, ZmidiStatus::Ok);
                        rm.receive(byte & 1 == 1, now);
                    }
                    2 => {
                        let (mut timeout, mut armed) = (false, false);
                        prop_assert_eq!(unsafe { zmidi_active_sensing_monitor_poll(&mut cm, now, &mut timeout, &mut armed) }, ZmidiStatus::Ok);
                        prop_assert_eq!(timeout, rm.poll(now));
                        prop_assert_eq!(armed, rm.is_armed());
                        let mut still = false;
                        prop_assert_eq!(unsafe { zmidi_active_sensing_monitor_is_armed(&cm, &mut still) }, ZmidiStatus::Ok);
                        prop_assert_eq!(still, rm.is_armed());
                    }
                    _ => {
                        let mut send = false;
                        if byte & 1 == 1 {
                            prop_assert_eq!(unsafe { zmidi_active_sensing_sender_sent(&mut cs, now) }, ZmidiStatus::Ok);
                            rs.sent(now);
                        }
                        prop_assert_eq!(unsafe { zmidi_active_sensing_sender_poll(&mut cs, now, &mut send) }, ZmidiStatus::Ok);
                        prop_assert_eq!(send, rs.poll(now));
                    }
                }
            }
        });
    }

    /// REQ-FFI-MIDI1-007: each storage holds its Rust type.
    #[test]
    fn midi1_storage_holds_the_rust_types() {
        use core::mem::size_of;
        assert!(zmidi_midi1_reader_size() <= size_of::<ZmidiMidi1Reader>());
        assert!(zmidi_midi1_writer_size() <= size_of::<ZmidiMidi1Writer>());
        assert!(zmidi_mode_state_size() <= size_of::<ZmidiModeState>());
        assert!(zmidi_controller_state_size() <= size_of::<ZmidiControllerState>());
        assert!(zmidi_parameter_state_size() <= size_of::<ZmidiParameterState>());
        assert!(zmidi_sync_receiver_size() <= size_of::<ZmidiSyncReceiver>());
        assert!(zmidi_active_sensing_monitor_size() <= size_of::<ZmidiActiveSensingMonitor>());
        assert!(zmidi_active_sensing_sender_size() <= size_of::<ZmidiActiveSensingSender>());
        assert!(zmidi_mpe_receiver_size() <= size_of::<ZmidiMpeReceiver>());
        assert!(zmidi_mpe_note_allocator_size() <= size_of::<ZmidiMpeNoteAllocator>());
    }

    /// The C event of a Rust MPE event (REQ-FFI-MIDI1-014).
    fn c_mpe_event(e: zmidi_io::MpeEvent) -> (u32, u32, bool, u16) {
        use zmidi_io::MpeEvent as E;
        match e {
            E::NoZone => (0, 0, false, 0),
            E::Zone { zone, manager } => (1, zone as u32, manager, 0),
            E::Layout { reset } => (2, 0, false, reset),
            E::Sensitivity { zone, manager } => (3, zone as u32, manager, 0),
            E::Mode { zone } => (4, zone as u32, false, 0),
            E::Ignored => (5, 0, false, 0),
        }
    }

    /// REQ-FFI-MIDI1-014: the C MPE receiver, fed with the events of the C parameter receivers,
    /// gives the events, information and channel values of the Rust receiver for any Channel Voice
    /// messages and any application layout.
    #[test]
    fn prop_mpe_receiver_equals_rust() {
        use zmidi_io::{MpeLayout, MpeReceiver, MpeZone};
        // MCMs and RPN 0 on Channels 1, 2 and 16 often, so that the layout changes.
        let step = prop_oneof![
            any_cv(),
            (
                prop_oneof![Just(0u8), Just(1), Just(15)],
                prop_oneof![Just(6u8), Just(38), Just(96), Just(100), Just(101)],
                0u8..20
            )
                .prop_map(|(c, i, v)| Midi1ChannelVoiceMessage::parse(&[0x20B0_0000
                    | u32::from(c) << 16
                    | u32::from(i) << 8
                    | u32::from(v)])
                .unwrap()),
        ];
        proptest!(|(steps in proptest::collection::vec(step, 0..60), at in 0usize..60, lower in 0u8..16, upper in 0u8..16)| {
            let mut c = ZmidiMpeReceiver { _opaque: [0; ZMIDI_MPE_RECEIVER_WORDS] };
            let mut cp: [ZmidiParameterState; 16] = core::array::from_fn(|_| ZmidiParameterState { _opaque: [0; ZMIDI_PARAMETER_STATE_WORDS] });
            prop_assert_eq!(unsafe { zmidi_mpe_receiver_init(&mut c) }, ZmidiStatus::Ok);
            for p in &mut cp {
                prop_assert_eq!(unsafe { zmidi_parameters_init(p) }, ZmidiStatus::Ok);
            }
            let mut r = MpeReceiver::new();
            let mut rp = [ParameterState::new(); 16];
            for (i, m) in steps.into_iter().enumerate() {
                if i == at {
                    let mut reset = 0;
                    let status = unsafe { zmidi_mpe_set_layout(&mut c, lower, upper, &mut reset) };
                    match MpeLayout::new(lower, upper) {
                        Ok(l) => prop_assert_eq!((status, reset), (ZmidiStatus::Ok, r.set_layout(l))),
                        Err(_) => prop_assert_eq!(status, ZmidiStatus::InvalidArgument),
                    }
                }
                let ch = m.channel().index() as usize;
                let mut pe = ZmidiParameterEvent { change: 9, parameter: ZmidiParameter { registered: false, msb: 0, lsb: 0 }, value: 0 };
                prop_assert_eq!(unsafe { zmidi_parameters_receive(&mut cp[ch], &cv(m), &mut pe) }, ZmidiStatus::Ok);
                let mut e = ZmidiMpeEvent { kind: 99, zone: 9, manager: true, reset: 9 };
                prop_assert_eq!(unsafe { zmidi_mpe_receive(&mut c, &cv(m), &pe, &mut e) }, ZmidiStatus::Ok);
                let want = c_mpe_event(r.receive(&m, rp[ch].receive(&m)));
                prop_assert_eq!((e.kind, e.zone, e.manager, e.reset), want);
                let mut info = ZmidiMpeInfo { layout: ZmidiMpeLayout { lower: 99, upper: 99 }, mode: [0; 2], manager: [ZmidiMpeSensitivity { semitones: 0, cents: 0 }; 2], member: [ZmidiMpeSensitivity { semitones: 0, cents: 0 }; 2] };
                prop_assert_eq!(unsafe { zmidi_mpe_info(&c, &mut info) }, ZmidiStatus::Ok);
                let l = r.layout();
                prop_assert_eq!((info.layout.lower, info.layout.upper), (l.members(MpeZone::Lower), l.members(MpeZone::Upper)));
                for (k, z) in [MpeZone::Lower, MpeZone::Upper].into_iter().enumerate() {
                    let (s0, s1) = (r.sensitivity(z, true), r.sensitivity(z, false));
                    prop_assert_eq!(info.mode[k], r.mode(z));
                    prop_assert_eq!((info.manager[k].semitones, info.manager[k].cents), (s0.semitones, s0.cents));
                    prop_assert_eq!((info.member[k].semitones, info.member[k].cents), (s1.semitones, s1.cents));
                }
                let mut v = ZmidiMpeChannel { pitch_bend: 0, pressure: 0, brightness: 0, in_zone: false, pitch_offset: 9 };
                prop_assert_eq!(unsafe { zmidi_mpe_channel(&c, ch as u8, &mut v) }, ZmidiStatus::Ok);
                let rc = Channel::from_bits(ch as u8);
                let offset = r.pitch_offset(rc);
                prop_assert_eq!((v.pitch_bend, v.pressure, v.brightness, v.in_zone, v.pitch_offset), (r.pitch_bend(rc), r.pressure(rc), r.brightness(rc), offset.is_some(), offset.unwrap_or(0)));
            }
            let pe = ZmidiParameterEvent { change: 0, parameter: ZmidiParameter { registered: false, msb: 0, lsb: 0 }, value: 0 };
            let bad_pe = ZmidiParameterEvent { change: 7, ..pe };
            let m = cv(Midi1ChannelVoiceMessage::parse(&[0x2090_3C40]).unwrap());
            let mut e = ZmidiMpeEvent { kind: 0, zone: 0, manager: false, reset: 0 };
            prop_assert_eq!(unsafe { zmidi_mpe_receive(&mut c, &m, &bad_pe, &mut e) }, ZmidiStatus::InvalidArgument);
            let sys = of_word(ZmidiMidi1Kind::System, 0x10F8_0000);
            prop_assert_eq!(unsafe { zmidi_mpe_receive(&mut c, &sys, &pe, &mut e) }, ZmidiStatus::InvalidArgument);
            prop_assert_eq!(unsafe { zmidi_mpe_channel(&c, 16, &mut ZmidiMpeChannel { pitch_bend: 0, pressure: 0, brightness: 0, in_zone: false, pitch_offset: 0 }) }, ZmidiStatus::InvalidArgument);
        });
    }

    /// REQ-FFI-MIDI1-015: the C layout, class, pitch helper, writers and note allocator give the
    /// results of the Rust functions.
    #[test]
    fn prop_mpe_sender_equals_rust() {
        use zmidi_io::{MpeLayout, MpeMessage, MpeNoteAllocator, MpeRole, MpeSensitivity, MpeZone};
        let zones = [MpeZone::Lower, MpeZone::Upper];
        proptest!(|(lower in 0u8..16, upper in 0u8..16, zone in 0u32..3, members in 0u8..18, ops in proptest::collection::vec((0u8..3, 0u8..0x80, 0u8..16), 0..40), offset in any::<i32>(), value in 0u16..0x4001, semitones in 0u8..0x81, cents in 0u8..0x81, kind in 0u32..22, flags in 0u8..8)| {
            let mut out = ZmidiMpeLayout { lower: 99, upper: 99 };
            let rust = MpeLayout::new(lower, upper);
            prop_assert_eq!(unsafe { zmidi_mpe_layout_new(lower, upper, &mut out) }, if rust.is_ok() { ZmidiStatus::Ok } else { ZmidiStatus::InvalidArgument });
            let z = zones.get(zone as usize).copied();
            if let (Ok(l), Some(z)) = (rust, z) {
                prop_assert_eq!((out.lower, out.upper), (lower, upper));
                let mut after = ZmidiMpeLayout { lower: 99, upper: 99 };
                prop_assert_eq!(unsafe { zmidi_mpe_layout_with_mcm(lower, upper, zone, members, &mut after) }, ZmidiStatus::Ok);
                let a = l.with_mcm(z, members);
                prop_assert_eq!((after.lower, after.upper), (a.members(MpeZone::Lower), a.members(MpeZone::Upper)));
                for ch in 0..16u8 {
                    let (mut role, mut rz) = (9, 9);
                    prop_assert_eq!(unsafe { zmidi_mpe_layout_role(lower, upper, ch, &mut role, &mut rz) }, ZmidiStatus::Ok);
                    let want = match l.role(Channel::from_bits(ch)) { MpeRole::NoZone => (0, 0), MpeRole::Manager(z) => (1, z as u32), MpeRole::Member(z) => (2, z as u32) };
                    prop_assert_eq!((role, rz), want);
                }
                let (mut active, mut lowest) = (false, 99);
                prop_assert_eq!(unsafe { zmidi_mpe_layout_lowest_member(lower, upper, zone, &mut active, &mut lowest) }, ZmidiStatus::Ok);
                prop_assert_eq!((active, lowest), (l.lowest_member(z).is_some(), l.lowest_member(z).map_or(0, Channel::index)));
                // Writers.
                let mut buf = [of_word(ZmidiMidi1Kind::None, 0); 48];
                let mut n = 99;
                let st = unsafe { zmidi_mpe_mcm_messages(zone, members, buf.as_mut_ptr(), 48, &mut n) };
                match zmidi_io::mcm_messages(z, members) {
                    Ok(m) => { prop_assert_eq!((st, n), (ZmidiStatus::Ok, 3)); prop_assert_eq!(&buf[..3], &m.map(cv)[..]); }
                    Err(_) => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
                }
                let st = unsafe { zmidi_mpe_zone_sensitivity_messages(lower, upper, zone, semitones, members, buf.as_mut_ptr(), 48, &mut n) };
                let mut rb = [Midi1ChannelVoiceMessage::parse(&[0x20C0_0000]).unwrap(); 48];
                match zmidi_io::zone_sensitivity_messages(l, z, semitones, members, &mut rb) {
                    Ok(k) => { prop_assert_eq!((st, n), (ZmidiStatus::Ok, k)); prop_assert!(buf[..k].iter().zip(&rb[..k]).all(|(a, b)| *a == cv(*b))); }
                    Err(_) => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
                }
                let mut one = of_word(ZmidiMidi1Kind::None, 0);
                let st = unsafe { zmidi_mpe_mode_message(lower, upper, zone, flags & 1 != 0, &mut one) };
                match zmidi_io::mode_message(l, z, flags & 1 != 0) {
                    Ok(m) => prop_assert_eq!((st, one), (ZmidiStatus::Ok, cv(m))),
                    Err(_) => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
                }
                // The note allocator.
                let mut c = ZmidiMpeNoteAllocator { _opaque: [0; ZMIDI_MPE_NOTE_ALLOCATOR_WORDS] };
                prop_assert_eq!(unsafe { zmidi_mpe_note_allocator_init(&mut c) }, ZmidiStatus::Ok);
                prop_assert_eq!(unsafe { zmidi_mpe_set_same_note(&mut c, flags & 2 != 0) }, ZmidiStatus::Ok);
                let mut r = MpeNoteAllocator::new();
                r.set_same_note(flags & 2 != 0);
                for (op, note, ch) in ops {
                    if op < 2 {
                        let (mut found, mut got) = (false, 99);
                        prop_assert_eq!(unsafe { zmidi_mpe_note_on(&mut c, lower, upper, zone, note, &mut found, &mut got) }, ZmidiStatus::Ok);
                        let want = r.note_on(l, z, note);
                        prop_assert_eq!((found, got), (want.is_some(), want.map_or(0, Channel::index)));
                    } else {
                        prop_assert_eq!(unsafe { zmidi_mpe_note_off(&mut c, ch) }, ZmidiStatus::Ok);
                        r.note_off(Channel::from_bits(ch));
                    }
                    let mut count = 99;
                    prop_assert_eq!(unsafe { zmidi_mpe_active_notes(&c, ch, &mut count) }, ZmidiStatus::Ok);
                    prop_assert_eq!(count, r.active_notes(Channel::from_bits(ch)));
                }
                let mut on = false;
                prop_assert_eq!(unsafe { zmidi_mpe_note_reset(&mut c) }, ZmidiStatus::Ok);
                prop_assert_eq!(unsafe { zmidi_mpe_same_note(&c, &mut on) }, ZmidiStatus::Ok);
                prop_assert_eq!(on, flags & 2 != 0);
            }
            if z.is_none() {
                let mut after = ZmidiMpeLayout { lower: 0, upper: 0 };
                prop_assert_eq!(unsafe { zmidi_mpe_layout_with_mcm(0, 0, zone, members, &mut after) }, ZmidiStatus::InvalidArgument);
            }
            // The pitch helper and the class function.
            let rs = MpeSensitivity { semitones, cents };
            let (mut pb, mut off) = (0u16, 0i32);
            let ok = semitones <= 0x7F && cents <= 0x7F;
            prop_assert_eq!(unsafe { zmidi_mpe_pitch_bend_value(offset, semitones, cents, &mut pb) }, if ok { ZmidiStatus::Ok } else { ZmidiStatus::InvalidArgument });
            prop_assert_eq!(unsafe { zmidi_mpe_pitch_offset(value, semitones, cents, &mut off) }, if ok && value <= 0x3FFF { ZmidiStatus::Ok } else { ZmidiStatus::InvalidArgument });
            if ok {
                prop_assert_eq!(pb, zmidi_io::pitch_bend_value(offset, rs));
                if value <= 0x3FFF { prop_assert_eq!(off, zmidi_io::pitch_offset(value, rs)); }
            }
            let kinds = [MpeMessage::Mcm, MpeMessage::PitchBendSensitivity, MpeMessage::PitchBend, MpeMessage::ChannelPressure, MpeMessage::Brightness, MpeMessage::PolyPressure, MpeMessage::AllSoundOff, MpeMessage::ResetAllControllers, MpeMessage::LocalControl, MpeMessage::AllNotesOff, MpeMessage::OmniOff, MpeMessage::OmniOn, MpeMessage::MonoModeOn, MpeMessage::PolyModeOn, MpeMessage::OtherControlChange, MpeMessage::OtherRpn, MpeMessage::Nrpn, MpeMessage::ProgramChange, MpeMessage::BankSelect, MpeMessage::Note, MpeMessage::System];
            let mut class = 99;
            let st = unsafe { zmidi_mpe_class(kind, flags & 1 != 0, flags & 2 != 0, flags & 4 != 0, &mut class) };
            match kinds.get(kind as usize) {
                Some(&k) => prop_assert_eq!((st, class), (ZmidiStatus::Ok, zmidi_io::mpe_class(k, flags & 1 != 0, flags & 2 != 0, flags & 4 != 0) as u32)),
                None => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
            }
            // The note writer.
            let mut buf = [of_word(ZmidiMidi1Kind::None, 0); 4];
            let mut n = 0;
            let st = unsafe { zmidi_mpe_note_messages(members, semitones, cents, value, flags * 20, semitones, buf.as_mut_ptr(), 4, &mut n) };
            match Channel::new(members).and_then(|ch| zmidi_io::note_messages(ch, semitones, cents, value, flags * 20, semitones)) {
                Ok(m) => prop_assert_eq!((st, n, buf), (ZmidiStatus::Ok, 4, m.map(cv))),
                Err(_) => prop_assert_eq!(st, ZmidiStatus::InvalidArgument),
            }
        });
    }
}

/// REQ-FFI-ABI-004: the features of the MIDI 1.0-only build turn on no MIDI 2.0 feature of a
/// dependency, and io, ble and usb keep their MIDI 2.0 features (rule R0 of DEC-022).
#[test]
fn midi1_features_have_no_midi2_dependency() {
    let manifest = include_str!("../Cargo.toml");
    let features = &manifest[manifest.find("[features]").unwrap()..];
    let list = |name: &str| -> Vec<String> {
        let line = features
            .lines()
            .find(|l| l.starts_with(&format!("{name} = [")))
            .unwrap_or_else(|| panic!("feature {name}"));
        line[line.find('[').unwrap() + 1..line.rfind(']').unwrap()]
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    // The closure of `midi1` over the features of this crate.
    let mut todo = vec!["midi1".to_string()];
    let mut all = Vec::new();
    while let Some(f) = todo.pop() {
        for item in list(&f) {
            if !item.contains(':') && !item.contains('/') {
                todo.push(item.clone());
            }
            all.push(item);
        }
    }
    for midi2 in [
        "midi2-protocol",
        "alt1",
        "channel-voice2",
        "/full",
        "zmidi-ble/midi2",
    ] {
        assert!(
            !all.iter().any(|i| i.contains(midi2)),
            "midi1 turns on {midi2}: {all:?}"
        );
    }
    assert_eq!(
        list("io"),
        [
            "io-midi1",
            "zmidi-io/midi2-protocol",
            "zmidi-ump/channel-voice2"
        ]
    );
    assert_eq!(list("ble"), ["ble-midi1", "zmidi-ble/midi2-protocol"]);
    assert_eq!(list("usb"), ["usb-midi1", "zmidi-usb/alt1"]);
}

/// REQ-FFI-ABI-005: the C ABI check script builds fw_midi1, checks its flash budget and fails on a
/// MIDI 2.0 symbol.
#[test]
fn ffi_check_has_the_midi1_firmware() {
    let script = include_str!("../../../scripts/ffi-check.sh");
    assert!(
        script.contains("ZMIDI_CHECK_MIDI1_ONLY=ON"),
        "fw_midi1 build"
    );
    assert!(script.contains("fw_midi1 flash"), "fw_midi1 budget");
    assert!(
        script.contains("fw_midi1 has MIDI 2.0 code"),
        "fw_midi1 symbol check"
    );
}

/// REQ-FFI-SX-001 to REQ-FFI-SX-009, REQ-FFI-MIDI1-016: the C ABI of zmidi-sysex and of the GM
/// receiver gives the results of the Rust types.
#[cfg(feature = "midi1")]
mod sysex {
    use proptest::prelude::*;
    use zmidi::midi1::*;
    use zmidi::status::ZmidiStatus;
    use zmidi::sysex::*;
    use zmidi_sysex::{
        BarMarker, DeviceControl, GeneralMidi, InfoField, InfoFields, InquiryMessage, MmcCommand,
        MmcMessage, MtcGenerator, MtcReader, MtcTime, Sysex, SysexId, TimeCodeType, TimeSignature,
        TuningMessage, Universal,
    };

    fn universal(bytes: &[u8]) -> Option<Universal<'_>> {
        Universal::parse(&Sysex::parse(bytes).ok()?).ok()?
    }

    /// A Universal message with a header, a Sub-ID#1 of a part and random data.
    fn message() -> impl Strategy<Value = Vec<u8>> {
        (
            any::<bool>(),
            0u8..0x80,
            prop::sample::select(vec![0x01u8, 0x04, 0x06, 0x08, 0x09]),
            proptest::collection::vec(0u8..0x80, 0..24),
        )
            .prop_map(|(rt, dev, sub1, data)| {
                let mut v = vec![0xF0, if rt { 0x7F } else { 0x7E }, dev, sub1];
                v.extend(data);
                v.push(0xF7);
                v
            })
    }

    #[test]
    fn prop_sysex_parse_equals_rust() {
        proptest!(|(bytes in proptest::collection::vec(prop_oneof![0u8..0x80, Just(0xF0u8), Just(0xF7u8), any::<u8>()], 0..20))| {
            let mut c = ZmidiSysex { kind: 9, manufacturer: [9; 3], device_id: 0, sub_id1: 0, sub_id2: 0, has_sub_id2: false, data_offset: 0, data_length: 0 };
            let got = unsafe { zmidi_sysex_parse(bytes.as_ptr(), bytes.len(), &mut c) };
            let want = Sysex::parse(&bytes).ok().and_then(|s| match s.id {
                SysexId::Manufacturer(id) => Some((ZmidiSysexClass::Manufacturer as u32, id.to_midi1(), None, s.data)),
                SysexId::NonCommercial => Some((ZmidiSysexClass::NonCommercial as u32, [0; 3], None, s.data)),
                _ => Universal::parse(&s).ok().flatten().map(|u| {
                    let k = if u.real_time { ZmidiSysexClass::RealTime } else { ZmidiSysexClass::NonRealTime };
                    (k as u32, [0; 3], Some((u.device_id, u.sub_id1, u.sub_id2)), u.data)
                }),
            });
            match want {
                None => prop_assert_eq!(got, ZmidiStatus::InvalidArgument),
                Some((kind, m, header, data)) => {
                    prop_assert_eq!(got, ZmidiStatus::Ok);
                    prop_assert_eq!((c.kind, c.manufacturer), (kind, m));
                    if let Some((d, s1, s2)) = header {
                        prop_assert_eq!((c.device_id, c.sub_id1, c.has_sub_id2.then_some(c.sub_id2)), (d, s1, s2));
                    }
                    let at = c.data_offset as usize;
                    prop_assert_eq!(&bytes[at..at + c.data_length as usize], data);
                }
            }
        });
    }

    /// The status class of a Rust part reader result (REQ-FFI-SX-002).
    fn class<T>(r: &Option<Result<Option<T>, zmidi_sysex::SysexError>>) -> ZmidiStatus {
        match r {
            Some(Ok(Some(_))) => ZmidiStatus::Ok,
            Some(Ok(None)) => ZmidiStatus::NoPending,
            _ => ZmidiStatus::InvalidArgument,
        }
    }

    #[test]
    fn prop_sysex_parts_equal_rust() {
        proptest!(|(bytes in message())| {
            let u = universal(&bytes);
            let (p, n) = (bytes.as_ptr(), bytes.len());
            let mut out = [0u8; 600];
            let mut len = 0usize;
            // Inquiry: the status class, and the responder against zmidi_sysex::respond.
            let want = u.as_ref().map(InquiryMessage::from_universal);
            let (mut reply, mut dev) = (false, 0u8);
            let mut id = ZmidiIdentity { manufacturer: [0x41, 0, 0], family: 1, member: 2, revision: [0, 0, 0, 1] };
            prop_assert_eq!(unsafe { zmidi_inquiry_read(p, n, &mut reply, &mut dev, &mut id) }, class(&want));
            let ident = zmidi_sysex::Identity::new(zmidi_ump::ManufacturerId::from_midi1(&[0x41]).unwrap(), 1, 2, [0, 0, 0, 1]).unwrap();
            let c_id = ZmidiIdentity { manufacturer: [0x41, 0, 0], family: 1, member: 2, revision: [0, 0, 0, 1] };
            let got = unsafe { zmidi_inquiry_respond(p, n, 0x10, &c_id, out.as_mut_ptr(), out.len(), &mut len) };
            let mut rust = [0u8; 600];
            match want {
                Some(Ok(Some(m))) => match zmidi_sysex::respond(&m, 0x10, &ident, &mut rust) {
                    Ok(Some(k)) => { prop_assert_eq!(got, ZmidiStatus::Ok); prop_assert_eq!(&out[..len], &rust[..k]); }
                    _ => prop_assert_eq!(got, ZmidiStatus::NoPending),
                },
                ref w => prop_assert_eq!(got, class(w)),
            }
            // General MIDI and Device Control: the status class and the values.
            let want = u.as_ref().map(GeneralMidi::from_universal);
            let (mut d, mut cmd) = (0u8, 0u8);
            prop_assert_eq!(unsafe { zmidi_gm_system_read(p, n, &mut d, &mut cmd) }, class(&want));
            if let Some(Ok(Some(m))) = want {
                prop_assert_eq!((d, cmd), (m.device_id, m.command.into_bits()));
            }
            let want = u.as_ref().map(DeviceControl::from_universal);
            let mut dc = ZmidiDeviceControl { kind: 9, device_id: 0, value: 0 };
            prop_assert_eq!(unsafe { zmidi_device_control_read(p, n, &mut dc) }, class(&want));
            if let Some(Ok(Some(m))) = want {
                prop_assert_eq!(unsafe { zmidi_device_control_write(&dc, out.as_mut_ptr(), out.len(), &mut len) }, ZmidiStatus::Ok);
                let k = m.write(&mut rust).unwrap();
                prop_assert_eq!(&out[..len], &rust[..k]);
            }
            // MIDI Tuning: the status class, the kind and the notes.
            let want = u.as_ref().map(TuningMessage::from_universal);
            let mut t = core::mem::MaybeUninit::<ZmidiTuning>::zeroed();
            let mut notes = [ZmidiTuningNote { key: 0, word: ZmidiFrequencyWord { semitone: 0, fraction: 0 } }; 128];
            let got = unsafe { zmidi_tuning_read(p, n, t.as_mut_ptr(), notes.as_mut_ptr(), notes.len()) };
            prop_assert_eq!(got, class(&want));
            if let Some(Ok(Some(TuningMessage::NoteChange { changes, .. }))) = want {
                let t = unsafe { t.assume_init() };
                prop_assert_eq!(t.count as usize, changes.len());
                for (c, (k, w)) in notes.iter().zip(changes) {
                    prop_assert_eq!((c.key, c.word.semitone, c.word.fraction), (k, w.semitone, w.fraction));
                }
            }
        });
    }

    #[test]
    fn prop_notation_equals_rust() {
        // REQ-FFI-SX-011: the Notation readers and writers give what zmidi-sysex gives.
        proptest!(|(
            rt in any::<bool>(),
            dev in 0u8..0x80,
            sub2 in prop::sample::select(vec![0x01u8, 0x02, 0x42, 0x03]),
            data in proptest::collection::vec(0u8..0x80, 0..12),
            ln_ok in any::<bool>(),
        )| {
            let mut bytes = vec![0xF0, if rt { 0x7F } else { 0x7E }, dev, 0x03, sub2];
            let mut data = data;
            if ln_ok && !data.is_empty() {
                data[0] = data.len() as u8 - 1;
            }
            bytes.extend(&data);
            bytes.push(0xF7);
            let u = universal(&bytes);
            let (p, n) = (bytes.as_ptr(), bytes.len());
            let mut out = [0u8; 40];
            let mut rust = [0u8; 40];
            let mut len = 0usize;
            let want = u.as_ref().map(BarMarker::from_universal);
            let (mut d, mut bar) = (0u8, 0i16);
            prop_assert_eq!(unsafe { zmidi_bar_marker_read(p, n, &mut d, &mut bar) }, class(&want));
            if let Some(Ok(Some(m))) = want {
                prop_assert_eq!((d, bar), (m.device_id, m.bar.value()));
                prop_assert_eq!(unsafe { zmidi_bar_marker_write(d, bar, out.as_mut_ptr(), out.len(), &mut len) }, ZmidiStatus::Ok);
                let k = m.write(&mut rust).unwrap();
                prop_assert_eq!(&out[..len], &rust[..k]);
            }
            let want = u.as_ref().map(TimeSignature::from_universal);
            let mut t = core::mem::MaybeUninit::<ZmidiTimeSignature>::zeroed();
            prop_assert_eq!(unsafe { zmidi_time_signature_read(p, n, t.as_mut_ptr()) }, class(&want));
            if let Some(Ok(Some(m))) = want {
                let t = unsafe { t.assume_init() };
                let at = t.additional_offset as usize;
                let extra = &bytes[at..at + t.additional_length as usize];
                prop_assert_eq!(extra, m.additional);
                prop_assert_eq!((t.device_id, t.delayed, t.numerator, t.denominator_power), (m.device_id, m.delayed, m.numerator, m.denominator_power));
                prop_assert_eq!((t.clocks_per_click, t.thirty_seconds_per_quarter), (m.clocks_per_click, m.thirty_seconds_per_quarter));
                let got = unsafe {
                    zmidi_time_signature_write(t.device_id, t.delayed, t.numerator, t.denominator_power, t.clocks_per_click, t.thirty_seconds_per_quarter, extra.as_ptr(), extra.len(), out.as_mut_ptr(), out.len(), &mut len)
                };
                prop_assert_eq!(got, ZmidiStatus::Ok);
                let k = m.write(&mut rust).unwrap();
                prop_assert_eq!(&out[..len], &rust[..k]);
            }
        });
    }

    fn c_time(t: MtcTime) -> ZmidiMtcTime {
        ZmidiMtcTime {
            hours: t.hours,
            minutes: t.minutes,
            seconds: t.seconds,
            frames: t.frames,
            rate: t.rate.into_bits(),
        }
    }

    #[test]
    fn prop_mtc_equals_rust() {
        proptest!(|(bytes in proptest::collection::vec(0u8..0x80, 0..60), full_at in 0usize..60, ops in proptest::collection::vec(0u8..5, 0..40), start in 0u32..2_000_000, rate in 0u8..4)| {
            // The reader: each event and the state after each byte.
            let mut c = ZmidiMtcReader { _opaque: [0; ZMIDI_MTC_READER_WORDS] };
            prop_assert_eq!(unsafe { zmidi_mtc_reader_init(&mut c) }, ZmidiStatus::Ok);
            let mut r = MtcReader::new();
            let time = MtcTime::from_frame_count(start, TimeCodeType::from_bits(rate));
            for (i, &b) in bytes.iter().enumerate() {
                if i == full_at {
                    prop_assert_eq!(unsafe { zmidi_mtc_reader_full_message(&mut c, &c_time(time)) }, ZmidiStatus::Ok);
                    r.full_message(&zmidi_sysex::FullMessage { device_id: 0x7F, time });
                }
                let mut e = core::mem::MaybeUninit::<ZmidiMtcEvent>::zeroed();
                prop_assert_eq!(unsafe { zmidi_mtc_reader_receive(&mut c, b, e.as_mut_ptr()) }, ZmidiStatus::Ok);
                let (e, w) = (unsafe { e.assume_init() }, r.receive(b));
                prop_assert_eq!((e.piece, e.boundary, e.has_sequence), (w.piece, w.boundary, w.sequence.is_some()));
                if let Some(s) = w.sequence {
                    prop_assert_eq!((e.raw, e.current, e.discontinuity), (c_time(s.raw), c_time(s.current), s.discontinuity));
                }
                let mut info = core::mem::MaybeUninit::<ZmidiMtcReaderInfo>::zeroed();
                prop_assert_eq!(unsafe { zmidi_mtc_reader_info(&mut c, info.as_mut_ptr()) }, ZmidiStatus::Ok);
                let info = unsafe { info.assume_init() };
                prop_assert_eq!((info.locked, info.running, info.has_time), (r.is_locked(), r.is_running(), r.time().is_some()));
                prop_assert_eq!(info.time, r.time().map_or(c_time(MtcTime::from_frame_count(0, TimeCodeType::Fps24)), c_time));
            }
            // The generator: each byte after random direction, run and locate changes.
            let mut g = ZmidiMtcGenerator { _opaque: [0; ZMIDI_MTC_GENERATOR_WORDS] };
            prop_assert_eq!(unsafe { zmidi_mtc_generator_init(&mut g, &c_time(time), false) }, ZmidiStatus::Ok);
            let mut w = MtcGenerator::new(time, zmidi_sysex::Direction::Forward);
            for op in ops {
                match op {
                    0 => { unsafe { assert_eq!(zmidi_mtc_generator_set_direction(&mut g, true), ZmidiStatus::Ok) }; w.set_direction(zmidi_sysex::Direction::Reverse); }
                    1 => { unsafe { assert_eq!(zmidi_mtc_generator_set_direction(&mut g, false), ZmidiStatus::Ok) }; w.set_direction(zmidi_sysex::Direction::Forward); }
                    2 => { unsafe { assert_eq!(zmidi_mtc_generator_set_running(&mut g, false), ZmidiStatus::Ok) }; w.set_running(false); }
                    3 => { unsafe { assert_eq!(zmidi_mtc_generator_set_running(&mut g, true), ZmidiStatus::Ok) }; w.set_running(true); }
                    _ => {}
                }
                let mut b = 0u8;
                let got = unsafe { zmidi_mtc_generator_next(&mut g, &mut b) };
                match w.next_byte() {
                    Some(x) => { prop_assert_eq!(got, ZmidiStatus::Ok); prop_assert_eq!(b, x); }
                    None => prop_assert_eq!(got, ZmidiStatus::NoPending),
                }
            }
        });
    }

    #[test]
    fn prop_mmc_equals_rust() {
        proptest!(|(string in proptest::collection::vec(prop_oneof![0u8..0x10, 0x40u8..0x50, 0u8..0x80], 1..30))| {
            // The command reader: each command kind and the error, at the same positions.
            let u = Universal { real_time: true, device_id: 0x7F, sub_id1: 0x06, sub_id2: None, data: &string };
            let Ok(Some(MmcMessage::Commands { commands, .. })) = MmcMessage::from_universal(&u) else { panic!() };
            let mut at = 0usize;
            for want in commands {
                let mut c = core::mem::MaybeUninit::<ZmidiMmcCommand>::zeroed();
                let got = unsafe { zmidi_mmc_next_command(string.as_ptr(), string.len(), &mut at, c.as_mut_ptr()) };
                match want {
                    Err(_) => { prop_assert_eq!(got, ZmidiStatus::InvalidArgument); prop_assert_eq!(at, string.len()); }
                    Ok(w) => {
                        prop_assert_eq!(got, ZmidiStatus::Ok);
                        let c = unsafe { c.assume_init() };
                        let kind = match w {
                            MmcCommand::Write(_) => ZmidiMmcCommandKind::Write,
                            MmcCommand::LocateField { .. } => ZmidiMmcCommandKind::LocateField,
                            MmcCommand::LocateTarget(_) => ZmidiMmcCommandKind::LocateTarget,
                            MmcCommand::Move { .. } => ZmidiMmcCommandKind::Move,
                            MmcCommand::Other { .. } => ZmidiMmcCommandKind::Other,
                            _ => ZmidiMmcCommandKind::Stop,
                        };
                        if kind != ZmidiMmcCommandKind::Stop {
                            prop_assert_eq!(c.kind, kind as u32);
                        }
                        // A command read back writes the same bytes as the Rust writer.
                        let (mut out, mut len, mut rust) = ([0u8; 64], 0usize, [0u8; 64]);
                        let got = unsafe { zmidi_mmc_write_commands(0x7F, &c, 1, string.as_ptr(), string.len(), out.as_mut_ptr(), out.len(), &mut len) };
                        match zmidi_sysex::write_commands(&mut rust, 0x7F, &[w]) {
                            Ok(k) => { prop_assert_eq!(got, ZmidiStatus::Ok); prop_assert_eq!(&out[..len], &rust[..k]); }
                            Err(_) => prop_assert_eq!(got, ZmidiStatus::InvalidArgument),
                        }
                    }
                }
            }
            let mut c = core::mem::MaybeUninit::<ZmidiMmcCommand>::zeroed();
            prop_assert_eq!(unsafe { zmidi_mmc_next_command(string.as_ptr(), string.len(), &mut at, c.as_mut_ptr()) }, ZmidiStatus::NoPending);
            // The Information Field reader.
            let mut at = 0usize;
            for want in InfoFields::new(&string) {
                let mut f = core::mem::MaybeUninit::<ZmidiMmcField>::zeroed();
                let got = unsafe { zmidi_mmc_next_field(string.as_ptr(), string.len(), &mut at, f.as_mut_ptr()) };
                match want {
                    Err(_) => prop_assert_eq!(got, ZmidiStatus::InvalidArgument),
                    Ok(w) => {
                        prop_assert_eq!(got, ZmidiStatus::Ok);
                        let f = unsafe { f.assume_init() };
                        let kind = match w {
                            InfoField::TimeCode { .. } => ZmidiMmcFieldKind::TimeCode,
                            InfoField::Short { .. } => ZmidiMmcFieldKind::Short,
                            InfoField::Other { .. } => ZmidiMmcFieldKind::Other,
                        };
                        prop_assert_eq!(f.kind, kind as u32);
                    }
                }
            }
        });
    }

    #[test]
    fn prop_gm_receiver_equals_rust() {
        use zmidi_io::{GmConfig, GmLevel, GmReceiver};
        proptest!(|(capable in any::<u16>(), mono in any::<bool>(), steps in proptest::collection::vec((0u8..4, 0u8..16, 0u8..0x80, 0u8..0x80), 0..40))| {
            let mut c = ZmidiGmReceiver { _opaque: [0; ZMIDI_GM_RECEIVER_WORDS] };
            prop_assert_eq!(unsafe { zmidi_gm_receiver_init(&mut c, &ZmidiGmConfig { rhythm_capable: capable, rhythm_mono: mono }) }, ZmidiStatus::Ok);
            let mut r = GmReceiver::new(GmConfig::new(capable, mono));
            for (op, ch, a, b) in steps {
                let channel = zmidi_ump::Channel::from_bits(ch);
                match op {
                    0 => {
                        let bank = u16::from(a) << 7 | u16::from(b);
                        prop_assert_eq!(unsafe { zmidi_gm_receiver_program(&mut c, ch, true, bank) }, ZmidiStatus::Ok);
                        r.program(channel, Some(bank));
                    }
                    1 => {
                        let level = u32::from(a % 3);
                        prop_assert_eq!(unsafe { zmidi_gm_receiver_reset(&mut c, level) }, ZmidiStatus::Ok);
                        r.reset([GmLevel::Off, GmLevel::Gm1, GmLevel::Gm2][level as usize]);
                    }
                    _ => {
                        // A Control Change (also a Channel Mode message) or a Note On.
                        let status = if op == 2 { 0xB0 } else { 0x90 } | ch;
                        let word = 0x2000_0000 | u32::from(status) << 16 | u32::from(a | (0x78 * u8::from(op == 2 && a & 1 == 0))) << 8 | u32::from(b);
                        let m = Midi1ChannelVoiceMessageAt::of(word);
                        let mut act = core::mem::MaybeUninit::<ZmidiModeAction>::zeroed();
                        prop_assert_eq!(unsafe { zmidi_gm_receiver_receive(&mut c, &m.0, act.as_mut_ptr()) }, ZmidiStatus::Ok);
                        let want = r.receive(&m.1);
                        let act = unsafe { act.assume_init() };
                        let kind = match want {
                            zmidi_io::ModeAction::Ignore => 0,
                            zmidi_io::ModeAction::Voice { .. } => 1,
                            zmidi_io::ModeAction::Global => 2,
                            zmidi_io::ModeAction::AllNotesOff { .. } => 3,
                            zmidi_io::ModeAction::AllSoundOff { .. } => 4,
                            zmidi_io::ModeAction::ResetAllControllers { .. } => 5,
                            zmidi_io::ModeAction::LocalControl { .. } => 6,
                            zmidi_io::ModeAction::ModeChange { .. } => 7,
                        };
                        prop_assert_eq!(act.kind, kind);
                    }
                }
                let mut info = core::mem::MaybeUninit::<ZmidiGmInfo>::zeroed();
                prop_assert_eq!(unsafe { zmidi_gm_receiver_info(&mut c, info.as_mut_ptr()) }, ZmidiStatus::Ok);
                let info = unsafe { info.assume_init() };
                let mono_mask = (0..16u8).fold(0u16, |m, n| m | u16::from(r.is_mono(zmidi_ump::Channel::from_bits(n))) << n);
                prop_assert_eq!((info.rhythm_channels, info.mono_channels), (r.rhythm_channels(), mono_mask));
            }
        });
    }

    /// A C message and the Rust message of one MIDI 1.0 Channel Voice word.
    struct Midi1ChannelVoiceMessageAt(ZmidiMidi1Message, zmidi_ump::Midi1ChannelVoiceMessage);

    impl Midi1ChannelVoiceMessageAt {
        fn of(w: u32) -> Self {
            let c = ZmidiMidi1Message {
                kind: ZmidiMidi1Kind::ChannelVoice as u32,
                status: (w >> 16) as u8,
                data1: (w >> 8) as u8 & 0x7F,
                data2: w as u8 & 0x7F,
                length: 0,
            };
            Self(c, zmidi_ump::Midi1ChannelVoiceMessage::parse(&[w]).unwrap())
        }
    }

    #[test]
    fn sysex_storage_holds_the_rust_types() {
        use core::mem::size_of;
        assert!(zmidi_mtc_generator_size() <= size_of::<ZmidiMtcGenerator>());
        assert!(zmidi_mtc_reader_size() <= size_of::<ZmidiMtcReader>());
        assert!(zmidi_gm_receiver_size() <= size_of::<ZmidiGmReceiver>());
    }
}
