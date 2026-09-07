# File mappings, cache ownership, and writeback

## ABI

| Syscall | Name | Behavior |
| --- | --- | --- |
| 58 | MmapFile | Eager read-only private snapshot |
| 59 | MmapFileWritable | Eager writable private snapshot |
| 60 | MmapFileLazy | Demand-paged read-only private mapping |
| 61 | MmapFileLazyWritable | Demand-paged writable private mapping |
| 62 | MmapFileShared | Demand-paged shared writable mapping |
| 63 | Msync | Flush an entire shared mapping; persist named /mnt files |

Mapping calls take RDI=fd, RSI=byte length, RDX=file offset. Length must be
1..65536, offset must be page aligned, and the requested range must fit the
file at creation. The return value is an address or u64::MAX. All mappings are
NX. Anonymous mmap (8) keeps its existing ABI. Shared mappings require FileRead
and FileWrite capabilities and a writable regular file; immutable RAM files
are rejected. Imported disk files are materialized into their bounded RAM
inode image before becoming writable, then persisted by msync.

Shared lengths must be page aligned or end exactly at EOF, so aliases agree on
backing-page contents. Msync takes the mapping base and a length that rounds to
the entire allocation; it requires FileWrite and returns zero or u64::MAX.
Partial msync and partial munmap are not supported. Munmap remains syscall 9.
Wrappers are provided for all file mapping calls and msync.

The 16 mapping slots occupy offsets 0x0e0000..0x1dffff in the user region,
below the stack guard. A compile-time assertion prevents the former overlap
between the last mmap slot and the stack. Per-task entry stacks are 128 KiB.

## Lifetime and coherence

58/59 retain their creation-time snapshot semantics. Lazy private mappings
read each page on first access; later changes to a resident private page's
source do not modify that page. Private writes use COW and do not change the
file. Mapping reads never advance the shared open-file offset. Closing an fd
is safe because each mapping owns a reference, including across fork.

Unlink removes the name while retaining the inode until its last reference
closes. Recreating the pathname allocates another node. Path-based FAT32
backing is copied into the existing RAM inode buffer before an open disk file
is unlinked, preventing a persisted replacement from redirecting old faults.
Rename preserves node identity.

Shared aliases use the same physical page, including after fork. VFS reads
observe shared writes before msync; VFS writes update resident shared pages.
Truncation zeroes resident shared pages beyond the new EOF. An absent page
whose required backing bytes no longer exist fails to populate. Resident
private pages retain their bytes. Growth updates overlapping shared cache
pages; mappings do not expand their reserved address range.

Shared unmap/exit/exec copy resident bytes back to the RAM inode before
releasing frames. Only explicit msync guarantees an attempted durable flush
for a named /mnt file; ATA/FAT32/flush failures return an error. An unlinked
inode remains an in-memory object and is not written over a replacement name.

## Physical cache and reclamation

file_frames.rs owns up to 64 physical pages. Mapping aliases reference those
frames directly; private writes split through the existing COW ownership
table. Keys include inode identity, page offset, and private source version.
Private loads avoid stale cached snapshots while a shared writable alias can
change the file. The existing FAT32 byte cache remains below the backing read
path; this milestone does not replace every byte buffer with a physical frame.

LRU eviction selects cache-owned pages with no live mapping references.
Allocation failure also reclaims unpinned cache pages and retries. Mapped
frames are never freed by reclaim. Unmap releases unused cache ownership;
self-tests verify exact frame and open-reference reclamation. Ownership is
reserved before publishing a fork PTE; full-table and counter-overflow tests
verify failure cannot release a still-live source frame.

This is bounded cache reclamation, not swap or eviction of live process pages.
If all cache slots are pinned or frames remain unavailable, population fails.

## Fault I/O and multicore boundary

The fault path copies its mapping metadata and releases the process lock
before loading. Physical-cache misses perform I/O without holding that cache lock, then
recheck the key before publishing a frame. CPU interrupts are enabled during I/O, allowing timer IRQs to be serviced. A BSP preemption guard prevents another task from spinning on an
I/O lock held by the interrupted task; interrupt state is restored afterward.
A boot test verifies timer ticks advance, the task stays stable, and guard and
interrupt state are restored. The existing ATA and byte-cache backend locks
remain serialized by the BSP guard; they are not asynchronous I/O queues.

Only the bootstrap CPU schedules tasks. These ownership and lock boundaries
are preparation for SMP, not an SMP implementation: per-CPU preemption state,
in-flight mapping/unmap synchronization, remote TLB shootdowns, and AP startup
are still required before additional CPUs may schedule tasks. Fault I/O is
synchronous; there is no asynchronous pager worker yet.

## Tests

Run the isolated full boot suite from the repository directory:

```powershell
python scripts/test-memory-qemu.py
```

The runner builds the test feature, uses read-only boot media, leaves user disk
images untouched, and requires both QEMU exit 33 and ALL VALIDATIONS PASSED.
Existing test-only dead-code/import warning allowances are scoped to that
build. The scheduler ordering panic, mmap/stack overlap, and malformed
Ring-3 fixture pointers are fixed; the complete boot suite now passes.

For interactive tests, close the previous QEMU window and run:

```powershell
.\run-stage9.ps1
```

Inside WovenHat:

```text
mmaptest
mmaptest
msynctest
```

Expect FILE MMAP: PASS twice and MSYNC DISK: PASS. mmaptest exercises private,
lazy, shared, fork, kernel buffer-copy, and msync syscall paths. The kernel
suite also tests physical alias identity, pinned-frame protection, LRU eviction,
truncation, unlink/recreation, and resource reclamation.

msynctest creates /mnt/vmsync.txt and /mnt/vmpin.txt only when absent. Existing
files are verified, never overwritten by the probe. It checks durable msync
and disk-backed unlink/replacement isolation. Restart QEMU and rerun msynctest
to verify the saved data on a fresh boot. A missing/read-only ATA disk, I/O
failure, or existing fixture with different contents makes the probe fail.

run-stage9.ps1 obtains the normal image from Cargo's --print-image output,
so running the boot suite cannot make it select a newer self-test image.
