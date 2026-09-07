# Memory milestone validation - 2026-09-07

| Area | Implemented and checked |
| --- | --- |
| Boot baseline | Scheduler initialized before pipe tests; mmap arena no longer overlaps the stack; corrected Ring-3 pointer fixture; full boot suite passes |
| Mapping lifetime | Referenced RAM and disk inodes survive unlink/replacement; shared truncation zeroes resident tails and absent truncated reads fail |
| Physical cache | Mapping aliases share physical cache frames; private writes split through COW; VFS shared read/write coherence |
| Reclamation | Unpinned LRU eviction, allocation-pressure reclaim/retry, pinned-frame protection, exact cleanup; reference-table exhaustion and overflow rollback |
| Shared mappings | Syscall 62, shared aliases across fork, syscall 63 msync, RAM writeback and durable ATA/FAT32 flush |
| Fault I/O | No process/physical-cache lock across backing reads; CPU IRQs remain enabled with a BSP preemption guard; live timer IRQ and state-restoration test |

Validation performed:

- `python scripts/test-memory-qemu.py`: PASS, QEMU exit 33 and
  `[BOOT] ALL VALIDATIONS PASSED`, with two CPUs advertised but BSP scheduling.
- Host bounds test: 1 passed; buffer-cache tests: 4 passed; file-page-cache/FAT32
  tests: 5 passed.
- Normal QEMU: repeated `mmaptest` reports `FILE MMAP: PASS`.
- Disposable ATA disk: `msynctest` reports `MSYNC DISK: PASS`, checks unlink of
  a disk-backed mapped file followed by persisted replacement, and verifies
  both saved fixtures after a fresh boot.
- `git diff --check`: passed. Clippy still reports existing repository issues;
  this is not a claim of a clean Clippy baseline.

Limits: eviction targets unpinned cache frames, not live process pages. No swap,
asynchronous pager worker, or application-processor scheduling is implemented.
Before enabling SMP, the BSP preemption guard must become per-CPU, and mapping
publication/unmap synchronization and remote TLB shootdowns must be added.
See `file-mmap.md` for ABI and detailed semantics.
