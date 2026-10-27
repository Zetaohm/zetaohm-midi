//! `trouble-host` 0.7 adapter for the zmidi-ble BLE-MIDI v1.0 codec (DEC-016).
//!
//! The adapter connects the codec of `zmidi-ble` to the GATT server of `trouble-host`. It adds no
//! protocol logic (DEC-004): the packets go through a [`PacketReader`] and a [`PacketWriter`],
//! and the GATT data comes from [`zmidi_ble::gatt`].
//!
//! - [`MidiService::add`] adds the MIDI Service and the MIDI Data I/O Characteristic (BLE-MIDI
//!   v1.0 section 3: read, write, write without response, notify) to an `AttributeTable`. The
//!   value in the table is always empty, so a read gives a packet with no payload (section 5).
//! - [`advertising_data`] and [`scan_response_data`] give the advertising data with the
//!   128-bit MIDI Service UUID and the name.
//! - [`request_link`] asks for a connection interval of 7.5 ms to 15 ms (section 4,
//!   [`CONNECTION_PARAMS`]) and for link-layer packets of 251 octets ([`DATA_LENGTH`]).
//!   `trouble-host` answers the MTU exchange (section 6); the [`Sender`] uses the ATT_MTU of
//!   the connection.
//! - [`Receiver::next`] runs the GATT events of one connection. It copies a written packet,
//!   sends the GATT reply first, and then gives the UMPs of the packet to a synchronous sink.
//!   It never waits for the application: `trouble-host` drops a received PDU when the GATT
//!   queue of the connection is full (found in T-011 with 0.5.1; in 0.7 the host counts it in
//!   `rx_errors`, [`host_rx_errors`]). The sink must not block either.
//! - [`Sender::send`] writes the UMPs into packets of at most ATT_MTU - 3 bytes and sends each
//!   packet as a notification. When the packet pool has no free packet, it waits
//!   [`RETRY_DELAY`] and tries again.
//!
//! One [`Receiver`] and one [`Sender`] serve one connection; run them in two futures (for
//! example with `select`) so that a slow notification never stops the GATT events.

#![no_std]

use core::iter::Peekable;

use bt_hci::cmd::le::{LeConnUpdate, LeReadLocalSupportedFeatures, LeSetDataLength};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use bt_hci::param::Status;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_time::{Duration, Timer};
use trouble_host::prelude::{
    AdStructure, AttributeTable, BleHostError, Characteristic, Connection, Controller,
    GattConnection, GattConnectionEvent, GattEvent, PacketPool, RequestedConnParams, Service,
    Stack, BR_EDR_NOT_SUPPORTED, LE_GENERAL_DISCOVERABLE,
};
pub use zmidi_ble;
use zmidi_ble::{gatt, PacketReader, PacketWriter, ReadError, Received, TimedUmp, WriteError};

/// The link-layer packet length that [`request_link`] asks for, in octets: the largest LE data
/// length, so that a notification of 244 bytes is one link-layer packet (found in T-011: with
/// 27 octets the echo was slower than the input).
pub const DATA_LENGTH: u16 = 251;

/// The transmit time of a packet of [`DATA_LENGTH`] octets on the LE 1M PHY, in microseconds:
/// (251 + 14) x 8.
pub const DATA_LENGTH_TIME_US: u16 = 2120;

/// The connection parameters that [`request_link`] asks for: an interval of 7.5 ms to 15 ms
/// (BLE-MIDI v1.0 section 4: 15 ms or less; 7.5 ms is the smallest LE interval), no peripheral
/// latency and a supervision timeout of 2 s.
pub const CONNECTION_PARAMS: RequestedConnParams = RequestedConnParams {
    min_connection_interval: Duration::from_micros(7_500),
    max_connection_interval: Duration::from_millis(gatt::MAX_CONNECTION_INTERVAL_MS as u64),
    max_latency: 0,
    min_event_length: Duration::from_ticks(0),
    max_event_length: Duration::from_ticks(0),
    supervision_timeout: Duration::from_secs(2),
};

