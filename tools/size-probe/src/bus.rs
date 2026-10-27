//! A stub `usb-device` bus: the probe measures the class and the `usb-device` core, not a HAL.
//! Each method reads or writes the volatile `REG`, so the compiler cannot fold the USB code away.

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use usb_device::bus::{PollResult, UsbBus};
use usb_device::endpoint::{EndpointAddress, EndpointType};
use usb_device::{Result, UsbDirection, UsbError};

#[no_mangle]
static mut REG: [u32; 32] = [0; 32];

fn reg(i: usize) -> u32 {
    unsafe { read_volatile(addr_of!(REG[i % 32])) }
}

fn set(i: usize, v: u32) {
    unsafe { write_volatile(addr_of_mut!(REG[i % 32]), v) }
}

/// Endpoint addresses 1 to 15 in each direction, given in order.
pub struct StubBus {
    next: [u8; 2],
}

impl StubBus {
    pub const fn new() -> Self {
        StubBus { next: [1, 1] }
    }
}

impl UsbBus for StubBus {
    fn alloc_ep(
        &mut self,
        ep_dir: UsbDirection,
        ep_addr: Option<EndpointAddress>,
        _ep_type: EndpointType,
        max_packet_size: u16,
        _interval: u8,
    ) -> Result<EndpointAddress> {
        set(1, u32::from(max_packet_size));
        if let Some(a) = ep_addr {
            return Ok(a);
        }
        let d = usize::from(ep_dir == UsbDirection::In);
        if self.next[d] > 15 {
            return Err(UsbError::EndpointOverflow);
        }
        let n = self.next[d];
        self.next[d] += 1;
        Ok(EndpointAddress::from_parts(usize::from(n), ep_dir))
    }

    fn enable(&mut self) {
        set(2, 1);
    }

    fn reset(&self) {
        set(2, 2);
    }

    fn set_device_address(&self, addr: u8) {
        set(3, u32::from(addr));
    }

    fn write(&self, ep_addr: EndpointAddress, buf: &[u8]) -> Result<usize> {
        for (i, b) in buf.iter().enumerate() {
            set(8 + i % 16, u32::from(*b) | (u32::from(u8::from(ep_addr)) << 8));
        }
        Ok(buf.len())
    }

    fn read(&self, ep_addr: EndpointAddress, buf: &mut [u8]) -> Result<usize> {
        let n = (reg(4 + ep_addr.index() % 4) as usize).min(buf.len());
        if n == 0 {
            return Err(UsbError::WouldBlock);
        }
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = reg(8 + i % 16) as u8;
        }
        Ok(n)
    }

    fn set_stalled(&self, ep_addr: EndpointAddress, stalled: bool) {
        set(5, u32::from(u8::from(ep_addr)) | (u32::from(stalled) << 8));
    }

    fn is_stalled(&self, ep_addr: EndpointAddress) -> bool {
        reg(5) & (1 << ep_addr.index()) != 0
    }

    fn suspend(&self) {
        set(2, 3);
    }

    fn resume(&self) {
        set(2, 4);
    }

    fn poll(&self) -> PollResult {
        let r = reg(0);
        match r & 7 {
            1 => PollResult::Reset,
            2 => PollResult::Suspend,
            3 => PollResult::Resume,
            4 => PollResult::Data {
                ep_out: (r >> 8) as u16,
                ep_in_complete: (r >> 16) as u16,
                ep_setup: (r >> 24) as u16,
            },
            _ => PollResult::None,
        }
    }
}
