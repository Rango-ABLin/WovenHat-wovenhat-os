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

## PCI inventory

The HAL scans PCI configuration mechanism 1 across all 256 buses, 32 device slots,
and advertised multifunction functions. It records up to 64 functions and reports
total storage, network, display, and bridge classes over serial. Inventory truncation
is explicit. Enumeration is read-only and completes before interrupts are enabled.
A detected legacy primary-master disk is registered separately as ata0.

## Invariants

- The registry has a fixed capacity and does not allocate.
- Device names are unique.
- A hardware IRQ can belong to only one registered device.
- Lookup returns a copied descriptor, so callers cannot mutate registry state.
- Boot fails closed if registration or the registry self-test fails.

## Extension path

PCI functions are inventoried but are only added to the device registry after a driver
binds them. ACPI discovery should supply interrupt routing and board topology. Driver
binding should extend Device with bus identity and lifecycle state while preserving
the name and IRQ uniqueness checks.
