# Storage Architecture

WovenHat's storage stack is split into a hardware-independent sector layer and
filesystem parsers.

## Block-device contract

`kernel/src/block.rs` defines 512-byte logical sectors through `BlockDevice`:

- `sector_count` reports the addressable media size.
- `read_sector` and `write_sector` require exactly one 512-byte buffer.
- Out-of-range, malformed-buffer, and read-only failures are explicit.
- `RamDisk` provides a deterministic implementation for kernel validation.

The interface is intentionally allocation-free and can be implemented by ATA, AHCI,
NVMe, virtio-blk, or USB mass-storage drivers.

## FAT32 validation

`kernel/src/fat32.rs` currently provides strict mount metadata validation and
short-name lookup in the first root-directory sector. It validates:

- the 0x55AA boot signature;
- 512-byte sectors;
- power-of-two cluster geometry;
- reserved-sector and FAT counts;
- FAT32-only BPB fields;
- declared media size against the block device;
- FAT/data-region overflow;
- the FAT32 minimum cluster count;
- root-cluster bounds;
- deleted, long-name, and volume-label directory entries.

The parser reports corrupt and unsupported media without indexing outside a sector.

## Current boundary

The parser does not yet follow FAT chains or write directory entries. Physical media is
also not connected yet. The next storage increments are:

1. ATA PIO discovery and sector transport.
2. FAT-chain traversal with loop detection.
3. Mounting a FAT32 volume into the VFS namespace.
4. Read support followed by crash-safe file updates.

Boot-time self-tests validate block bounds, read-only protection, FAT32 geometry,
root lookup, missing entries, and invalid signatures.
