//! WovenHat IPv4 network stack.
//!
//! The physical device is the real VirtIO-net transport in `virtio_net` and
//! smoltcp supplies Ethernet/ARP/IPv4 processing. Stage 3 uses QEMU user-mode
//! networking's conventional 10.0.2.0/24 topology so the OS has a usable
//! standalone network without depending on DHCP yet.

use spin::{Mutex, Once};
use smoltcp::{
    iface::{Config, Interface, SocketSet, SocketStorage},
    phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken},
    time::Instant,
    wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address},
};

use crate::{timer, virtio_net::{self, MAX_FRAME}};

pub const DEFAULT_IPV4: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
pub const DEFAULT_GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);
pub const DEFAULT_PREFIX: u8 = 24;

pub fn default_cidr() -> IpCidr { IpCidr::new(IpAddress::Ipv4(DEFAULT_IPV4), DEFAULT_PREFIX) }

pub struct VirtioSmolDevice { rx: [u8; MAX_FRAME] }
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
        caps.max_transmission_unit = 1500;
        caps.checksum = ChecksumCapabilities::ignored();
        caps
    }
}

struct Runtime { iface: Interface, device: VirtioSmolDevice }
static RUNTIME: Once<Mutex<Runtime>> = Once::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError { Transport(virtio_net::InitError), Route }

pub fn init() -> Result<(), InitError> {
    if RUNTIME.get().is_some() { return Ok(()); }
    virtio_net::init().map_err(InitError::Transport)?;

    let mac = EthernetAddress(virtio_net::mac_address());
    let mut device = VirtioSmolDevice::new();
    let mut config = Config::new(mac.into());
    config.random_seed = 0x5748_4f53_4e45_5433;
    let now = now();
    let mut iface = Interface::new(config, &mut device, now);
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(default_cidr());
    });
    iface.routes_mut().add_default_ipv4_route(DEFAULT_GATEWAY).map_err(|_| InitError::Route)?;
    RUNTIME.call_once(|| Mutex::new(Runtime { iface, device }));
    Ok(())
}

pub fn poll() {
    virtio_net::poll();
    let Some(runtime) = RUNTIME.get() else { return; };
    let mut runtime = runtime.lock();
    let mut storage = [const { SocketStorage::EMPTY }; 1];
    let mut sockets = SocketSet::new(&mut storage[..]);
    let Runtime { iface, device } = &mut *runtime;
    let _ = iface.poll(now(), device, &mut sockets);
}

pub fn initialized() -> bool { RUNTIME.get().is_some() && virtio_net::is_initialized() }

pub fn self_test() -> bool {
    default_cidr().prefix_len() == DEFAULT_PREFIX
        && DEFAULT_GATEWAY == Ipv4Address::new(10, 0, 2, 2)
        && virtio_net::self_test()
}

fn now() -> Instant {
    let millis = timer::ticks().saturating_mul(1000) / timer::FREQUENCY_HZ as u64;
    Instant::from_millis(millis as i64)
}
