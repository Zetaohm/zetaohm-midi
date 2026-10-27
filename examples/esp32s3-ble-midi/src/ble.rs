//! The BLE part of the echo peripheral, with the `zmidi-ble-trouble` adapter. It has no ESP32
//! code, so the host tests of `crates/zmidi-ble-trouble` (`tests/example_ble.rs`) type-check it
//! with the workspace toolchain. The file that includes it defines the `log!` macro first.
//!
//! - GATT: the adapter adds the MIDI Service and the MIDI Data I/O Characteristic (BLE-MIDI
//!   v1.0 section 3) after the GAP service. A read gives no payload (section 5).
//! - Advertising: the 128-bit MIDI Service UUID; the name is in the scan response.
//! - After the connection, `request_link` asks for a connection interval of 7.5 ms to 15 ms
//!   (section 4) and for link-layer packets of 251 octets (found in T-011: with 27 octets the
//!   echo is slower than the input). `trouble-host` answers the MTU exchange (section 6).
//! - Each received channel voice or System Exclusive message goes back to the central with a
//!   notification. Other messages are counted only.
//! - The GATT task never waits for the echo queue: `trouble-host` drops a received PDU when
//!   the GATT queue of the connection is full (T-011). A full echo queue drops the UMP and
//!   counts it; the host receive errors are in the status line.

use core::sync::atomic::{AtomicU32, Ordering};

use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetDataLength};
use bt_hci::controller::ControllerCmdSync;
use embassy_futures::join::join;
use embassy_futures::select::select3;
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::channel::Channel;
use embassy_time::{Instant, Timer};
use trouble_host::prelude::*;
use zmidi_ble::{ReadError, Received, TimedUmp};
use zmidi_ble_trouble::{
    advertising_data, host_rx_errors, request_link, scan_response_data, Event, MidiService,
    Receiver, Sender,
};

/// The device name (GAP and scan response).
pub const NAME: &str = "zetaohm-midi";
/// The largest packet: the packet pool MTU of 255 bytes gives L2CAP SDUs of 251 bytes, so the
/// ATT_MTU is 251 or less and a packet 248 bytes or less.
const PACKET_MAX: usize = 248;
/// UMPs that wait to go back to the central.
const QUEUE: usize = 256;
/// The most UMPs from one packet: a message is 2 bytes or more (a timestamp byte and a System
/// Real-Time status, or 2 data bytes with running status).
const PACKET_UMPS: usize = PACKET_MAX / 2;
/// The most UMPs for one batch of the sender.
const BATCH: usize = 64;
/// The attributes: GAP (5), GATT (1) and the MIDI Service (4), with room.
const ATTRIBUTES: usize = 16;

type Pool = DefaultPacketPool;
type Server<'v> = AttributeServer<'v, NoopRawMutex, Pool, ATTRIBUTES, 1>;

/// A UMP of at most 4 words, its word count and its time in milliseconds.
type Echo = ([u32; 4], usize, u64);
static ECHO: Channel<CriticalSectionRawMutex, Echo, QUEUE> = Channel::new();

static RECEIVED: AtomicU32 = AtomicU32::new(0);
static ECHOED: AtomicU32 = AtomicU32::new(0);
static IGNORED: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static ERRORS: AtomicU32 = AtomicU32::new(0);
static TOO_LONG: AtomicU32 = AtomicU32::new(0);
static RETRIES: AtomicU32 = AtomicU32::new(0);

/// A controller that can also set the data length (the other commands are in `Controller`).
pub trait MidiController:
    Controller + ControllerCmdSync<LeReadLocalSupportedFeatures> + ControllerCmdSync<LeSetDataLength>
{
}

impl<C> MidiController for C where
    C: Controller
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeSetDataLength>
{
}

