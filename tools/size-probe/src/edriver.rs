//! A stub `embassy-usb` driver: the probe measures the class and the `embassy-usb` core, not a
//! HAL. Each method reads or writes the volatile `EREG`, so the compiler cannot fold the code.

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use embassy_usb::driver::{
    Bus, ControlPipe, Direction, Driver, Endpoint, EndpointAddress, EndpointAllocError,
    EndpointError, EndpointIn, EndpointInfo, EndpointOut, EndpointType, Event, Unsupported,
};

#[no_mangle]
static mut EREG: [u32; 32] = [0; 32];

fn reg(i: usize) -> u32 {
    unsafe { read_volatile(addr_of!(EREG[i % 32])) }
}

fn set(i: usize, v: u32) {
    unsafe { write_volatile(addr_of_mut!(EREG[i % 32]), v) }
}

pub struct StubDriver;
pub struct StubOut(EndpointInfo);
pub struct StubIn(EndpointInfo);
pub struct StubBus;
pub struct StubPipe;

fn info(dir: Direction, ep_type: EndpointType, addr: Option<EndpointAddress>, mps: u16, interval_ms: u8)
    -> Result<EndpointInfo, EndpointAllocError> {
    let addr = addr.unwrap_or(EndpointAddress::from_parts(reg(1) as usize % 16, dir));
    set(1, u32::from(mps));
    Ok(EndpointInfo { addr, ep_type, max_packet_size: mps, interval_ms })
}

impl<'a> Driver<'a> for StubDriver {
    type EndpointOut = StubOut;
    type EndpointIn = StubIn;
    type ControlPipe = StubPipe;
    type Bus = StubBus;

    fn alloc_endpoint_out(&mut self, t: EndpointType, a: Option<EndpointAddress>, mps: u16, i: u8)
        -> Result<StubOut, EndpointAllocError> {
        info(Direction::Out, t, a, mps, i).map(StubOut)
    }

    fn alloc_endpoint_in(&mut self, t: EndpointType, a: Option<EndpointAddress>, mps: u16, i: u8)
        -> Result<StubIn, EndpointAllocError> {
        info(Direction::In, t, a, mps, i).map(StubIn)
    }

    fn start(self, _control_max_packet_size: u16) -> (StubBus, StubPipe) {
        (StubBus, StubPipe)
    }
}

impl Endpoint for StubOut {
    fn info(&self) -> &EndpointInfo {
        &self.0
    }
    async fn wait_enabled(&mut self) {}
}

impl EndpointOut for StubOut {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, EndpointError> {
        let n = (reg(4) as usize).min(buf.len());
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = reg(8 + i % 16) as u8;
        }
        if reg(5) == 0 { Ok(n) } else { Err(EndpointError::Disabled) }
    }
}

impl Endpoint for StubIn {
    fn info(&self) -> &EndpointInfo {
        &self.0
    }
    async fn wait_enabled(&mut self) {}
}

impl EndpointIn for StubIn {
    async fn write(&mut self, buf: &[u8]) -> Result<(), EndpointError> {
        for (i, b) in buf.iter().enumerate() {
            set(8 + i % 16, u32::from(*b));
        }
        if reg(5) == 0 { Ok(()) } else { Err(EndpointError::Disabled) }
    }
}

impl Bus for StubBus {
    async fn enable(&mut self) {
        set(2, 1);
    }
    async fn disable(&mut self) {
        set(2, 0);
    }
    async fn poll(&mut self) -> Event {
        match reg(0) & 3 {
            0 => Event::Reset,
            1 => Event::Suspend,
            2 => Event::Resume,
            _ => Event::PowerDetected,
        }
    }
    fn endpoint_set_enabled(&mut self, a: EndpointAddress, e: bool) {
        set(3, u32::from(u8::from(a)) | u32::from(e) << 8);
    }
    fn endpoint_set_stalled(&mut self, a: EndpointAddress, s: bool) {
        set(6, u32::from(u8::from(a)) | u32::from(s) << 8);
    }
    fn endpoint_is_stalled(&mut self, _a: EndpointAddress) -> bool {
        reg(6) != 0
    }
    async fn remote_wakeup(&mut self) -> Result<(), Unsupported> {
        Err(Unsupported)
    }
}

impl ControlPipe for StubPipe {
    fn max_packet_size(&self) -> usize {
        64
    }
    async fn setup(&mut self) -> [u8; 8] {
        let a = reg(10).to_le_bytes();
        let b = reg(11).to_le_bytes();
        [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
    }
    async fn data_out(&mut self, buf: &mut [u8], _first: bool, _last: bool) -> Result<usize, EndpointError> {
        let n = (reg(12) as usize).min(buf.len());
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = reg(13 + i % 4) as u8;
        }
        Ok(n)
    }
    async fn data_in(&mut self, data: &[u8], _first: bool, _last: bool) -> Result<(), EndpointError> {
        for (i, b) in data.iter().enumerate() {
            set(16 + i % 8, u32::from(*b));
        }
        Ok(())
    }
    async fn accept(&mut self) {
        set(7, 1);
    }
    async fn reject(&mut self) {
        set(7, 2);
    }
    async fn accept_set_address(&mut self, addr: u8) {
        set(7, u32::from(addr));
    }
}
