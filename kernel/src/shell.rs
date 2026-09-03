use crate::{
    benchmark, capability::Capability, console::Console, heap, keyboard::Key, memory, paging,
    storage, syscall, task, timer, userspace, vfs,
};

const PROMPT: &str = "WOVENHAT> ";
const COMMAND_CAPACITY: usize = 128;

/// Kernel shell working directory (independent of userspace process cwd).
static mut SHELL_CWD: [u8; 128] = [b'/', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
static mut SHELL_CWD_LEN: usize = 1;

pub struct Shell {
    command: [u8; COMMAND_CAPACITY],
    length: usize,
}

impl Shell {
    pub const fn new() -> Self {
        Self {
            command: [0; COMMAND_CAPACITY],
            length: 0,
        }
    }

    pub fn print_prompt(&self, console: &mut Console<'_>) {
        console.print(PROMPT);
    }

    pub fn handle_key(&mut self, key: Key, console: &mut Console<'_>) {
        match key {
            Key::Char(character) => self.push_character(character, console),
            Key::Backspace => self.backspace(console),
            Key::Enter => self.submit(console),
            Key::Tab | Key::F1 => {}
        }
    }

    fn push_character(&mut self, character: char, console: &mut Console<'_>) {
        if !character.is_ascii() || character.is_ascii_control() {
            return;
        }

        if self.length == self.command.len() {
            return;
        }

        self.command[self.length] = character as u8;
        self.length += 1;
        console.put_char(character);
    }

    fn backspace(&mut self, console: &mut Console<'_>) {
        if self.length == 0 {
            return;
        }

        self.length -= 1;
        console.backspace();
    }

    fn submit(&mut self, console: &mut Console<'_>) {
        console.newline();

        let command = core::str::from_utf8(&self.command[..self.length])
            .unwrap_or("")
            .trim();

        let (verb, arg) = split_command(command);

        match verb {
            "" => {}
            "help" => {
                console.println("COMMANDS: HELP CLEAR VERSION TICKS TASKS CAPS MEMORY PAGING HEAP BENCH FS LS CAT INIT SPAWN SYSCALL USER RING3");
                console.println("  LS [PATH]   list directory (default /)");
                console.println("  CAT <PATH>  print file (loads /mnt/... from disk on demand)");
                console.println("  INIT        spawn userspace init process");
                console.println("  CD [PATH]   change directory");
                console.println("  PWD         print working directory");
            }
            "clear" => {
                if !authorize(Capability::Console, console) {
                    self.finish(console);
                    return;
                }

                console.clear();
            }
            "version" => {
                console.println("WOVENHAT KERNEL 0.0.7");
            }
            "ticks" => {
                if !authorize(Capability::TimerRead, console) {
                    self.finish(console);
                    return;
                }

                console.print("TIMER TICKS: ");
                print_u64(console, timer::ticks());
                console.newline();
            }
            "tasks" => {
                if !authorize(Capability::TaskInspect, console) {
                    self.finish(console);
                    return;
                }

                let summary = task::summary();
                console.print("TASKS: ");
                print_u64(console, summary.task_count as u64);
                console.print(" PROCESSES: ");
                print_u64(console, task::process_count() as u64);
                console.print(" READY: ");
                print_u64(console, summary.ready_tasks as u64);
                console.print(" BLOCKED: ");
                print_u64(console, summary.blocked_tasks as u64);
                console.print(" CURRENT: ");
                print_u64(console, summary.current_id.as_u64());
                console.print(" ");
                console.print(summary.current_name);
                console.print(" STATE: ");
                console.print(summary.current_state);
                console.print(" PRIORITY: ");
                print_u64(console, summary.current_priority as u64);
                console.print(" SWITCHES: ");
                print_u64(console, summary.context_switches);
                console.print(" PREEMPTIONS: ");
                print_u64(console, summary.preemption_switches);
                console.print(" IDLE: ");
                print_u64(console, summary.idle_heartbeats);
                console.newline();
            }
            "caps" => {
                console.print("CAPS:");
                print_capability(console, Capability::Console, " CONSOLE");
                print_capability(console, Capability::TimerRead, " TIMER_READ");
                print_capability(console, Capability::TaskInspect, " TASK_INSPECT");
                print_capability(console, Capability::TaskControl, " TASK_CONTROL");
                print_capability(console, Capability::DeviceIo, " DEVICE_IO");
                print_capability(console, Capability::InterruptControl, " INTERRUPT_CONTROL");
                print_capability(console, Capability::MemoryInspect, " MEMORY_INSPECT");
                print_capability(console, Capability::FileRead, " FILE_READ");
                print_capability(console, Capability::FileWrite, " FILE_WRITE");
                print_capability(console, Capability::Ipc, " IPC");
                print_capability(console, Capability::ProcessCreate, " PROCESS_CREATE");
                console.newline();
            }
            "memory" => {
                if !authorize(Capability::MemoryInspect, console) {
                    self.finish(console);
                    return;
                }

                let stats = memory::stats();
                console.print("MEMORY REGIONS: ");
                print_u64(console, stats.usable_regions as u64);
                console.print(" FRAMES: ");
                print_u64(console, stats.total_frames);
                console.print(" USED: ");
                print_u64(console, stats.allocated_frames);
                console.print(" FREE: ");
                print_u64(console, stats.remaining_frames);
                console.newline();
            }
            "heap" => {
                if !authorize(Capability::MemoryInspect, console) {
                    self.finish(console);
                    return;
                }

                let stats = heap::stats();
                console.print("HEAP: START: ");
                print_hex_u64(console, stats.start);
                console.print(" SIZE: ");
                print_u64(console, stats.size as u64);
                console.print(" USED: ");
                print_u64(console, stats.allocated_bytes as u64);
                console.print(" ALLOCATIONS: ");
                print_u64(console, stats.allocations as u64);
                console.newline();
            }
            "bench" => {
                if !authorize(Capability::TaskInspect, console)
                    || !authorize(Capability::MemoryInspect, console)
                {
                    self.finish(console);
                    return;
                }
                let delta = benchmark::sample();
                if !delta.baseline_ready {
                    console.println("BENCHMARK BASELINE CAPTURED");
                } else {
                    console.print("BENCH TICKS: ");
                    print_u64(console, delta.ticks);
                    console.print(" SWITCHES: ");
                    print_u64(console, delta.context_switches);
                    console.print(" PREEMPTIONS: ");
                    print_u64(console, delta.preemptions);
                    console.print(" IDLE: ");
                    print_u64(console, delta.idle_heartbeats);
                    console.print(" FRAMES: ");
                    print_i64(console, delta.frame_change);
                    console.print(" HEAP_BYTES: ");
                    print_i64(console, delta.heap_byte_change);
                    console.print(" ALLOCS: ");
                    print_u64(console, delta.heap_allocations);
                    console.newline();
                }
            }
            "fs" => {
                if !authorize(Capability::FileRead, console) {
                    self.finish(console);
                    return;
                }
                console.print("VFS NODES: ");
                print_u64(console, vfs::node_count() as u64);
                console.print(" PROCESS OPEN FILES: ");
                print_u64(console, task::open_file_count() as u64);
                console.newline();
            }
            "ls" => {
                if !authorize(Capability::FileRead, console) {
                    self.finish(console);
                    return;
                }
                let path = if arg.is_empty() { "/" } else { arg };
                run_ls(path, console);
            }
            "cat" => {
                if !authorize(Capability::FileRead, console) {
                    self.finish(console);
                    return;
                }
                let path = if arg.is_empty() { "/etc/motd" } else { arg };
                run_cat(path, console);
            }
            "init" => {
                if !authorize(Capability::TaskControl, console) {
                    self.finish(console);
                    return;
                }
                run_init(console);
            }
            "cd" => {
                if !authorize(Capability::FileRead, console) {
                    self.finish(console);
                    return;
                }
                let path = if arg.is_empty() { "/" } else { arg };
                if !shell_chdir(path) {
                    console.println("CD FAILED");
                }
            }
            "pwd" => {
                if !authorize(Capability::FileRead, console) {
                    self.finish(console);
                    return;
                }
                console.println(shell_cwd());
            }
            "spawn" => {
                if !authorize(Capability::TaskControl, console) {
                    self.finish(console);
                    return;
                }

                match task::spawn("demo", demo_task) {
                    Ok(id) => {
                        console.print("TASK SPAWNED: ");
                        print_u64(console, id.as_u64());
                        console.newline();
                    }
                    Err(_) => {
                        console.println("TASK SPAWN FAILED: SCHEDULER FULL");
                    }
                }
            }
            "syscall" => {
                if !authorize(Capability::InterruptControl, console) {
                    self.finish(console);
                    return;
                }

                console.println("TRIGGERING SYSCALL 0x80");
                if syscall::test() {
                    console.println("SYSCALL HANDLER: OK");
                } else {
                    console.println("SYSCALL HANDLER: FAILED");
                }
            }
            "user" | "ring3" => {
                if !authorize(Capability::TaskControl, console) {
                    self.finish(console);
                    return;
                }

                let Some(program) = userspace::create_stub_process() else {
                    console.println("USER IMAGE MAPPING FAILED");
                    self.finish(console);
                    return;
                };

                match task::spawn_user_process("usermode", program) {
                    Ok((id, context)) => {
                        console.print("USER PROCESS SCHEDULED: PID=");
                        print_u64(console, id.as_u64());
                        console.print(" ENTRY=");
                        print_hex_u64(console, context.entry);
                        console.print(" STACK=");
                        print_hex_u64(console, context.stack_top);
                        console.newline();
                    }
                    Err(_) => console.println("USER PROCESS SPAWN FAILED: CAPACITY EXHAUSTED"),
                }
            }
            "paging" => {
                if !authorize(Capability::MemoryInspect, console) {
                    self.finish(console);
                    return;
                }

                let stats = paging::stats();
                console.print("PAGING: ");
                print_u64(console, stats.successful_translations as u64);
                console.print("/");
                print_u64(console, stats.tested_translations as u64);
                console.print(" L4: ");
                print_hex_u64(console, stats.level_4_frame);
                console.print(" OFFSET: ");
                print_hex_u64(console, stats.physical_memory_offset);
                console.print(" MAP: ");
                console.print(if stats.mapping_test_passed {
                    "OK"
                } else {
                    "FAILED"
                });
                console.newline();
            }
            _ => {
                console.print("UNKNOWN COMMAND: ");
                console.println(verb);
            }
        }

        self.finish(console);
    }

    fn finish(&mut self, console: &mut Console<'_>) {
        self.length = 0;
        self.print_prompt(console);
    }
}



fn shell_cwd() -> &'static str {
    // SAFETY: kernel shell is single-threaded at the console.
    unsafe {
        core::str::from_utf8(&SHELL_CWD[..SHELL_CWD_LEN]).unwrap_or("/")
    }
}

