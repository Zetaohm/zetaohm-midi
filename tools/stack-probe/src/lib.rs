//! Stack depth probe (docs/optimization-backlog.md, OPT-003). Each `probe_*` function calls one main
//! entry point with inputs that the compiler cannot see, so that the code is kept. Nothing calls
//! the functions; `scripts/stack-check.py` reads their deepest call chain from the disassembly.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::hint::black_box;
use core::net::SocketAddr;

use zmidi_ci::{CiDevice, Event};
use zmidi_ci_pe::{EngineConfig, PeEngine};
use zmidi_io::BytesToUmp;
use zmidi_net::{Client, ClientConfig, Host, HostConfig, NoCredentials};
use zmidi_ump::{Group, StreamProtocol};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

type NetHost = Host<1>;
type Engine = PeEngine<'static, 2, 256, 4, 2>;
type Device = CiDevice<'static, 4, 4, 8>;

#[no_mangle]
pub fn probe_net_host_new(cfg: &HostConfig, out: &mut Option<NetHost>) {
    *out = NetHost::new(black_box(*cfg)).ok();
}

#[no_mangle]
pub fn probe_net_host_init(cfg: &HostConfig, slot: &mut Option<NetHost>) {
    black_box(NetHost::init(slot, black_box(*cfg)).is_ok());
}

#[no_mangle]
pub fn probe_net_client_init(cfg: &ClientConfig, slot: &mut Option<Client<1>>) {
    black_box(Client::<1>::init(slot, black_box(*cfg)).is_ok());
}

#[no_mangle]
pub fn probe_net_client_new(cfg: &ClientConfig, out: &mut Option<Client<1>>) {
    *out = Client::<1>::new(black_box(*cfg)).ok();
}

#[no_mangle]
pub fn probe_net_host_receive(h: &mut NetHost, now: u64, from: SocketAddr, payload: &[u8], buf: &mut [u8]) {
    let mut creds = NoCredentials;
    let mut r = h.receive(now, from, payload, &mut creds);
    for ev in &mut r {
        black_box(&ev);
    }
    while let Some(x) = r.write_next(buf) {
        black_box(x);
    }
}

#[no_mangle]
pub fn probe_net_host_poll(h: &mut NetHost, now: u64, buf: &mut [u8]) {
    for ev in h.poll(now) {
        black_box(&ev);
    }
    black_box(h.write_pending(now, buf));
}

#[no_mangle]
pub fn probe_pe_new(cfg: &EngineConfig<'static>, out: &mut Option<Engine>) {
    *out = Some(Engine::new(black_box(*cfg)));
}

#[no_mangle]
pub fn probe_pe_receive(e: &mut Engine, d: &mut Device, ev: &Event<'static>, now: u64, scratch: &mut [u8]) {
    black_box(e.receive(d, ev, now, scratch).ok());
}

#[no_mangle]
pub fn probe_pe_write_pending(e: &mut Engine, d: &mut Device, now: u64, out: &mut [u8]) {
    black_box(e.write_pending(d, now, out));
}

#[no_mangle]
pub fn probe_io_bytes_to_ump_new(group: u8, out: &mut Option<BytesToUmp>) {
    let g = Group::new(group & 0x0F).unwrap_or_default();
    *out = Some(BytesToUmp::new(black_box(g), StreamProtocol::Midi2));
}

#[cfg(feature = "zlib")]
#[no_mangle]
pub fn probe_pe_zlib_decode(window: &'static mut [u8], input: &[u8], out: &mut [u8]) {
    let mut d = zmidi_ci_pe::ZlibMcoded7Decoder::new(window);
    black_box(d.decode(input, out).ok());
}

#[cfg(feature = "zlib")]
#[no_mangle]
pub fn probe_pe_zlib_reset(d: &mut zmidi_ci_pe::ZlibMcoded7Decoder<'static>, input: &[u8], out: &mut [u8]) {
    d.reset();
    black_box(d.decode(input, out).ok());
}