/// Runs the BLE host and the echo peripheral.
pub async fn run<C: MidiController>(controller: C) -> ! {
    // A static random address: the two most significant bits are 1. A new GATT layout needs a new
    // address (T-016, 2026-10-25): macOS keeps the GATT handles of a known address, and the GATT
    // service of trouble-host 0.7 has no Service Changed characteristic that tells it to look again.
    // With the address of the trouble-host 0.5.1 build (last byte 0xC5) a read gave the old value
    // and the notify request failed with "The handle is invalid".
    let address = Address::random([0x4D, 0x49, 0x44, 0x49, 0x5A, 0xC6]);
    let mut resources: HostResources<C, Pool, 1, 2> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .build();
    let mut peripheral = stack.peripheral();
    let mut runner = stack.runner();

    let mut table: AttributeTable<'_, NoopRawMutex, ATTRIBUTES> = AttributeTable::new();
    GapConfig::Peripheral(PeripheralConfig {
        name: NAME,
        appearance: &appearance::UNKNOWN,
    })
    .build(&mut table)
    .unwrap();
    let midi = MidiService::add(&mut table);
    let server: Server<'_> = AttributeServer::new(table);

    // 21 and 14 bytes: the buffers of 31 bytes are large enough (tests of the adapter).
    let mut adv = [0u8; 31];
    let adv_len = advertising_data(&mut adv).unwrap();
    let mut scan = [0u8; 31];
    let scan_len = scan_response_data(NAME, &mut scan).unwrap();
    log!("esp32s3-ble-midi: advertising \"{NAME}\"");

    let ble = async {
        loop {
            if let Err(e) = runner.run().await {
                log!("BLE host: {e:?}");
            }
        }
    };
    let app = async {
        let mut rx = Receiver::<PACKET_MAX>::new(midi);
        let mut tx = Sender::<PACKET_MAX>::new(midi);
        loop {
            match advertise(&mut peripheral, &server, &adv[..adv_len], &scan[..scan_len]).await {
                Ok(conn) => {
                    log!("connected");
                    let link = request_link(&stack, conn.raw()).await;
                    if let Err(e) = link.interval {
                        log!("connection interval request: {e:?}");
                    }
                    if let Err(e) = link.data_length {
                        log!("data length request: {e:?}");
                    }
                    while ECHO.try_receive().is_ok() {}
                    rx.reset();
                    tx.reset();
                    select3(
                        gatt_events(&stack, &conn, &mut rx),
                        send(&conn, &mut tx),
                        status(&stack),
                    )
                    .await;
                    log_counters("disconnected", &stack);
                }
                Err(e) => log!("advertising: {e:?}"),
            }
        }
    };
    join(ble, app).await.0
}

/// Prints the counters, with the receive errors of the BLE host (a PDU dropped there is lost).
fn log_counters<C: Controller>(label: &str, stack: &Stack<'_, C, Pool>) {
    log!(
        "{label}; received {}, echoed {}, ignored {}, dropped {}, errors {}, too long {}, notify retries {}, host rx errors {}",
        RECEIVED.load(Ordering::Relaxed),
        ECHOED.load(Ordering::Relaxed),
        IGNORED.load(Ordering::Relaxed),
        DROPPED.load(Ordering::Relaxed),
        ERRORS.load(Ordering::Relaxed),
        TOO_LONG.load(Ordering::Relaxed),
        RETRIES.load(Ordering::Relaxed),
        host_rx_errors(stack),
    );
}

/// Prints the counters every 2 s while they change.
async fn status<C: Controller>(stack: &Stack<'_, C, Pool>) {
    let mut last = (u32::MAX, u32::MAX);
    loop {
        Timer::after_secs(2).await;
        let now = (
            RECEIVED.load(Ordering::Relaxed),
            ECHOED.load(Ordering::Relaxed),
        );
        if now != last {
            log_counters("status", stack);
            last = now;
        }
    }
}

/// Advertises the MIDI Service and waits for a central.
async fn advertise<'d, 's, C: Controller>(
    peripheral: &mut Peripheral<'d, C, Pool>,
    server: &'s Server<'_>,
    adv_data: &[u8],
    scan_data: &[u8],
) -> Result<GattConnection<'d, 's, Pool>, BleHostError<C::Error>> {
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data,
                scan_data,
            },
        )
        .await?;
    Ok(advertiser.accept().await?.with_attribute_server(server)?)
}

