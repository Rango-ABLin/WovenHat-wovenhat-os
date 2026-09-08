# WovenHat OS 0.8.0 - Multicore Foundation

This release starts and schedules work on up to four x86-64 CPUs. It preserves
BSP ownership of userspace, filesystem, network, pager and block-I/O services.
It is a bounded multicore kernel foundation, not unrestricted multicore userspace.

## Implemented

- Validated ACPI processor IDs, IOAPIC address/GSI base and ISA overrides.
- INIT/SIPI AP startup through a usable low-memory page excluded from allocation.
- Separate CPU bootstrap stacks, GDTs, TSSs, privilege stacks and double-fault stacks.
- Calibrated periodic LAPIC timers and IOAPIC keyboard routing; PIC fallback when
  usable ACPI topology is absent. Unused IOAPIC inputs remain masked.
- CPU-local current-task and round-robin state. Tasks remain pinned to their CPU.
  Only the owning CPU can switch away from or reap a task's stack.
- Explicit kernel-job affinity and least-loaded placement for audited parallel jobs.
- Synchronous NMI-based TLB shootdowns. The handler takes no locks; the initiator
  disables local preemption and waits for every online CPU's generation acknowledgement.
  Mapping changes invalidate translations before old physical frames are reclaimed.
- `smp` diagnostics and `smptest` regression command in the diagnostic shell.
- Fixed pre-existing storage compiler errors (inconsistent depth constant and a
  recursive mutable-device borrow); directory traversal finishes before recursion.
- Fixed strict lint failures and separated freestanding kernel lint from host lint.

## Release verification

Run from the repository root:

```powershell
python scripts/test-release.py
python scripts/test-shell-qemu.py
```

Linux/alternate installations must supply `--qemu` and `--firmware` paths.
The release runner writes per-check logs and `results.json` to
`target/release-validation/`. It fails immediately on any failed gate.

The automated gates cover:

1. Strict kernel and host Clippy checks.
2. Standalone cache, file-mapping and FAT32 host regressions.
3. Complete debug boot on 1, 2 and 4 CPUs, both diskless and with disposable ATA/FAT32.
4. Optimized 4-CPU boot, both diskless and with disposable ATA/FAT32.
5. Normal optimized image build.
6. A separate normal-image keyboard smoke test: QMP injects PS/2 keys for `smptest`,
   which must execute through the IOAPIC and complete all SMP checks.

SMP checkpoints require an exact online CPU count, an all-CPU execution barrier,
per-CPU timer preemption of non-yielding jobs, remote cached mapping replacement
and frame reclamation, and repeated acknowledged shootdowns. A VM configured
with `-smp 4` alone is not accepted as proof of SMP.

Storage tests now require the complete boot-suite success marker and QEMU exit 33,
rather than terminating as soon as the first storage checkpoint appears. They
only recreate disks under `target/storage-regression-*`; the runtime data disk
is not used by these tests.

## Try the release

```powershell
.\run-release.ps1 -Cpus 4
```

In the diagnostic shell, run `version`, `smp`, `smptest`, `tasks`, `ls /mnt`,
`df /mnt`, and `fscheck /mnt`. `smptest` is one-shot per boot. The launcher uses
and preserves the existing `wovenhat-disk.img`; ordinary shell writes persist.

## Supported scope and remaining work

- 1-4 CPUs, xAPIC IDs below 256, one IOAPIC, and QEMU `pc`/`q35` configurations.
  Unsupported or truncated topology fails explicitly. x2APIC, CPU hotplug,
  multiple IOAPICs, NUMA and physical-hardware certification remain future work.
- Shared scheduler metadata is locked; per-CPU runnable sets are bounded views
  over the fixed task table. Placement balances audited kernel jobs at creation.
  There is no work stealing or task migration.
- Userspace and I/O stay on CPU 0. Unrestricted cross-core userspace, concurrent
  filesystem calls and movable pager/block workers need further ownership work.
- Kernel/AP stacks are separately owned but do not yet have unmapped guards.
- Shootdowns conservatively flush all CPUs and all translations. Targeted
  address-space masks/ranges and batching are future performance improvements.
- Tests demonstrate concurrency and correctness, not a near-linear speedup claim.
- FAT32 crash consistency, modern storage drivers, USB, a compositor, SDK/packages
  and the broader project-map differentiators are outside this release.

## Architecture reference

AP startup and APIC behavior follow the Intel SDM system-programming chapters:
https://cdrdv2-public.intel.com/874240/325462-090-sdm-vol-1-2abcd-3abcd-4.pdf