/// The wait of the [`Sender`] before it tries a notification again when the packet pool of
/// `trouble-host` has no free packet.
pub const RETRY_DELAY: Duration = Duration::from_millis(2);

/// The longest attribute value of the attribute protocol, and so the longest BLE-MIDI packet
/// (Bluetooth Core Vol 3 Part F 3.2.9, from memory; the Bluetooth Core document is not in
/// spec_pdfs/).
pub const MAX_PACKET: usize = 512;

/// Error of the adapter.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// An error of `trouble-host`, for example [`trouble_host::Error::Disconnected`].
    #[error("BLE host: {0:?}")]
    Host(trouble_host::Error),
    /// The packet writer refused the ATT_MTU or the buffer.
    #[error("packet writer: {0}")]
    Write(WriteError),
}

impl From<trouble_host::Error> for Error {
    fn from(e: trouble_host::Error) -> Self {
        Error::Host(e)
    }
}

impl From<WriteError> for Error {
    fn from(e: WriteError) -> Self {
        Error::Write(e)
    }
}

/// The MIDI Service and its MIDI Data I/O Characteristic in an attribute table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiService {
    /// The handle of the service declaration.
    pub service: u16,
    /// The MIDI Data I/O Characteristic. The value has no storage (`[u8; 0]`); the packets go
    /// through `notify_raw`.
    characteristic: Characteristic<[u8; 0]>,
}

impl MidiService {
    /// Adds the MIDI Service with the MIDI Data I/O Characteristic (BLE-MIDI v1.0 section 3,
    /// properties [`gatt::CHARACTERISTIC_PROPERTIES`]) to `table`.
    ///
    /// The value has no storage: it is always empty, so each read gives a packet with no
    /// payload (section 5). [`Receiver::next`] reads the written packets from the GATT events,
    /// and [`Sender::send`] notifies without a store into the table.
    pub fn add<M: RawMutex, const MAX: usize>(table: &mut AttributeTable<'_, M, MAX>) -> Self {
        let mut service = table.add_service(Service::new(gatt::SERVICE_UUID));
        let store: &mut [u8] = &mut [];
        let characteristic = service
            .add_characteristic::<[u8; 0], _, _>(
                gatt::CHARACTERISTIC_UUID,
                gatt::CHARACTERISTIC_PROPERTIES,
                [],
                store,
            )
            .build();
        Self {
            service: service.build(),
            characteristic,
        }
    }

    /// The handle of the value of the MIDI Data I/O Characteristic.
    pub fn value_handle(&self) -> u16 {
        self.characteristic.handle
    }

    /// The handle of the Client Characteristic Configuration descriptor of the MIDI Data I/O
    /// Characteristic (present, because the characteristic has the notify property).
    pub fn cccd_handle(&self) -> Option<u16> {
        self.characteristic.cccd_handle
    }

    /// The characteristic properties in the declaration ([`gatt::CHARACTERISTIC_PROPERTIES`]).
    pub fn properties(&self) -> u8 {
        self.characteristic.props.to_raw()
    }

    /// True when the central asked for notifications of the characteristic on `conn`.
    pub fn is_subscribed<P: PacketPool>(&self, conn: &GattConnection<'_, '_, P>) -> bool {
        self.characteristic.should_notify(conn)
    }
}

/// Writes the advertising data into `buf` and gives its length: the flags (LE General
/// Discoverable, BR/EDR not supported) and the 128-bit MIDI Service UUID (BLE-MIDI v1.0
/// section 3). 21 bytes; a legacy advertisement has room for 31.
pub fn advertising_data(buf: &mut [u8]) -> Result<usize, Error> {
    Ok(AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids128(&[gatt::SERVICE_UUID_LE]),
        ],
        buf,
    )
    .map_err(trouble_host::Error::from)?)
}

/// Writes the scan response data into `buf` and gives its length: the complete local name
/// (29 bytes or less in a legacy scan response).
pub fn scan_response_data(name: &str, buf: &mut [u8]) -> Result<usize, Error> {
    Ok(
        AdStructure::encode_slice(&[AdStructure::CompleteLocalName(name.as_bytes())], buf)
            .map_err(trouble_host::Error::from)?,
    )
}