/// The GATT events of one connection. The adapter answers the MIDI reads and writes; this task
/// queues the echo with no wait and answers the other events at once.
async fn gatt_events<C: Controller>(
    stack: &Stack<'_, C, Pool>,
    conn: &GattConnection<'_, '_, Pool>,
    rx: &mut Receiver<PACKET_MAX>,
) {
    loop {
        let mut echo = heapless::Vec::<Echo, PACKET_UMPS>::new();
        match rx.next(conn, |r| collect_echo(r, &mut echo)).await {
            Event::Packet { .. } => {
                for u in echo {
                    if ECHO.try_send(u).is_err() {
                        DROPPED.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Event::PacketTooLong { len } => {
                TOO_LONG.fetch_add(1, Ordering::Relaxed);
                log!("packet of {len} bytes dropped");
            }
            Event::ReplyFailed(e) => log!("GATT reply: {e:?}"),
            Event::Disconnected { reason } => {
                log!("disconnect reason {reason:?}");
                return;
            }
            Event::Other(GattConnectionEvent::ConnectionParamsUpdated {
                conn_interval,
                peripheral_latency,
                ..
            }) => log!(
                "connection interval {} us, latency {peripheral_latency}",
                conn_interval.as_micros()
            ),
            Event::Other(GattConnectionEvent::DataLengthUpdated {
                max_tx_octets,
                max_rx_octets,
                ..
            }) => log!("data length: tx {max_tx_octets}, rx {max_rx_octets} octets"),
            Event::Other(GattConnectionEvent::PhyUpdated { tx_phy, rx_phy }) => {
                log!("PHY: tx {tx_phy:?}, rx {rx_phy:?}")
            }
            Event::Other(GattConnectionEvent::RequestConnectionParams(req)) => {
                if let Err(e) = req.accept(None, stack).await {
                    log!("connection parameters of the central: {e:?}");
                }
            }
            // The GAP and GATT services: the attribute server answers.
            Event::Other(GattConnectionEvent::Gatt { event }) => match event.accept() {
                Ok(reply) => reply.send().await,
                Err(e) => log!("GATT reply: {e:?}"),
            },
            Event::Other(_) => {}
        }
    }
}

/// The sink of the receiver: collects the channel voice and System Exclusive UMPs of one packet
/// for the echo.
fn collect_echo(r: Result<Received, ReadError>, echo: &mut heapless::Vec<Echo, PACKET_UMPS>) {
    match r {
        Ok(r) => {
            RECEIVED.fetch_add(1, Ordering::Relaxed);
            let mut words = [0u32; 4];
            let n = r.ump.write(&mut words).unwrap_or(0);
            // Message Type 0x2 and 0x4 (channel voice), 0x3 (System Exclusive 7-bit).
            if n > 0 && matches!(words[0] >> 28, 0x2..=0x4) {
                if echo.push((words, n, Instant::now().as_millis())).is_err() {
                    DROPPED.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                IGNORED.fetch_add(1, Ordering::Relaxed);
            }
        }
        Err(_) => {
            ERRORS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Sends the queued UMPs as notifications, in packets of the ATT_MTU of the connection.
async fn send(conn: &GattConnection<'_, '_, Pool>, tx: &mut Sender<PACKET_MAX>) {
    let mut batch = [([0u32; 4], 0usize, 0u64); BATCH];
    let mut last_mtu = 0;
    loop {
        batch[0] = ECHO.receive().await;
        let mut n = 1;
        // Check the room first: a UMP taken from the queue must go into the batch.
        while n < BATCH {
            let Ok(u) = ECHO.try_receive() else { break };
            batch[n] = u;
            n += 1;
        }
        let mtu = Sender::<PACKET_MAX>::att_mtu_for(conn.raw().att_mtu());
        if mtu != last_mtu {
            log!("ATT_MTU {mtu}: packets of {} bytes", mtu - 3);
            last_mtu = mtu;
        }
        let mut umps = batch[..n]
            .iter()
            .map(|(w, len, t)| TimedUmp {
                millis: *t,
                words: &w[..*len],
            })
            .peekable();
        match tx.send(conn, &mut umps).await {
            Ok(report) => {
                RETRIES.fetch_add(report.retries, Ordering::Relaxed);
            }
            Err(e) => {
                log!("notification: {e}");
                return;
            }
        }
        ECHOED.fetch_add((n - umps.count()) as u32, Ordering::Relaxed);
    }
}
