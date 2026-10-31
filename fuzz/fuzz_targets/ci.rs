//! MIDI-CI messages from a peer: `CiDevice::receive` with all categories, its automatic replies,
//! the one-chunk Property Exchange Responder and the full `PeEngine` (with zlib). A known peer
//! is discovered first, so that its messages reach the category handlers.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use zmidi_ci::{
    Categories, CiDevice, DeviceConfig, DeviceIdentity, Discovery, Framing, ManufacturerId, Muid,
    PiCapabilities,
};
use zmidi_ci_pe::one_chunk::{respond, respond_flat, Resources};
use zmidi_ci_pe::{CanSet, EngineConfig, PeEngine, ResourceEntry, Status};

type Dev = CiDevice<'static, 4, 2, 4>;

fn config(m: u8, framing: Framing) -> DeviceConfig<'static> {
    DeviceConfig {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::OneByte(m),
            family: 1,
            model: 2,
            revision: [0, 0, 0, 1],
        },
        categories: Categories {
            protocol_negotiation: false,
            profile_configuration: true,
            property_exchange: true,
            process_inquiry: true,
        },
        max_sysex_size: 512,
        function_block: Some(1),
        first_group: 0,
        group_count: 2,
        channels_in_use: 0x0003,
        product_instance_id: Some("fuzz-1"),
        framing,
        process_inquiry: PiCapabilities {
            midi_message_report: true,
        },
    }
}

/// One Resource "X-State" with a value of up to 64 bytes.
#[derive(Default)]
struct App {
    state: Vec<u8>,
}

impl Resources for App {
    fn get(&mut self, resource: &[u8], _: Option<&[u8]>, out: &mut [u8]) -> Result<usize, Status> {
        if resource != b"X-State" {
            return Err(Status::NOT_FOUND);
        }
        let n = self.state.len().min(out.len());
        out[..n].copy_from_slice(&self.state[..n]);
        Ok(n)
    }

    fn set(&mut self, resource: &[u8], _: Option<&[u8]>, data: &[u8]) -> Result<(), Status> {
        if resource != b"X-State" || data.len() > 64 {
            return Err(Status::NOT_FOUND);
        }
        self.state = data.to_vec();
        Ok(())
    }
}

const LIST: &[u8] = br#"[{"resource":"DeviceInfo"},{"resource":"X-State","canSet":"full"}]"#;

/// The Resource table of the engine: ASCII, Mcoded7, zlib, Subscription and Flow Control.
fn resources() -> &'static [ResourceEntry<'static>] {
    static TABLE: OnceLock<Vec<ResourceEntry<'static>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut patch = ResourceEntry::new("X-Patch");
        patch.caps.can_set = CanSet::Partial;
        patch.caps.can_subscribe = true;
        patch.caps.encodings.mcoded7 = true;
        let mut flow = ResourceEntry::new("X-Flow");
        flow.caps.can_set = CanSet::Full;
        flow.caps.fc_on_get = true;
        flow.caps.fc_on_set = true;
        let mut zip = ResourceEntry::new("X-Zip");
        zip.caps.can_set = CanSet::Full;
        zip.caps.encodings.zlib_mcoded7 = true;
        let mut list = ResourceEntry::new("ProgramList");
        list.caps.can_paginate = true;
        vec![ResourceEntry::new("DeviceInfo"), patch, flow, zip, list]
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let framing = if mode & 1 == 0 {
        Framing::Bare
    } else {
        Framing::SysEx
    };
    let mut seed = 0x1234_5678u32;
    let mut rng = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed
    };
    let Ok(mut dev) = Dev::new(config(0x42, framing), &mut rng) else {
        return;
    };
    let mut buf = [0u8; 600];
    let peer = Muid::new(0x0111_1111).unwrap();
    let discovery = Discovery {
        identity: config(0x41, framing).identity,
        categories: config(0x41, framing).categories,
        max_sysex_size: 512 + u32::from(mode >> 2) * 64,
        output_path_id: Some(0),
    };
    if let Ok(n) = discovery.write(peer, framing, &mut buf) {
        let msg = buf[..n].to_vec();
        let mut r = dev.receive(0, &msg, 0, &mut rng);
        for _ in 0..64 {
            if !matches!(r.write_next(&mut buf), Some(Ok(_))) {
                break;
            }
        }
    }
    let mut app = App::default();
    let mut engine: PeEngine<'static, 2, 256, 4, 2> = PeEngine::new(EngineConfig {
        resources: resources(),
        zlib_window: 4096,
    });
    let (mut body, mut out, mut scratch) = ([0u8; 1024], [0u8; 600], vec![0u8; 8192]);
    let mut now = 0u64;
    for (i, msg) in zmidi_fuzz::parts(rest).enumerate() {
        now += 250_000 * (i as u64 % 5);
        let mut r = dev.receive((i % 2) as u8, msg, now, &mut rng);
        let event = r.event();
        for _ in 0..64 {
            if !matches!(r.write_next(&mut buf), Some(Ok(_))) {
                break;
            }
        }
        let Some(event) = event else { continue };
        if mode & 2 == 0 {
            let _ = respond(
                &mut dev,
                &event,
                LIST,
                &mut app,
                &mut body,
                &mut out,
                |_| {},
            );
        } else {
            let _ = respond_flat(
                &mut dev,
                &event,
                LIST,
                &mut app,
                &mut body,
                &mut out,
                |_| {},
            );
        }
        let _ = engine.receive(&mut dev, &event, now / 1000, &mut scratch);
        while engine.poll(&mut dev, now / 1000).is_some() {}
        for _ in 0..64 {
            if !matches!(
                engine.write_pending(&mut dev, now / 1000, &mut out),
                Some(Ok(_))
            ) {
                break;
            }
        }
    }
});
