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