/// The results of [`request_link`]. A refusal does not stop the connection; BLE-MIDI works
/// with the parameters of the central, only more slowly.
#[derive(Debug)]
pub struct LinkRequest<E> {
    /// The request of [`CONNECTION_PARAMS`].
    pub interval: Result<(), BleHostError<E>>,
    /// The request of [`DATA_LENGTH`].
    pub data_length: Result<(), BleHostError<E>>,
}

/// Asks for the connection parameters of BLE-MIDI v1.0 section 4 ([`CONNECTION_PARAMS`]) and
/// for link-layer packets of [`DATA_LENGTH`] octets on `conn`. Call it once after the
/// connection, before the [`Receiver`] and [`Sender`] run.
pub async fn request_link<C, P>(
    stack: &Stack<'_, C, P>,
    conn: &Connection<'_, P>,
) -> LinkRequest<C::Error>
where
    C: Controller
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdAsync<LeConnUpdate>
        + ControllerCmdSync<LeSetDataLength>,
    P: PacketPool,
{
    let interval = conn
        .update_connection_params(stack, &CONNECTION_PARAMS)
        .await;
    let data_length = conn
        .update_data_length(stack, DATA_LENGTH, DATA_LENGTH_TIME_US)
        .await;
    LinkRequest {
        interval,
        data_length,
    }
}

/// The receive errors of the BLE host: a PDU that `trouble-host` could not process, for
/// example because the GATT queue of the connection was full, is lost and counted here.
pub fn host_rx_errors<C: Controller, P: PacketPool>(stack: &Stack<'_, C, P>) -> u32 {
    stack.metrics(|m| m.rx_errors)
}

/// An event of [`Receiver::next`].
#[derive(Debug)]
pub enum Event<'stack, 'server, P: PacketPool> {
    /// The central wrote a packet of `len` bytes; the sink got its UMPs and errors.
    Packet {
        /// The packet length.
        len: usize,
    },
    /// The central wrote a packet longer than the buffer of the [`Receiver`]; the packet is
    /// dropped.
    PacketTooLong {
        /// The packet length.
        len: usize,
    },
    /// The GATT reply could not be made (for example no free packet in the pool). For a write
    /// of a packet, the sink got its UMPs before.
    ReplyFailed(trouble_host::Error),
    /// The connection ended. Make a new [`Receiver`] state with [`Receiver::reset`] for the
    /// next connection.
    Disconnected {
        /// The reason of the disconnection.
        reason: Status,
    },
    /// Another event of the connection: the connection parameters, the data length, the PHY,
    /// a request of the central, or a GATT event of another attribute. Handle it at once (a GATT
    /// event: `event.accept()` and send the reply), then call [`Receiver::next`] again.
    Other(GattConnectionEvent<'stack, 'server, P>),
}

/// The receive side of one connection: the packet reader and a buffer of `N` bytes for one
/// packet. `N` is the longest packet that the receiver takes: ATT_MTU - 3 of the largest
/// ATT_MTU of the firmware, at most [`MAX_PACKET`].
#[derive(Debug, Clone)]
pub struct Receiver<const N: usize> {
    service: MidiService,
    reader: PacketReader,
    buf: [u8; N],
}

impl<const N: usize> Receiver<N> {
    /// A receiver for `service` with the default reader (Group 0, MIDI 1.0 Protocol).
    pub fn new(service: MidiService) -> Self {
        Self::with_reader(service, PacketReader::default())
    }

    /// A receiver for `service` with `reader` (another Group or Protocol).
    pub fn with_reader(service: MidiService, reader: PacketReader) -> Self {
        Self {
            service,
            reader,
            buf: [0; N],
        }
    }

    /// The packet reader, for example to see an open SysEx message.
    pub fn reader(&self) -> &PacketReader {
        &self.reader
    }

    /// Forgets an open SysEx message, for a new connection.
    pub fn reset(&mut self) {
        self.reader.reset();
    }

