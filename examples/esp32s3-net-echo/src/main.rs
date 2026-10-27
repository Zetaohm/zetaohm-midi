//! Network MIDI 2.0 echo Host on an ESP32-S3 (Wi-Fi station), with `zmidi-net` (DEC-012).
//!
//! - Wi-Fi with DHCP (`esp-radio`, `embassy-net`). Set the network at build time:
//!   `SSID=... PASSWORD=... cargo run --release`.
//! - A `zmidi-net` Host on UDP port 5673: it accepts each Client and sends each received UMP
//!   back, as `examples/net-echo` `host` does.
//! - DNS-SD: the small responder of [`mdns`] announces the Host as "zetaohm-midi esp32s3"
//!   (`_midi2._udp`), with the hostname "zetaohm-esp32s3.local.". `examples/net-echo`
//!   `client --browse` finds it.

#![no_std]
#![no_main]

mod mdns;

use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{IpAddress, IpEndpoint, Runner, Stack, StackResources};
use embassy_time::{Duration, Instant, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, ram, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_radio::wifi::{ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent, WifiStaState};
use esp_radio::Controller;
use zmidi_net::dns::Hostname;
use zmidi_net::{EventKind, Host, HostConfig, Identity, NoCredentials};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

const SSID: &str = match option_env!("SSID") {
    Some(s) => s,
    None => "",
};
const PASSWORD: &str = match option_env!("PASSWORD") {
    Some(s) => s,
    None => "",
};
const MIDI_PORT: u16 = 5673;
const INSTANCE: &str = "zetaohm-midi esp32s3";
const HOSTNAME: &str = "zetaohm-esp32s3.local.";
const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
/// UMPs that wait to go back to the peer after one received payload: a 1472-byte UDP payload has
/// at most 368 words, so at most 368 UMPs.
const ECHO: usize = 368;

/// Two Client sessions; the queue capacities stay the default.
type EchoHost = Host<2>;

fn to_socket(e: IpEndpoint) -> Option<SocketAddr> {
    match e.addr {
        IpAddress::Ipv4(a) => Some(SocketAddr::new(IpAddr::V4(a), e.port)),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

fn to_endpoint(a: SocketAddr) -> Option<IpEndpoint> {
    match a {
        SocketAddr::V4(v4) => Some(IpEndpoint::new(IpAddress::Ipv4(*v4.ip()), v4.port())),
        SocketAddr::V6(_) => None,
    }
}

fn now_ms() -> u64 {
    Instant::now().as_millis()
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    static RADIO: StaticCell<Controller<'static>> = StaticCell::new();
    let radio = &*RADIO.init(esp_radio::init().unwrap());
    let (controller, interfaces) =
        esp_radio::wifi::new(radio, peripherals.WIFI, Default::default()).unwrap();

    let rng = Rng::new();
    let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());
    static RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        interfaces.sta,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(wifi_task(controller)).ok();
    spawner.spawn(net_task(runner)).ok();

    let ip = loop {
        if let Some(c) = stack.config_v4() {
            break c.address.address();
        }
        Timer::after(Duration::from_millis(500)).await;
    };
    println!("IP {ip}; Network MIDI 2.0 Host on UDP port {MIDI_PORT}");

    // The Host is built in its static cell, not on the stack (OPT-003, OPT-013).
    static HOST: StaticCell<Option<EchoHost>> = StaticCell::new();
    let identity = Identity::new(INSTANCE, "esp32s3-1").unwrap();
    let host = EchoHost::init(HOST.init(None), HostConfig::new(identity)).unwrap();
    let records = host.host_records(INSTANCE, MIDI_PORT, HOSTNAME).unwrap();
    let hostname = Hostname::new(HOSTNAME).unwrap();
    static RESPONDER: StaticCell<mdns::Responder> = StaticCell::new();
    let responder = RESPONDER.init(mdns::Responder::new(&records, &hostname, ip.octets()).unwrap());
    spawner.spawn(mdns_task(stack, responder)).ok();

    run_host(stack, host).await
}

/// The `zmidi-net` main loop on one UDP socket: receive, poll, write (endpoint module docs).
async fn run_host(stack: Stack<'static>, host: &mut EchoHost) -> ! {
    let mut rx_meta = [PacketMetadata::EMPTY; 8];
    let mut tx_meta = [PacketMetadata::EMPTY; 8];
    let mut rx_buf = [0u8; 4096];
    let mut tx_buf = [0u8; 4096];
    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    socket.bind(MIDI_PORT).unwrap();
    let mut buf = [0u8; 1500];
    let mut out = [0u8; 1500];
    let mut echo = [([0u32; 4], 0usize, None::<SocketAddr>); ECHO];
    let mut dropped = 0u32;
    loop {
        let wait = host
            .next_deadline()
            .map_or(100, |d| d.saturating_sub(now_ms()))
            .clamp(1, 100);
        let got = select(socket.recv_from(&mut buf), Timer::after(Duration::from_millis(wait))).await;
        if let Either::First(Ok((n, meta))) = got {
            if let Some(from) = to_socket(meta.endpoint) {
                let mut queued = 0;
                let mut creds = NoCredentials;
                let mut r = host.receive(now_ms(), from, &buf[..n], &mut creds);
                for e in r.by_ref() {
                    match e.kind {
                        EventKind::Data(d) => {
                            for m in d.messages() {
                                if let Some(slot) = echo.get_mut(queued) {
                                    let w = m.words();
                                    slot.0[..w.len()].copy_from_slice(w);
                                    slot.1 = w.len();
                                    slot.2 = Some(e.peer);
                                    queued += 1;
                                } else {
                                    dropped += 1;
                                }
                            }
                        }
                        kind => println!("{}: {kind:?}", e.peer),
                    }
                }
                while let Some((to, len)) = r.write_next(&mut out) {
                    if let Some(ep) = to_endpoint(to) {
                        let _ = socket.send_to(&out[..len], ep).await;
                    }
                }
                drop(r);
                for (words, len, peer) in echo[..queued].iter() {
                    if let Some(peer) = peer {
                        if host.send_ump(*peer, &words[..*len]).is_err() {
                            // The send queue (256 words) is full: send it, then try again.
                            let now = now_ms();
                            while let Some((to, len)) = host.write_pending(now, &mut out) {
                                if let Some(ep) = to_endpoint(to) {
                                    let _ = socket.send_to(&out[..len], ep).await;
                                }
                            }
                            if host.send_ump(*peer, &words[..*len]).is_err() {
                                dropped += 1;
                            }
                        }
                    }
                }
                if dropped > 0 {
                    println!("echo: {dropped} UMPs dropped in total");
                }
            }
        }
        let now = now_ms();
        for e in host.poll(now) {
            println!("{}: {:?}", e.peer, e.kind);
        }
        while let Some((to, len)) = host.write_pending(now, &mut out) {
            if let Some(ep) = to_endpoint(to) {
                let _ = socket.send_to(&out[..len], ep).await;
            }
        }
    }
}

/// DNS-SD: announce at start-up (twice, 1 s apart, RFC 6762 8.3), then answer queries.
#[embassy_executor::task]
async fn mdns_task(stack: Stack<'static>, responder: &'static mdns::Responder) {
    if stack.join_multicast_group(MDNS_GROUP).is_err() {
        println!("mDNS: no multicast group");
        return;
    }
    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buf = [0u8; 1536];
    let mut tx_buf = [0u8; 2048];
    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    socket.bind(mdns::PORT).unwrap();
    let group = IpEndpoint::new(IpAddress::Ipv4(MDNS_GROUP), mdns::PORT);
    let mut out = [0u8; mdns::MAX_REPLY];
    for _ in 0..2 {
        if let Some(n) = responder.announcement(&mut out) {
            let _ = socket.send_to(&out[..n], group).await;
        }
        Timer::after(Duration::from_secs(1)).await;
    }
    println!("mDNS: \"{INSTANCE}\" _midi2._udp port {MIDI_PORT} announced");
    let mut buf = [0u8; 1536];
    loop {
        let Ok((n, meta)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        if let Some((len, unicast)) = responder.answer(&buf[..n], meta.endpoint.port, &mut out) {
            let to = if unicast { meta.endpoint } else { group };
            let _ = socket.send_to(&out[..len], to).await;
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn wifi_task(mut controller: WifiController<'static>) {
    if SSID.is_empty() {
        println!("Wi-Fi: no SSID; build with SSID=... PASSWORD=...");
        return;
    }
    loop {
        if esp_radio::wifi::sta_state() == WifiStaState::Connected {
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await;
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(SSID.into())
                    .with_password(PASSWORD.into()),
            );
            controller.set_config(&config).unwrap();
            controller.start_async().await.unwrap();
        }
        match controller.connect_async().await {
            Ok(()) => println!("Wi-Fi: connected to {SSID}"),
            Err(e) => {
                println!("Wi-Fi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await;
            }
        }
    }
}
