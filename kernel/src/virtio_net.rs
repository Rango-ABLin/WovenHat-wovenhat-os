//! VirtIO network device discovery and bounded frame queues.
//!
//! The PCI transport discovery recognizes both transitional (0x1000) and modern
//! (0x1041) VirtIO network functions. Queue ownership is kept here so the smoltcp
//! adapter does not depend on PCI details. The DMA/virtqueue transport can replace
//! `inject_rx`/`take_tx` without changing the network stack API.

use spin::Mutex;
use crate::hal::pci;

pub const VIRTIO_VENDOR: u16 = 0x1af4;
pub const VIRTIO_NET_LEGACY: u16 = 0x1000;
pub const VIRTIO_NET_MODERN: u16 = 0x1041;
pub const MAX_FRAME: usize = 1536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciLocation { pub bus: u8, pub device: u8, pub function: u8 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeStatus { Missing, Found(PciLocation) }

#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; MAX_FRAME],
    len: usize,
    ready: bool,
}
impl Frame { const fn empty() -> Self { Self { bytes: [0; MAX_FRAME], len: 0, ready: false } } }

struct Queues { rx: Frame, tx: Frame }
static QUEUES: Mutex<Queues> = Mutex::new(Queues { rx: Frame::empty(), tx: Frame::empty() });

pub fn probe() -> ProbeStatus {
    let mut index = 0usize;
    while let Some(dev) = pci::device(index) {
        if dev.vendor_id == VIRTIO_VENDOR
            && matches!(dev.device_id, VIRTIO_NET_LEGACY | VIRTIO_NET_MODERN)
            && dev.class == 0x02
        {
            return ProbeStatus::Found(PciLocation { bus: dev.bus, device: dev.device, function: dev.function });
        }
        index += 1;
    }
    ProbeStatus::Missing
}

/// Called by the eventual virtqueue interrupt path to hand an Ethernet frame up.
pub fn inject_rx(frame: &[u8]) -> bool {
    if frame.is_empty() || frame.len() > MAX_FRAME { return false; }
    let mut q = QUEUES.lock();
    if q.rx.ready { return false; }
    q.rx.bytes[..frame.len()].copy_from_slice(frame);
    q.rx.len = frame.len();
    q.rx.ready = true;
    true
}

pub fn receive_into(out: &mut [u8]) -> Option<usize> {
    let mut q = QUEUES.lock();
    if !q.rx.ready || out.len() < q.rx.len { return None; }
    let len = q.rx.len;
    out[..len].copy_from_slice(&q.rx.bytes[..len]);
    q.rx = Frame::empty();
    Some(len)
}

/// Queue one frame for the VirtIO transport. Returns false under backpressure.
pub fn transmit(frame: &[u8]) -> bool {
    if frame.is_empty() || frame.len() > MAX_FRAME { return false; }
    let mut q = QUEUES.lock();
    if q.tx.ready { return false; }
    q.tx.bytes[..frame.len()].copy_from_slice(frame);
    q.tx.len = frame.len();
    q.tx.ready = true;
    true
}

/// Called by the transport to consume the next pending TX frame.
pub fn take_tx(out: &mut [u8]) -> Option<usize> {
    let mut q = QUEUES.lock();
    if !q.tx.ready || out.len() < q.tx.len { return None; }
    let len = q.tx.len;
    out[..len].copy_from_slice(&q.tx.bytes[..len]);
    q.tx = Frame::empty();
    Some(len)
}

pub fn self_test() -> bool {
    let sample = [0x5au8; 64];
    let mut out = [0u8; 64];
    inject_rx(&sample) && receive_into(&mut out) == Some(64) && out == sample
        && transmit(&sample) && take_tx(&mut out) == Some(64) && out == sample
}