fn shell_resolve(path: &str) -> Option<alloc::string::String> {
    let absolute = if path.starts_with('/') {
        alloc::string::String::from(path)
    } else {
        let cwd = shell_cwd();
        let mut joined = alloc::string::String::new();
        if cwd == "/" {
            joined.push('/');
            joined.push_str(path);
        } else {
            joined.push_str(cwd);
            joined.push('/');
            joined.push_str(path);
        }
        joined
    };
    // Normalize . and ..
    let mut stack = alloc::vec::Vec::new();
    for component in absolute.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            let _ = stack.pop();
            continue;
        }
        stack.push(component);
    }
    if stack.is_empty() {
        return Some(alloc::string::String::from("/"));
    }
    let mut out = alloc::string::String::from("/");
    for (i, part) in stack.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(part);
    }
    Some(out)
}

fn shell_chdir(path: &str) -> bool {
    let Some(absolute) = shell_resolve(path) else {
        return false;
    };
    match vfs::stat(&absolute) {
        Ok(stat) if stat.kind == vfs::NodeKind::Directory => {}
        _ => return false,
    }
    let bytes = absolute.as_bytes();
    if bytes.len() >= 128 {
        return false;
    }
    unsafe {
        SHELL_CWD = [0; 128];
        SHELL_CWD[..bytes.len()].copy_from_slice(bytes);
        SHELL_CWD_LEN = bytes.len();
    }
    true
}

