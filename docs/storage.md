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

The ATA module implements polling-mode discovery and read-only sector transport for
the legacy primary-master channel. IDENTIFY and every transfer use bounded status
polling, reject device faults, and expose at most the LBA28 address range. A detected
disk is registered as ata0; systems without legacy ATA continue booting without a block
device. Boot validates an actual LBA 0 transfer when present.

## FAT32 validation

`kernel/src/fat32.rs` provides strict mount metadata validation, short-name lookup
across the root-directory cluster chain, FAT-chain decoding, and bounded file reads. It
validates:

- the 0x55AA boot signature;
- 512-byte sectors;
- power-of-two cluster geometry;
- reserved-sector and FAT counts;
- FAT32-only BPB fields;
- declared media size against the block device;
- FAT/data-region overflow;
- the FAT32 minimum cluster count;
- root-cluster bounds;
- deleted, long-name, and volume-label directory entries;
- free, bad, end-of-chain, and out-of-range FAT entries;
- cyclic, overlong, and prematurely terminated file chains.

The parser reports corrupt and unsupported media without indexing outside a sector.

## VFS mount

The VFS uses a bounded 16-node registry with 64-byte paths and 8 KiB file payloads.
Built-in files retain explicit write permissions. When ata0 contains a FAT32 volume,
boot first checks for a superfloppy filesystem at LBA 0, then the four primary MBR
entries for FAT32 types 0x0B and 0x0C, and finally a CRC-validated GPT behind a
protective MBR. EFI System and Microsoft Basic Data GUIDs are candidates; FAT32 BPB
validation determines the actual format. Partition-relative reads are bounds checked
against both the selected partition and underlying device. See
[partition-table discovery](partition-tables.md). Boot imports up to eight
regular root files as read-only lowercase 8.3 paths below /mnt and records the resulting
node count for later lifecycle checks. No-disk and non-FAT media are non-fatal outcomes.

## Current boundary

File reads follow at most 64 clusters and may span multiple sectors and clusters.
Subdirectory lookup and directory mutation are not yet
implemented. The next storage increments are:

1. Subdirectory lookup and path traversal.
2. Backup-GPT validation and extended/logical MBR partitions.
3. ATA writes plus secondary-channel and slave-device discovery.
4. AHCI, NVMe, or virtio-blk transport.
5. Crash-safe file and directory updates.

Boot-time self-tests validate block bounds, read-only protection, FAT32 geometry,
root-chain lookup, missing entries, invalid signatures, MBR/GPT bounds, GPT CRC corruption, partition-relative I/O, multi-cluster reads,
end-of-chain handling, and file/directory cycle rejection.
