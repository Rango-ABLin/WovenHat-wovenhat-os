# WovenHat OS: Complete AI Agent Directive & Implementation Guide

**Version**: 1.0  
**Date**: 2026-09-02  
**Status**: Active Development (v0.0.7 → v2.0.0)  
**Audience**: AI Agents, Developers, Project Stakeholders

---

## 📖 Quick Navigation

- [Executive Summary](#executive-summary) - What is WovenHat OS
- [Project Vision](#project-vision) - Strategic goals & differentiation
- [Current State Analysis](#current-state-analysis) - v0.0.7 breakdown
- [Complete Development Roadmap](#complete-development-roadmap) - Phase 1-5 timeline
- [Phase 1 Implementation](#phase-1-implementation-detailed) - Week-by-week guide with code
- [Architecture Specifications](#architecture-specifications) - Syscalls, capabilities, memory model
- [AI Agent Instructions](#ai-agent-instructions) - How to use this document

---

## Executive Summary

### What is WovenHat OS?

WovenHat is a revolutionary **AI-native, capability-based operating system** designed to become the world's first OS that:

1. **Understands User Intent** — Users command apps into existence via natural language
2. **Provides Advanced Security** — Fine-grained capability model prevents privilege escalation
3. **Scales Transparently** — Single machine or multi-node cluster, seamless to apps
4. **Enables Time-Travel Debugging** — Step forward/backward through execution deterministically
5. **Adapts Intelligently** — OS learns context and auto-themes UI
6. **Shares Apps Peer-to-Peer** — Encrypted, signed app transfer without app stores

### Current Status

- **Version**: 0.0.7 (Stable x86_64 microkernel foundation)
- **What Works**: Memory, scheduling, interrupts, capabilities, basic shell
- **What's Missing**: GUI, filesystem, AI, distributed architecture
- **Timeline**: 24 months to v2.0.0

### 24-Month Roadmap

```
v0.0.7 (NOW) → v0.1.0 (3mo) → v0.2.0 (6mo) → v1.0.0 (10mo) → 
v1.5.0 (14mo) → v2.0.0 (24mo)
```

---

## Project Vision

### Differentiation

| Aspect | Linux | macOS/Windows | WovenHat |
|--------|-------|---|---|
| **App Creation** | Manual coding | Manual coding | **AI: voice/text → app** |
| **Security** | User/Group DAC | Limited | **Capability-based, granular** |
| **Debugging** | GDB (limited replay) | Limited | **Full time-travel replay** |
| **Microkernel** | Monolithic | Monolithic | **True microkernel** |
| **Distributed** | Custom setup | Not built-in | **Built-in clustering** |
| **Open Source** | Yes (GPL) | Limited | **100% open** |

---

## Current State Analysis

### v0.0.7 Architecture

```
Monolithic x86_64 Kernel
├── Memory: Frame alloc, paging, 256 KiB heap
├── Tasks: Scheduling, context switch, preemption
├── Interrupts: IDT/GDT, exception handlers, 8259 PIC
├── Devices: Serial, PS/2 keyboard, PIT timer, framebuffer
├── Security: Capability model (7 capabilities)
├── Syscalls: 5 basic (yield, read, write, exit, getpid)
└── UI: Text shell only
```

### Limitations

1. No filesystem or persistent storage
2. No GUI or graphics rendering
3. No IPC (processes can't communicate)
4. Only 5 basic syscalls
5. No networking
6. No AI integration
7. Monolithic (not microkernel)
8. Single machine only
9. Hard-coded drivers (QEMU only)
10. Poor panic handling (silent halt)

---

## Complete Development Roadmap

### Phase 1: Foundation (Months 1-3)
- Structured panic handler
- Hardware abstraction layer (HAL)
- Filesystem (tmpfs + FAT32)
- 14+ syscalls
- Graphics engine
- GUI framework

### Phase 2: Desktop OS (Months 4-6)
- Complete window manager
- Widget framework
- Desktop shell (taskbar, menu)
- File manager, text editor
- Terminal emulator
- IPC framework

### Phase 3: AI-Powered (Months 7-10)
- LLM integration (Llama 2)
- Intent classifier
- Code generator (UI + logic)
- Sandbox enforcer
- App package format
- Adaptive runtime

### Phase 4: Distributed (Months 11-14)
- Microkernel refactoring
- Multi-node clustering
- Transparent RPC
- Multi-ISA support (ARM64, RISC-V)
- Task migration
- Device abstraction

### Phase 5: Novel Features (Months 15+)
- Time-travel debugging
- Semantic versioning
- Zero-trust audit logging
- Context-aware UI
- Federated app sharing
- Production hardening

---

## Phase 1 Implementation (Detailed)

### Week 1-2: Structured Panic Handler

**Goal**: Replace silent halt with CPU state dump

**Create**: `kernel/src/panic.rs`

```rust
use core::fmt;
use x86_64::registers::control::Cr2;

#[repr(C)]
pub struct PanicContext {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rbp: u64,
    pub r8: u64, pub r9: u64, pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rip: u64, pub rsp: u64, pub rflags: u64, pub cr2: u64,
}

impl PanicContext {
    pub fn capture() -> Self {
        let mut ctx = PanicContext {
            rax: 0, rbx: 0, rcx: 0, rdx: 0, rsi: 0, rdi: 0,
            rbp: 0, r8: 0, r9: 0, r10: 0, r11: 0, r12: 0,
            r13: 0, r14: 0, r15: 0, rip: 0, rsp: 0, rflags: 0,
            cr2: Cr2::read().as_u64(),
        };
        unsafe {
            core::arch::asm!(
                "mov {rax}, rax; mov {rbx}, rbx;",
                rax = out(reg) ctx.rax,
                rbx = out(reg) ctx.rbx,
            );
        }
        ctx
    }
    
    pub fn print(&self) {
        // Print to serial/framebuffer
    }
}

pub fn kernel_panic(message: &str) -> ! {
    x86_64::instructions::interrupts::disable();
    let ctx = PanicContext::capture();
    ctx.print();
    loop {
        x86_64::instructions::hlt();
    }
}
```

**Integrate into main.rs**:
```rust
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::panic::kernel_panic("panic");
}
```

**Testing**: Boot, trigger panic, verify CPU state output

---

### Week 3-4: Hardware Abstraction Layer (HAL)

**Create**: `kernel/src/hal/mod.rs`, `kernel/src/hal/cpu.rs`

```rust
// kernel/src/hal/mod.rs
pub mod cpu;

#[derive(Debug, Clone)]
pub struct HardwareInfo {
    pub cpu_vendor: CpuVendor,
    pub cpu_features: CpuFeatures,
    pub num_cpus: u32,
}

#[derive(Debug, Clone)]
pub enum CpuVendor { Intel, Amd, Unknown }

#[derive(Debug, Clone, Default)]
pub struct CpuFeatures {
    pub has_tsc: bool,
    pub has_rdrand: bool,
    pub has_aes_ni: bool,
    pub has_avx: bool,
}

pub fn init() -> HardwareInfo {
    HardwareInfo::detect()
}

// kernel/src/hal/cpu.rs
use x86_64::cpuid::CpuId;

pub fn detect_vendor() -> super::CpuVendor {
    let cpuid = CpuId::new();
    if let Some(vendor) = cpuid.get_vendor_info() {
        match vendor.as_str() {
            "GenuineIntel" => super::CpuVendor::Intel,
            "AuthenticAMD" => super::CpuVendor::Amd,
            _ => super::CpuVendor::Unknown,
        }
    } else {
        super::CpuVendor::Unknown
    }
}

pub fn detect_features() -> super::CpuFeatures {
    let cpuid = CpuId::new();
    let mut features = super::CpuFeatures::default();
    
    if let Some(f) = cpuid.get_feature_info() {
        features.has_tsc = f.has_tsc();
        features.has_sse4_2 = f.has_sse42();
    }
    features
}

pub fn count_cpus() -> u32 {
    let cpuid = CpuId::new();
    cpuid.get_feature_info()
        .map(|i| i.max_logical_processor_ids())
        .unwrap_or(1)
}
```

**Testing**: Boot, verify CPU vendor and feature detection

---

### Week 5-6: Filesystem (VFS + tmpfs)

**Create**: `kernel/src/fs/mod.rs`, `kernel/src/fs/tmpfs.rs`

```rust
// kernel/src/fs/mod.rs
#[derive(Debug)]
pub struct FileHandle(u32);

#[derive(Debug)]
pub enum FsError {
    NotFound, PermissionDenied, InvalidHandle, Full, InvalidPath, Exists,
}

pub trait FileSystem {
    fn open(&mut self, path: &str, flags: u32) -> Result<FileHandle, FsError>;
    fn close(&mut self, handle: FileHandle) -> Result<(), FsError>;
    fn read(&mut self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write(&mut self, handle: FileHandle, buf: &[u8]) -> Result<usize, FsError>;
}

static mut FILESYSTEM: Option<tmpfs::TmpFS> = None;

pub fn init() {
    unsafe {
        FILESYSTEM = Some(tmpfs::TmpFS::new());
    }
}

pub fn open(path: &str, flags: u32) -> Result<FileHandle, FsError> {
    unsafe {
        if let Some(fs) = &mut FILESYSTEM {
            fs.open(path, flags)
        } else {
            Err(FsError::InvalidHandle)
        }
    }
}

pub fn read(handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
    unsafe {
        if let Some(fs) = &mut FILESYSTEM {
            fs.read(handle, buf)
        } else {
            Err(FsError::InvalidHandle)
        }
    }
}

pub fn write(handle: FileHandle, buf: &[u8]) -> Result<usize, FsError> {
    unsafe {
        if let Some(fs) = &mut FILESYSTEM {
            fs.write(handle, buf)
        } else {
            Err(FsError::InvalidHandle)
        }
    }
}

// kernel/src/fs/tmpfs.rs
use alloc::collections::BTreeMap;
use super::{FileSystem, FileHandle, FsError};

pub struct TmpFS {
    files: BTreeMap<alloc::string::String, alloc::vec::Vec<u8>>,
    handles: BTreeMap<u32, TmpFileHandle>,
    next_handle: u32,
}

struct TmpFileHandle {
    path: alloc::string::String,
    position: u64,
}

impl TmpFS {
    pub fn new() -> Self {
        TmpFS {
            files: BTreeMap::new(),
            handles: BTreeMap::new(),
            next_handle: 0,
        }
    }
}

impl FileSystem for TmpFS {
    fn open(&mut self, path: &str, flags: u32) -> Result<FileHandle, FsError> {
        if !self.files.contains_key(path) && (flags & 1) != 0 {
            self.files.insert(path.to_string(), alloc::vec::Vec::new());
        }
        
        if !self.files.contains_key(path) {
            return Err(FsError::NotFound);
        }
        
        let handle_id = self.next_handle;
        self.next_handle += 1;
        
        self.handles.insert(handle_id, TmpFileHandle {
            path: path.to_string(),
            position: 0,
        });
        
        Ok(FileHandle(handle_id))
    }
    
    fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        self.handles.remove(&handle.0).ok_or(FsError::InvalidHandle)?;
        Ok(())
    }
    
    fn read(&mut self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
        let h = self.handles.get_mut(&handle.0).ok_or(FsError::InvalidHandle)?;
        let data = self.files.get(&h.path).ok_or(FsError::NotFound)?;
        
        let available = ((data.len() as u64 - h.position).min(buf.len() as u64)) as usize;
        if available > 0 {
            let pos = h.position as usize;
            buf[..available].copy_from_slice(&data[pos..pos + available]);
        }
        
        h.position += available as u64;
        Ok(available)
    }
    
    fn write(&mut self, handle: FileHandle, buf: &[u8]) -> Result<usize, FsError> {
        let h = self.handles.get_mut(&handle.0).ok_or(FsError::InvalidHandle)?;
        let data = self.files.get_mut(&h.path).ok_or(FsError::NotFound)?;
        
        let pos = h.position as usize;
        if pos + buf.len() > data.len() {
            data.resize(pos + buf.len(), 0);
        }
        
        data[pos..pos + buf.len()].copy_from_slice(buf);
        h.position += buf.len() as u64;
        
        Ok(buf.len())
    }
}
```

**Testing**: Create, write, read files end-to-end

---

### Week 7-8: Extended Syscalls

**Extend** `kernel/src/syscall.rs`:

```rust
pub enum Number {
    Yield = 0, Read = 1, Write = 2, Exit = 3, Getpid = 4,
    Open = 5, Close = 6, FRead = 7, FWrite = 8,
    MessageSend = 9, MessageRecv = 10,
    Fork = 11, Exec = 12, Wait = 13, Sleep = 14,
}
```

**Add syscall handlers** and create **userspace stubs**:

```rust
// userspace/src/syscall.rs
pub fn sys_open(path: &str) -> i32 {
    let fd: i32;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 5,
            in("rdi") path.as_ptr(),
            out("rax") fd,
            clobber_abi("C"),
        );
    }
    fd
}
```

**Testing**: Each syscall individually tested

---

### Week 9-10: Graphics & Framebuffer

**Create**: `kernel/src/graphics/mod.rs`, `kernel/src/graphics/rasterizer.rs`

```rust
// kernel/src/graphics/mod.rs
#[repr(C)]
pub struct Color {
    pub r: u8, pub g: u8, pub b: u8, pub a: u8,
}

pub struct Graphics {
    framebuffer: Framebuffer,
    width: u32,
    height: u32,
}

impl Graphics {
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color) {
        self.framebuffer.set_pixel(x, y, color);
    }
    
    pub fn draw_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Color) {
        // Bresenham algorithm
    }
    
    pub fn clear(&mut self, color: Color) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.set_pixel(x, y, color);
            }
        }
    }
    
    pub fn flush(&mut self) {
        // Sync to display
    }
}
```

**Testing**: Render pixels, lines, rectangles without corruption

---

### Week 11-12: GUI Framework & Desktop

**Create**: `kernel/src/gui/mod.rs`, `kernel/src/gui/widget.rs`, `kernel/src/gui/desktop.rs`

```rust
// kernel/src/gui/widget.rs
pub trait Widget {
    fn render(&self, g: &mut Graphics);
    fn on_click(&mut self, x: i32, y: i32);
}

// kernel/src/gui/desktop.rs
pub struct Desktop {
    pub windows: Vec<Window>,
    pub background_color: Color,
}

impl Desktop {
    pub fn new() -> Self {
        Desktop {
            windows: Vec::new(),
            background_color: Color { r: 128, g: 128, b: 128, a: 255 },
        }
    }
    
    pub fn render(&self, g: &mut Graphics) {
        g.clear(self.background_color);
        for window in &self.windows {
            window.render(g);
        }
        g.flush();
    }
}

pub struct Window {
    pub x: i32, pub y: i32,
    pub width: u32, pub height: u32,
    pub title: &'static str,
}

impl Window {
    pub fn render(&self, g: &mut Graphics) {
        // Draw frame
        g.draw_rect(self.x, self.y, self.width, 20, 
                   Color { r: 200, g: 200, b: 200, a: 255 }, true);
        g.draw_rect(self.x, self.y + 20, self.width, self.height - 20,
                   Color { r: 255, g: 255, b: 255, a: 255 }, true);
    }
}
```

**Testing**: Desktop boots, windows render, keyboard input works

---

## Architecture Specifications

### Syscall Table (14+)

| # | Name | Args | Purpose |
|---|------|------|---------|
| 0 | yield | - | Reschedule |
| 1-4 | read/write/exit/getpid | - | Basic |
| 5-8 | open/close/fread/fwrite | path, fd, buf | File I/O |
| 9-10 | message_send/recv | pid, msg | IPC |
| 11-13 | fork/exec/wait | path, argv | Process |
| 14 | sleep | ms | Sleep |
| 15-16 | mmap/munmap | addr, len | Memory |
| 17-18 | signal/sigaction | signum | Signals |

### Capability Model (64-bit)

```
Bit  Name              Allows
 0   Console           Framebuffer write, keyboard read
 1   TimerRead         Read timer ticks
 2   TaskInspect       Inspect task state
 3   TaskControl       Control tasks
 4   DeviceIo          I/O port access
 5   InterruptControl  Modify IDT
 6   MemoryInspect     Read other memory
 7   FileSystem        File operations
 8   NetworkIo         Network access
 9   ProcessCreate     Fork, exec
10   IpcCreate         Create queues
```

---

## Code Organization

**Post-Phase 1 Structure**:

```
wovenhat-os/
├── kernel/src/
│   ├── main.rs              boot, init
│   ├── panic.rs             panic handler
│   ├── hal/                 CPU, device abstraction
│   ├── memory.rs            frame allocator
│   ├── paging.rs            page tables
│   ├── heap.rs              kernel heap
│   ├── gdt.rs / interrupts.rs / pic.rs
│   ├── fs/                  filesystem (VFS, tmpfs)
│   ├── syscall.rs           14+ syscalls
│   ├── task.rs              scheduler
│   ├── capability.rs        capability checks
│   ├── serial.rs / keyboard.rs / timer.rs
│   ├── graphics/            framebuffer, drawing
│   ├── gui/                 widgets, windows, desktop
│   ├── ipc/                 message queues
│   └── userspace.rs         user program loader
├── userspace/src/
│   ├── syscall.rs           syscall wrappers
│   ├── stdio.rs, fs.rs
│   └── main.rs
├── docs/
│   ├── ARCHITECTURE.md
│   ├── SYSCALL-API.md
│   ├── COMPLETE-AI-DIRECTIVE.md  ← YOU ARE HERE
│   └── ROADMAP.md
└── Cargo.toml
```

---

## AI Agent Instructions

### Use This Document As:

1. **Master Blueprint** — Complete implementation guide
2. **Week-by-Week Roadmap** — What to build when
3. **Code Reference** — Rust implementation examples
4. **Testing Checklist** — Verify each component
5. **Architecture Guide** — System design specifications

### Workflow

```
Read Document
    ↓
Week 1-2: Build panic handler → test
    ↓
Week 3-4: Build HAL → test
    ↓
Week 5-6: Build filesystem → test
    ↓
Week 7-8: Build syscalls → test
    ↓
Week 9-10: Build graphics → test
    ↓
Week 11-12: Build GUI → test
    ↓
Phase 1 Complete! Move to Phase 2
```

### Code Quality Standards

Every implementation must:
- [ ] Compile without warnings
- [ ] Have unit tests
- [ ] Pass existing tests
- [ ] Update documentation
- [ ] Follow Rust idioms
- [ ] No unsafe outside HAL/syscall

### Build & Test Commands

```bash
# Build kernel
cargo build --release

# Run in QEMU
qemu-system-x86_64 -drive format=raw,file=kernel.bin -serial stdio

# Check for leaks
valgrind ./kernel

# Profile performance
perf record -g ./kernel
```

---

## Version History

| Ver | Date | Status |
|-----|------|--------|
| 1.0 | 2026-09-02 | Initial release |

---

**This is your complete AI directive. Follow it systematically to build WovenHat OS v2.0.0** 🚀

