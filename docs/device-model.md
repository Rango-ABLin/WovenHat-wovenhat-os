# Kernel Device Registry

WovenHat exposes boot hardware through a bounded kernel registry in `kernel/src/device.rs`.
The registry is initialized after the concrete drivers are initialized and before IRQs are
unmasked.

## Registered boot devices

| Name | Kind | IRQ |
| --- | --- | --- |
| `framebuffer-console` | Console | none |
| `com1` | Serial | none |
| `pit` | Timer | 0 |
| `ps2-keyboard` | Keyboard | 1 |

## Invariants

- The registry has a fixed capacity and does not allocate.
- Device names are unique.
- A hardware IRQ can belong to only one registered device.
- Lookup returns a copied descriptor, so callers cannot mutate registry state.
- Boot fails closed if registration or the registry self-test fails.

## Extension path

ACPI and PCI discovery should register discovered controllers through this API. Driver
binding should extend `Device` with bus identity and lifecycle state while preserving
the name and IRQ uniqueness checks.