fn split_command(command: &str) -> (&str, &str) {
    let command = command.trim();
    if command.is_empty() {
        return ("", "");
    }
    match command.find(char::is_whitespace) {
        Some(idx) => {
            let verb = &command[..idx];
            let arg = command[idx..].trim_start();
            (verb, arg)
        }
        None => (command, ""),
    }
}

fn run_ls(path: &str, console: &mut Console<'_>) {
    let Some(path) = shell_resolve(path) else {
        console.println("BAD PATH");
        return;
    };
    let path = path.as_str();
    match vfs::stat(path) {
        Ok(stat) if stat.kind == vfs::NodeKind::Directory => {}
        Ok(_) => {
            console.println("NOT A DIRECTORY");
            return;
        }
        Err(_) => {
            console.println("PATH NOT FOUND");
            return;
        }
    }
    let mut index = 0usize;
    loop {
        match vfs::readdir(path, index) {
            Ok(entry) => {
                console.print(if entry.kind == vfs::NodeKind::Directory {
                    "d "
                } else {
                    "f "
                });
                console.println(entry.name_str());
                index += 1;
            }
            Err(_) => break,
        }
        if index > 64 {
            break;
        }
    }
    if index == 0 {
        console.println("(empty)");
    }
}

fn run_cat(path: &str, console: &mut Console<'_>) {
    let Some(path) = shell_resolve(path) else {
        console.println("BAD PATH");
        return;
    };
    let path = path.as_str();
    // On-demand import for /mnt/... paths not yet in the VFS.
    if path.starts_with("/mnt/") {
        if let Err(err) = storage::ensure_path(path) {
            match err {
                storage::EnsureError::NotFound => console.println("FILE NOT FOUND ON VOLUME"),
                storage::EnsureError::NoDevice => console.println("NO BLOCK DEVICE"),
                storage::EnsureError::NotFat32 => console.println("NOT A FAT32 VOLUME"),
                storage::EnsureError::TooLarge => console.println("FILE TOO LARGE FOR VFS"),
                _ => console.println("MOUNT LOOKUP FAILED"),
            }
            return;
        }
    }
    let Ok(file) = vfs::open(path) else {
        console.println("VFS OPEN FAILED");
        return;
    };
    let mut buffer = [0_u8; 256];
    match vfs::read(file, &mut buffer) {
        Ok(0) => console.println("(empty)"),
        Ok(length) => match core::str::from_utf8(&buffer[..length]) {
            Ok(text) => {
                console.print(text);
                if !text.ends_with('\n') {
                    console.newline();
                }
            }
            Err(_) => console.println("VFS DATA IS NOT UTF-8"),
        },
        Err(_) => console.println("VFS READ FAILED"),
    }
    let _ = vfs::close_open_file(file);
}