    /// Copies a written packet into the buffer. `Err(len)` when it is longer than `N`.
    fn take(&mut self, data: &[u8]) -> Result<usize, usize> {
        let dst = self.buf.get_mut(..data.len()).ok_or(data.len())?;
        dst.copy_from_slice(data);
        Ok(data.len())
    }

    /// Gives the UMPs of the first `len` bytes of the buffer to `sink`.
    fn decode(&mut self, len: usize, sink: impl FnMut(Result<Received, ReadError>)) {
        self.reader.read(&self.buf[..len], sink);
    }

    /// The end of a write of the value: the packet arrived, so it is decoded also when the GATT
    /// reply failed (`replied`), and the event tells which (T-019). `taken` is the length of the
    /// copied packet, or the length of a packet longer than the buffer.
    fn finish<'stack, 'server, P: PacketPool>(
        &mut self,
        taken: Result<usize, usize>,
        replied: Result<(), trouble_host::Error>,
        sink: impl FnMut(Result<Received, ReadError>),
    ) -> Event<'stack, 'server, P> {
        let event = match taken {
            Ok(len) => {
                self.decode(len, sink);
                Event::Packet { len }
            }
            Err(len) => Event::PacketTooLong { len },
        };
        match replied {
            Ok(()) => event,
            Err(err) => Event::ReplyFailed(err),
        }
    }

    /// Waits for the next event of `conn` and handles the GATT events of the MIDI Data I/O
    /// Characteristic:
    /// - a write: the packet is copied, the reply is sent, then each UMP (with its 13-bit
    ///   timestamp) or read error of the packet goes to `sink`, and the result is
    ///   [`Event::Packet`];
    /// - a read: the reply has no payload (section 5); the receiver waits for the next event;
    /// - a write of its Client Characteristic Configuration descriptor: the attribute server
    ///   stores it; the receiver waits for the next event.
    ///
    /// `sink` runs in this task and must not block: put the UMPs in a queue with `try_send`
    /// and count what does not fit. Other events go back as [`Event::Other`].
    pub async fn next<'stack, 'server, P: PacketPool>(
        &mut self,
        conn: &GattConnection<'stack, 'server, P>,
        sink: impl FnMut(Result<Received, ReadError>),
    ) -> Event<'stack, 'server, P> {
        let value = self.service.value_handle();
        let cccd = self.service.cccd_handle();
        loop {
            let event = match conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    return Event::Disconnected { reason }
                }
                GattConnectionEvent::Gatt { event } => event,
                other => return Event::Other(other),
            };
            match event {
                GattEvent::Write(e) if e.handle() == value => {
                    let taken = e.with_data(|offset, data| {
                        if offset == 0 {
                            self.take(data)
                        } else {
                            Err(offset + data.len())
                        }
                    });
                    // The value has no storage: the reply does not write it into the table.
                    // Reply first, so the received PDU and the reply free their pool packets.
                    // A write without response has no reply PDU.
                    let replied = match e.accept_unprocessed() {
                        Ok(reply) => {
                            reply.send().await;
                            Ok(())
                        }
                        Err(err) => Err(err),
                    };
                    return self.finish(taken, replied, sink);
                }
                GattEvent::Read(e) if e.handle() == value => {
                    // The stored value is empty: the attribute server answers with no payload.
                    match e.accept() {
                        Ok(reply) => reply.send().await,
                        Err(err) => return Event::ReplyFailed(err),
                    }
                }
                GattEvent::Write(e) if Some(e.handle()) == cccd => match e.accept() {
                    Ok(reply) => reply.send().await,
                    Err(err) => return Event::ReplyFailed(err),
                },
                event => {
                    return Event::Other(GattConnectionEvent::Gatt { event });
                }
            }
        }
    }
}

/// The result of one [`Sender::send`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SendReport {
    /// The notifications sent.
    pub packets: u32,
    /// The waits for a free packet of the pool.
    pub retries: u32,
}

/// The send side of one connection: the packet writer and a buffer of `N` bytes for one
/// packet. The packets are at most `N` bytes; `N` is 20 to [`MAX_PACKET`].
#[derive(Debug, Clone)]
pub struct Sender<const N: usize> {
    service: MidiService,
    writer: PacketWriter,
    buf: [u8; N],
}

