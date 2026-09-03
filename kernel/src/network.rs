//! smoltcp-facing Ethernet adapter for WovenHat networking.
//!
//! This establishes the no_std smoltcp integration and packet boundary used by
//! the VirtIO-net transport. IPv4 defaults are intentionally link-local until
//! DHCP/configuration syscalls are added.

use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};

use crate::virtio_net::{self, MAX_FRAME};

pub const DEFAULT_MAC: EthernetAddress = EthernetAddress([0x02, 0x57, 0x48, 0x00, 0x00, 0x01]);
pub const DEFAULT_IPV4: Ipv4Address = Ipv4Address::new(169, 254, 87, 72);

pub fn default_cidr() -> IpCidr { IpCidr::new(IpAddress::Ipv4(DEFAULT_IPV4), 16) }

pub struct VirtioSmolDevice {
    rx: [u8; MAX_FRAME],
}
impl VirtioSmolDevice { pub const fn new() -> Self { Self { rx: [0; MAX_FRAME] } } }

pub struct WovenRxToken<'a> { data: &'a mut [u8] }
pub struct WovenTxToken;

impl RxToken for WovenRxToken<'_> {
    fn consume<R, F>(self, f: F) -> R where F: FnOnce(&[u8]) -> R { f(self.data) }
}
impl TxToken for WovenTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R where F: FnOnce(&mut [u8]) -> R {
        let mut frame = [0u8; MAX_FRAME];
        let usable = core::cmp::min(len, MAX_FRAME);
        let result = f(&mut frame[..usable]);
        let _ = virtio_net::transmit(&frame[..usable]);
        result
    }
}

impl Device for VirtioSmolDevice {
    type RxToken<'a> = WovenRxToken<'a> where Self: 'a;
    type TxToken<'a> = WovenTxToken where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let len = virtio_net::receive_into(&mut self.rx)?;
        Some((WovenRxToken { data: &mut self.rx[..len] }, WovenTxToken))
    }
    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> { Some(WovenTxToken) }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps.checksum = ChecksumCapabilities::ignored();
        caps
    }
}

pub fn self_test() -> bool {
    default_cidr().prefix_len() == 16 && DEFAULT_MAC.0[0] & 1 == 0 && virtio_net::self_test()
}