fn run_init(console: &mut Console<'_>) {
    let Some(program) = userspace::create_init_process() else {
        console.println("INIT IMAGE FAILED");
        return;
    };
    match task::spawn_user_process("init", program) {
        Ok((id, context)) => {
            console.print("INIT SCHEDULED: PID=");
            print_u64(console, id.as_u64());
            console.print(" ENTRY=");
            print_hex_u64(console, context.entry);
            console.print(" STACK=");
            print_hex_u64(console, context.stack_top);
            console.newline();
        }
        Err(_) => console.println("INIT SPAWN FAILED"),
    }
}

fn demo_task() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn authorize(capability: Capability, console: &mut Console<'_>) -> bool {
    if task::current_has(capability) {
        return true;
    }

    console.println("ACCESS DENIED: MISSING CAPABILITY");
    false
}

fn print_capability(console: &mut Console<'_>, capability: Capability, name: &str) {
    if task::current_has(capability) {
        console.print(name);
    }
}

fn print_u64(console: &mut Console<'_>, mut value: u64) {
    let mut digits = [0u8; 20];
    let mut index = digits.len();

    if value == 0 {
        console.put_char('0');
        return;
    }

    while value != 0 {
        index -= 1;
        digits[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }

    for digit in &digits[index..] {
        console.put_char(*digit as char);
    }
}

fn print_i64(console: &mut Console<'_>, value: i64) {
    if value < 0 {
        console.put_char('-');
        print_u64(console, value.unsigned_abs());
    } else {
        print_u64(console, value as u64);
    }
}

fn print_hex_u64(console: &mut Console<'_>, value: u64) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    console.print("0X");
    let mut started = false;
    for shift in (0..16).rev() {
        let digit = ((value >> (shift * 4)) & 0xf) as usize;
        if digit != 0 || started || shift == 0 {
            started = true;
            console.put_char(HEX[digit] as char);
        }
    }
}