impl<const N: usize> Sender<N> {
    const VALID: () = assert!(
        N >= (zmidi_ble::MIN_ATT_MTU as usize - 3) && N <= MAX_PACKET,
        "the packet buffer of a Sender is 20 to 512 bytes"
    );

    /// A sender for `service` with the default writer (running status on).
    pub fn new(service: MidiService) -> Self {
        Self::with_writer(service, PacketWriter::default())
    }

    /// A sender for `service` with `writer` (for example running status off).
    pub fn with_writer(service: MidiService, writer: PacketWriter) -> Self {
        let () = Self::VALID;
        Self {
            service,
            writer,
            buf: [0; N],
        }
    }

    /// The packet writer.
    pub fn writer(&self) -> &PacketWriter {
        &self.writer
    }

    /// True when the writer holds bytes of a UMP for the next packet.
    pub fn has_held(&self) -> bool {
        self.writer.has_held()
    }

    /// Forgets the held bytes and an open SysEx message, for a new connection.
    pub fn reset(&mut self) {
        self.writer.reset();
    }

    /// The ATT_MTU that the writer uses on a connection with `att_mtu`: at most `N` + 3.
    pub fn att_mtu_for(att_mtu: u16) -> u16 {
        att_mtu.min(N as u16 + 3)
    }

    /// Writes the next packet for a connection with `att_mtu` from the held bytes and from
    /// `umps`; an empty slice when there is nothing to send.
    fn next_packet<'w, I>(&mut self, att_mtu: u16, umps: &mut Peekable<I>) -> Result<&[u8], Error>
    where
        I: Iterator<Item = TimedUmp<'w>>,
    {
        self.writer.set_att_mtu(Self::att_mtu_for(att_mtu))?;
        let len = self.writer.write_packet(umps, &mut self.buf)?;
        Ok(&self.buf[..len])
    }

    /// Sends the UMPs of `umps` as notifications on `conn`, until `umps` is empty and the
    /// writer holds nothing. Each packet has at most ATT_MTU - 3 bytes of the ATT_MTU of the
    /// connection (and at most `N`). When the central did not ask for notifications,
    /// `trouble-host` drops the packets with no error.
    ///
    /// On an error (for example [`trouble_host::Error::Disconnected`]) the UMPs not yet taken
    /// stay in `umps`.
    pub async fn send<'w, I, P>(
        &mut self,
        conn: &GattConnection<'_, '_, P>,
        umps: &mut Peekable<I>,
    ) -> Result<SendReport, Error>
    where
        I: Iterator<Item = TimedUmp<'w>>,
        P: PacketPool,
    {
        let characteristic = self.service.characteristic;
        self.send_with(
            || conn.raw().att_mtu(),
            umps,
            async |packet: &[u8]| characteristic.notify_raw(conn, packet, false).await,
        )
        .await
    }

    /// The loop of [`Sender::send`] with the ATT_MTU and the notification as arguments, so that
    /// a host test can make the notification fail (T-019): [`trouble_host::Error::OutOfMemory`]
    /// (no free packet in the pool) waits [`RETRY_DELAY`] and sends the same packet again;
    /// another error ends the call.
    async fn send_with<'w, I>(
        &mut self,
        att_mtu: impl Fn() -> u16,
        umps: &mut Peekable<I>,
        mut notify: impl AsyncFnMut(&[u8]) -> Result<(), trouble_host::Error>,
    ) -> Result<SendReport, Error>
    where
        I: Iterator<Item = TimedUmp<'w>>,
    {
        let mut report = SendReport::default();
        loop {
            let packet = self.next_packet(att_mtu(), umps)?;
            if packet.is_empty() {
                return Ok(report);
            }
            loop {
                match notify(packet).await {
                    Ok(()) => break,
                    Err(trouble_host::Error::OutOfMemory) => {
                        report.retries += 1;
                        Timer::after(RETRY_DELAY).await;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            report.packets += 1;
        }
    }
}

#[cfg(test)]
mod tests;
