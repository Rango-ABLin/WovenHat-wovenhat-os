//! WovenHat IPv4 network stack.
//!
//! The physical device is the real VirtIO-net transport in `virtio_net` and
//! smoltcp supplies Ethernet/ARP/IPv4 processing. Stage 3 uses QEMU user-mode
//! networking's conventional 10.0.2.0/24 topology so the OS has a usable
//! standalone network without depending on DHCP yet.

use spin::{Mutex, Once};
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet, SocketStorage},
    phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::udp,
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

struct Runtime {
    iface: Interface,
    device: VirtioSmolDevice,
    sockets: SocketSet<'static>,
    echo_handle: Option<SocketHandle>,
    echo_port: u16,
    echo_packets: u64,
}
static RUNTIME: Once<Mutex<Runtime>> = Once::new();

const SOCKET_SLOTS: usize = 4;
const UDP_META_SLOTS: usize = 4;
const UDP_BUFFER_BYTES: usize = 2048;
static mut SOCKET_STORAGE: [SocketStorage<'static>; SOCKET_SLOTS] =
    [const { SocketStorage::EMPTY }; SOCKET_SLOTS];
static mut ECHO_RX_META: [udp::PacketMetadata; UDP_META_SLOTS] =
    [udp::PacketMetadata::EMPTY; UDP_META_SLOTS];
static mut ECHO_TX_META: [udp::PacketMetadata; UDP_META_SLOTS] =
    [udp::PacketMetadata::EMPTY; UDP_META_SLOTS];
static mut ECHO_RX_DATA: [u8; UDP_BUFFER_BYTES] = [0; UDP_BUFFER_BYTES];
static mut ECHO_TX_DATA: [u8; UDP_BUFFER_BYTES] = [0; UDP_BUFFER_BYTES];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoError { NetworkOffline, AlreadyConfigured, BindFailed, SocketTableFull }

#[derive(Clone, Copy, Debug, Default)]
pub struct NetStats {
    pub online: bool,
    pub echo_active: bool,
    pub echo_port: u16,
    pub echo_packets: u64,
}


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
    let socket_storage: &'static mut [SocketStorage<'static>; SOCKET_SLOTS] = unsafe {
        &mut *core::ptr::addr_of_mut!(SOCKET_STORAGE)
    };
    let sockets = SocketSet::new(&mut socket_storage[..]);
    RUNTIME.call_once(|| Mutex::new(Runtime {
        iface,
        device,
        sockets,
        echo_handle: None,
        echo_port: 0,
        echo_packets: 0,
    }));
    Ok(())
}

pub fn poll() {
    virtio_net::poll();
    let Some(runtime) = RUNTIME.get() else { return; };
    let mut runtime = runtime.lock();

    {
        let Runtime { iface, device, sockets, .. } = &mut *runtime;
        let _ = iface.poll(now(), device, sockets);
    }

    if let Some(handle) = runtime.echo_handle {
        let mut reply = [0u8; 512];
        let received = {
            let socket = runtime.sockets.get_mut::<udp::Socket>(handle);
            match socket.recv() {
                Ok((data, meta)) => {
                    let len = core::cmp::min(data.len(), reply.len());
                    reply[..len].copy_from_slice(&data[..len]);
                    Some((len, meta.endpoint))
                }
                Err(_) => None,
            }
        };
        if let Some((len, remote)) = received {
            let sent = {
                let socket = runtime.sockets.get_mut::<udp::Socket>(handle);
                socket.send_slice(&reply[..len], remote).is_ok()
            };
            if sent { runtime.echo_packets = runtime.echo_packets.saturating_add(1); }
        }
    }

    {
        let Runtime { iface, device, sockets, .. } = &mut *runtime;
        let _ = iface.poll(now(), device, sockets);
    }
}

pub fn start_udp_echo(port: u16) -> Result<(), EchoError> {
    if port == 0 { return Err(EchoError::BindFailed); }
    let Some(runtime) = RUNTIME.get() else { return Err(EchoError::NetworkOffline); };
    let mut runtime = runtime.lock();
    if runtime.echo_handle.is_some() {
        return if runtime.echo_port == port { Ok(()) } else { Err(EchoError::AlreadyConfigured) };
    }

    let rx_meta: &'static mut [udp::PacketMetadata; UDP_META_SLOTS] = unsafe {
        &mut *core::ptr::addr_of_mut!(ECHO_RX_META)
    };
    let tx_meta: &'static mut [udp::PacketMetadata; UDP_META_SLOTS] = unsafe {
        &mut *core::ptr::addr_of_mut!(ECHO_TX_META)
    };
    let rx_data: &'static mut [u8; UDP_BUFFER_BYTES] = unsafe {
        &mut *core::ptr::addr_of_mut!(ECHO_RX_DATA)
    };
    let tx_data: &'static mut [u8; UDP_BUFFER_BYTES] = unsafe {
        &mut *core::ptr::addr_of_mut!(ECHO_TX_DATA)
    };

    let rx = udp::PacketBuffer::new(&mut rx_meta[..], &mut rx_data[..]);
    let tx = udp::PacketBuffer::new(&mut tx_meta[..], &mut tx_data[..]);
    let mut socket = udp::Socket::new(rx, tx);
    socket.bind(port).map_err(|_| EchoError::BindFailed)?;
    let handle = runtime.sockets.add(socket);
    runtime.echo_handle = Some(handle);
    runtime.echo_port = port;
    Ok(())
}

pub fn stats() -> NetStats {
    let Some(runtime) = RUNTIME.get() else { return NetStats::default(); };
    let runtime = runtime.lock();
    NetStats {
        online: virtio_net::is_initialized(),
        echo_active: runtime.echo_handle.is_some(),
        echo_port: runtime.echo_port,
        echo_packets: runtime.echo_packets,
    }
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
