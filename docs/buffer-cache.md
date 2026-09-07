# Persistent buffer cache

The ATA primary-master device owns a bounded, allocation-free 64-sector
(32 KiB payload) write-back buffer cache. The same cache serves boot mounting,
partition discovery, on-demand FAT32 reads, file persistence, and directory
persistence. Cache keys are absolute device LBAs; partition views translate
before reaching the cache. Each owned device has its own cache.

Reads hit cached sectors or load from the device. Full-sector writes avoid a
read-before-write and coalesce until explicit flush or eviction. LRU replacement
writes a dirty victim before reusing it. Failed writeback leaves the dirty
entry and its bytes intact for retry; failed reads do not install partial data.
Read-only devices reject writes immediately. Zero-capacity instances safely
bypass caching. Access ages saturate instead of wrapping.

`BlockDevice::flush` drains software buffers through wrappers, including
partition views and borrowed devices. Persist operations flush before reporting
success, retaining clean entries for later operations. Drop does not silently
flush or claim success. ATA hardware-cache durability, power-loss ordering,
journaling, and background writeback are not provided by this milestone.

`sync` snapshots mounted filenames before persisting, avoiding the previous
recursive VFS-lock acquisition. It retries the shared device cache flush and
reports failure instead of a successful count when persistence fails. The sync
syscall returns its existing generic error sentinel on failure.

The diagnostic-shell `fs` command shows hit/miss, writeback and eviction counts,
plus resident, dirty, and capacity sector counts. It reports no ATA device if
none is detected. VFS RAM-file reads still use their resident node data; this
is a sector buffer cache, not a VM-backed file-page cache. File page caching,
page reclamation, and mmap coherence remain separate future work.

## Validation (2026-09-07)

- Normal `cargo build` passed.
- Host tests: `rustc --edition 2021 --test tests/buffer_cache.rs -o
  target/buffer-cache-tests.exe`, then `target/buffer-cache-tests.exe`: 4 passed.
  These cover read reuse, write coalescing, dirty LRU eviction, backing-device
  contents, flush retry, failed reads, invalid buffers, bounds, read-only media,
  zero capacity, and partition flush forwarding.
- QEMU test feature: `[BUFFER CACHE] regression tests: PASSED` and
  `[VFS] read/write and path semantics: PASSED`. The broader suite then stops at
  the previously observed `scheduler not initialized` panic in task.rs.
  This feature required the existing invocation-only warning allowances
  `RUSTFLAGS='-A dead_code -A unused_imports'`.
- Live normal-kernel QEMU test used a disposable raw FAT32 image with 70,000
  sectors on `-machine pc` (legacy IDE). The current ATA PIO driver did not
  detect the Q35 SATA disk. Production disk contents were not used in the test.
  FAT32 mounted successfully. Two successive `persist /mnt/new.txt` calls
  increased hits from 13 to 34 while misses stayed at 6; `sync` returned after
  persisting two files and brought hits to 76 with misses still at 6. Dirty
  sectors were zero after each persistence operation. After QEMU shutdown,
  direct inspection of the backing FAT32 image confirmed `NEW.TXT` = `cached`.
- Target-specific Clippy still reports the same 16 pre-existing errors outside
  the cache changes. `git diff --check` passed.

## Manual check

On a normal boot with a supported ATA FAT32 disk, run:

```text
fs
write /mnt/new.txt cached
persist /mnt/new.txt
fs
persist /mnt/new.txt
fs
sync
fs
```

Successful persistence prints `persist: written to FAT32`; repeat operations
should show increased cache hits and no dirty sectors after a successful flush.

## Default launcher disk setup

`run-stage9.ps1` now uses legacy IDE (`-machine pc`) with the persistent raw
FAT32 image `wovenhat-disk.img`. On first use, `scripts/create-fat32.py` creates
an empty 35,840,000-byte FAT32 volume; existing images are never overwritten.
The image is ignored by Git. The old `wovenhat-data` host folder is preserved,
but its files are not automatically imported into the new image.

This replaces QEMU's `fat:rw:` folder backend, whose generated filesystem did
not satisfy the kernel FAT32 geometry requirements. Changing only the machine
type enabled ATA detection but still left persistence failing.

Validated a disposable copy of the generated image in QEMU: FAT32 mounted,
`persist /mnt/cache.txt` succeeded, repeat persist increased hits from 12 to 32
with misses fixed at 5 and dirty=0, and sync completed successfully.


Follow-up: demand-loaded clean FAT32 file-page caching is now implemented;
see `file-page-cache.md`. VM-backed mappings and physical-page reclamation
remain future work.
