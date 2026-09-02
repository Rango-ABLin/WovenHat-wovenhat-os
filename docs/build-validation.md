# Build and Boot Validation

## Local static gates

Run these from the repository root:

    cargo clippy --workspace -- -D warnings
    cargo check --workspace
    cargo build --release

The root build script consumes the freestanding x86_64 kernel artifact and creates a
UEFI disk image named wovenhat-os-uefi.img under the corresponding Cargo build output
directory.

## QEMU validation image

The qemu-test feature preserves every normal boot validation but, after the breakpoint
handler test and the serial marker shown below, writes 0x10 to the isa-debug-exit port:

    [BOOT] ALL VALIDATIONS PASSED

Build it with:

    cargo build --features qemu-test

With isa-debug-exit configured at I/O port 0xf4, QEMU returns host status 33 for this
success value. The normal feature set does not access that port and continues into the
desktop and diagnostic-shell loop.

## Continuous integration

The kernel workflow installs nightly Rust, the freestanding target, Clippy, QEMU, and
OVMF. It runs strict lint, builds the feature-gated UEFI image, boots it headlessly with
serial output, enforces a 90-second timeout, and accepts only exit status 33.

A representative Linux invocation is:

    qemu-system-x86_64 -machine q35 -m 256M -display none -serial stdio \
      -no-reboot -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
      -bios /usr/share/OVMF/OVMF_CODE.fd -drive format=raw,file=<image>

QEMU is not currently installed in the Windows development environment, so runtime
results must come from CI or a machine with QEMU and OVMF. Static builds do not replace
the emulator boot gate.
