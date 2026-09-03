//! Kernel debug shell — interactive console attached to the boot task.
//!
//! This is not a userspace shell. It runs with kernel privileges (subject to
//! the current task's capability set) and operates directly on the VFS,
//! scheduler, and hardware status helpers.

use crate::{
    benchmark, capability::Capability, console::Console, heap, keyboard::Key, memory, paging,
    storage, syscall, task, timer, userspace, vfs,
};

const PROMPT_PREFIX: &str = "wovenhat:";
const COMMAND_CAPACITY: usize = 128;
const CWD_CAPACITY: usize = 128;

/// Kernel shell working directory (independent of userspace process cwd).
struct ShellState {
    cwd: [u8; CWD_CAPACITY],
    cwd_len: usize,
}

impl ShellState {
    const fn new() -> Self {
        let mut cwd = [0u8; CWD_CAPACITY];
        cwd[0] = b'/';
        Self { cwd, cwd_len: 1 }
    }

    fn cwd_str(&self) -> &str {
        core::str::from_utf8(&self.cwd[..self.cwd_len]).unwrap_or("/")
    }

    fn set_cwd(&mut self, path: &str) -> bool {
        let bytes = path.as_bytes();
        if bytes.is_empty() || bytes.len() >= CWD_CAPACITY {
            return false;
        }
        self.cwd = [0; CWD_CAPACITY];
        self.cwd[..bytes.len()].copy_from_slice(bytes);
        self.cwd_len = bytes.len();
        true
    }
}

static mut STATE: ShellState = ShellState::new();

fn state() -> &'static mut ShellState {
    // SAFETY: console shell is driven from a single kernel task.
    unsafe { &mut *core::ptr::addr_of_mut!(STATE) }
}

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
        console.print(PROMPT_PREFIX);
        console.print(state().cwd_str());
        console.print("> ");
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

        let line = core::str::from_utf8(&self.command[..self.length])
            .unwrap_or("")
            .trim();
        let (verb, arg) = split_command(line);
        // Accept both lower and UPPER for convenience at the serial console.
        let mut verb_buf = [0u8; 32];
        let verb = lowercase(verb, &mut verb_buf);

        match verb {
            "" => {}
            "help" | "?" => print_help(console),
            "clear" => {
                if authorize(Capability::Console, console) {
                    console.clear();
                }
            }
            "version" | "ver" => console.println("WovenHat kernel 0.0.8"),
            "ticks" | "uptime" => {
                if authorize(Capability::TimerRead, console) {
                    console.print("ticks: ");
                    print_u64(console, timer::ticks());
                    console.newline();
                }
            }
            "tasks" | "ps" => {
                if authorize(Capability::TaskInspect, console) {
                    cmd_tasks(console);
                }
            }
            "caps" => cmd_caps(console),
            "memory" | "mem" => {
                if authorize(Capability::MemoryInspect, console) {
                    cmd_memory(console);
                }
            }
            "heap" => {
                if authorize(Capability::MemoryInspect, console) {
                    cmd_heap(console);
                }
            }
            "paging" => {
                if authorize(Capability::MemoryInspect, console) {
                    cmd_paging(console);
                }
            }
            "bench" => {
                if authorize(Capability::TaskInspect, console)
                    && authorize(Capability::MemoryInspect, console)
                {
                    cmd_bench(console);
                }
            }
            "fs" => {
                if authorize(Capability::FileRead, console) {
                    cmd_fs(console);
                }
            }
            "ls" => {
                if authorize(Capability::FileRead, console) {
                    let path = if arg.is_empty() {
                        state().cwd_str()
                    } else {
                        arg
                    };
                    cmd_ls(path, console);
                }
            }
            "cat" => {
                if authorize(Capability::FileRead, console) {
                    if arg.is_empty() {
                        console.println("usage: cat <path>");
                    } else {
                        cmd_cat(arg, console);
                    }
                }
            }
            "write" => {
                if authorize(Capability::FileWrite, console) {
                    cmd_write(arg, console);
                }
            }
            "rm" | "remove" => {
                if authorize(Capability::FileWrite, console) {
                    if arg.is_empty() {
                        console.println("usage: rm <path>");
                    } else {
                        cmd_rm(arg, console);
                    }
                }
            }
            "mkdir" => {
                if authorize(Capability::FileWrite, console) {
                    if arg.is_empty() {
                        console.println("usage: mkdir <path>");
                    } else {
                        cmd_mkdir(arg, console);
                    }
                }
            }
            "stat" => {
                if authorize(Capability::FileRead, console) {
                    if arg.is_empty() {
                        console.println("usage: stat <path>");
                    } else {
                        cmd_stat(arg, console);
                    }
                }
            }
            "echo" => {
                if arg.is_empty() {
                    console.newline();
                } else {
                    console.println(arg);
                }
            }
            "cd" => {
                if authorize(Capability::FileRead, console) {
                    let path = if arg.is_empty() { "/" } else { arg };
                    if !shell_chdir(path) {
                        console.println("cd: failed");
                    }
                }
            }
            "pwd" => {
                if authorize(Capability::FileRead, console) {
                    console.println(state().cwd_str());
                }
            }
            "run" => {
                if authorize(Capability::TaskControl, console)
                    && authorize(Capability::ProcessCreate, console)
                {
                    if arg.is_empty() {
                        console.println("usage: run <path>");
                    } else {
                        cmd_run(arg, console);
                    }
                }
            }
            "sh" => {
                if authorize(Capability::TaskControl, console) {
                    cmd_sh(console);
                }
            }
            "init" => {
                if authorize(Capability::TaskControl, console) {
                    cmd_init(console);
                }
            }
            "spawn" => {
                if authorize(Capability::TaskControl, console) {
                    match task::spawn("demo", demo_task) {
                        Ok(id) => {
                            console.print("task spawned: ");
                            print_u64(console, id.as_u64());
                            console.newline();
                        }
                        Err(_) => console.println("spawn failed: scheduler full"),
                    }
                }
            }
            "syscall" => {
                if authorize(Capability::InterruptControl, console) {
                    console.println("triggering syscall 0x80");
                    if syscall::test() {
                        console.println("syscall handler: ok");
                    } else {
                        console.println("syscall handler: failed");
                    }
                }
            }
            "user" | "ring3" => {
                if authorize(Capability::TaskControl, console) {
                    cmd_user(console);
                }
            }
            "mount" => {
                if authorize(Capability::FileRead, console) {
                    cmd_mount(console);
                }
            }
            _ => {
                console.print("unknown command: ");
                console.println(verb);
                console.println("type 'help' for a list");
            }
        }

        self.finish(console);
    }

    fn finish(&mut self, console: &mut Console<'_>) {
        self.length = 0;
        self.print_prompt(console);
    }
}

fn print_help(console: &mut Console<'_>) {
    console.println("WovenHat kernel shell 0.0.8");
    console.println("system:  help clear version ticks|uptime tasks|ps caps");
    console.println("         memory|mem heap paging bench fs mount syscall");
    console.println("files:   ls [path]  cat <path>  write <path> <text>");
    console.println("         mkdir <path>  rm <path>  stat <path>");
    console.println("nav:     cd [path]  pwd  echo <text>");
    console.println("process: run <elf>  sh  init  spawn  user|ring3");
}

fn cmd_tasks(console: &mut Console<'_>) {
    let summary = task::summary();
    console.print("tasks: ");
    print_u64(console, summary.task_count as u64);
    console.print("  processes: ");
    print_u64(console, task::process_count() as u64);
    console.print("  ready: ");
    print_u64(console, summary.ready_tasks as u64);
    console.print("  blocked: ");
    print_u64(console, summary.blocked_tasks as u64);
    console.newline();
    console.print("current: ");
    print_u64(console, summary.current_id.as_u64());
    console.print(" ");
    console.print(summary.current_name);
    console.print("  state: ");
    console.print(summary.current_state);
    console.print("  priority: ");
    print_u64(console, summary.current_priority as u64);
    console.newline();
    console.print("switches: ");
    print_u64(console, summary.context_switches);
    console.print("  preemptions: ");
    print_u64(console, summary.preemption_switches);
    console.print("  idle: ");
    print_u64(console, summary.idle_heartbeats);
    console.newline();
}

fn cmd_caps(console: &mut Console<'_>) {
    console.print("caps:");
    print_capability(console, Capability::Console, " console");
    print_capability(console, Capability::TimerRead, " timer");
    print_capability(console, Capability::TaskInspect, " task_inspect");
    print_capability(console, Capability::TaskControl, " task_control");
    print_capability(console, Capability::DeviceIo, " device_io");
    print_capability(console, Capability::InterruptControl, " irq");
    print_capability(console, Capability::MemoryInspect, " memory");
    print_capability(console, Capability::FileRead, " file_read");
    print_capability(console, Capability::FileWrite, " file_write");
    print_capability(console, Capability::Ipc, " ipc");
    print_capability(console, Capability::ProcessCreate, " process_create");
    console.newline();
}

fn cmd_memory(console: &mut Console<'_>) {
    let stats = memory::stats();
    console.print("memory regions: ");
    print_u64(console, stats.usable_regions as u64);
    console.print("  frames: ");
    print_u64(console, stats.total_frames);
    console.print("  used: ");
    print_u64(console, stats.allocated_frames);
    console.print("  free: ");
    print_u64(console, stats.remaining_frames);
    console.newline();
}

fn cmd_heap(console: &mut Console<'_>) {
    let stats = heap::stats();
    console.print("heap start: ");
    print_hex_u64(console, stats.start);
    console.print("  size: ");
    print_u64(console, stats.size as u64);
    console.print("  used: ");
    print_u64(console, stats.allocated_bytes as u64);
    console.print("  allocs: ");
    print_u64(console, stats.allocations as u64);
    console.newline();
}

fn cmd_paging(console: &mut Console<'_>) {
    let stats = paging::stats();
    console.print("paging: ");
    print_u64(console, stats.successful_translations as u64);
    console.print("/");
    print_u64(console, stats.tested_translations as u64);
    console.print("  l4: ");
    print_hex_u64(console, stats.level_4_frame);
    console.print("  offset: ");
    print_hex_u64(console, stats.physical_memory_offset);
    console.print("  map: ");
    console.println(if stats.mapping_test_passed {
        "ok"
    } else {
        "failed"
    });
}

fn cmd_bench(console: &mut Console<'_>) {
    let delta = benchmark::sample();
    if !delta.baseline_ready {
        console.println("benchmark baseline captured");
        return;
    }
    console.print("bench ticks: ");
    print_u64(console, delta.ticks);
    console.print("  switches: ");
    print_u64(console, delta.context_switches);
    console.print("  preemptions: ");
    print_u64(console, delta.preemptions);
    console.print("  idle: ");
    print_u64(console, delta.idle_heartbeats);
    console.print("  frames: ");
    print_i64(console, delta.frame_change);
    console.print("  heap_bytes: ");
    print_i64(console, delta.heap_byte_change);
    console.print("  allocs: ");
    print_u64(console, delta.heap_allocations);
    console.newline();
}

fn cmd_fs(console: &mut Console<'_>) {
    console.print("vfs nodes: ");
    print_u64(console, vfs::node_count() as u64);
    console.print("  open-file descriptions: ");
    print_u64(console, vfs::open_file_description_count() as u64);
    console.print("  process fds: ");
    print_u64(console, task::open_file_count() as u64);
    console.newline();
}

fn cmd_mount(console: &mut Console<'_>) {
    console.println("storage: ATA primary master (if present)");
    console.println("boot mounts FAT32 root into /mnt (read-only import)");
    console.println("on-demand: cat/stat/run of /mnt/... pulls missing paths");
    console.print("vfs nodes under /mnt: ");
    let mut count = 0usize;
    let mut index = 0usize;
    while let Ok(entry) = vfs::readdir("/mnt", index) {
        let _ = entry;
        count += 1;
        index += 1;
        if index > 64 {
            break;
        }
    }
    print_u64(console, count as u64);
    console.newline();
}

fn cmd_ls(path: &str, console: &mut Console<'_>) {
    let Some(path) = shell_resolve(path) else {
        console.println("ls: bad path");
        return;
    };
    // Pull directory listing for /mnt if needed.
    if path == "/mnt" || path.starts_with("/mnt/") {
        let _ = storage::ensure_path(&path);
    }
    match vfs::stat(&path) {
        Ok(stat) if stat.kind == vfs::NodeKind::Directory => {}
        Ok(_) => {
            // Listing a file: show its name only.
            console.print("f ");
            if let Some(name) = path.rsplit('/').next() {
                console.println(if name.is_empty() { path.as_str() } else { name });
            }
            return;
        }
        Err(_) => {
            console.println("ls: not found");
            return;
        }
    }
    let mut index = 0usize;
    let mut any = false;
    while let Ok(entry) = vfs::readdir(&path, index) {
        any = true;
        console.print(match entry.kind {
            vfs::NodeKind::Directory => "d ",
            vfs::NodeKind::File => "f ",
        });
        console.println(entry.name_str());
        index += 1;
        if index > 128 {
            break;
        }
    }
    if !any {
        console.println("(empty)");
    }
}

fn cmd_cat(path: &str, console: &mut Console<'_>) {
    let Some(path) = shell_resolve(path) else {
        console.println("cat: bad path");
        return;
    };
    if path.starts_with("/mnt/") {
        if let Err(err) = storage::ensure_path(&path) {
            console.println(match err {
                storage::EnsureError::NotFound => "cat: not found on volume",
                storage::EnsureError::NoDevice => "cat: no block device",
                storage::EnsureError::NotFat32 => "cat: not a fat32 volume",
                storage::EnsureError::TooLarge => "cat: file too large",
                _ => "cat: mount lookup failed",
            });
            return;
        }
    }
    let Ok(file) = vfs::open(&path) else {
        console.println("cat: open failed");
        return;
    };
    let mut buffer = [0u8; 512];
    let mut total = 0usize;
    loop {
        match vfs::read(file, &mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                total += length;
                match core::str::from_utf8(&buffer[..length]) {
                    Ok(text) => console.print(text),
                    Err(_) => {
                        console.println("\ncat: binary or invalid utf-8");
                        break;
                    }
                }
            }
            Err(_) => {
                console.println("\ncat: read failed");
                break;
            }
        }
    }
    if total == 0 {
        console.println("(empty)");
    } else {
        // Ensure trailing newline for tidy prompt.
        // (best-effort; we may not know last char)
        console.newline();
    }
    let _ = vfs::close_open_file(file);
}

fn cmd_write(arg: &str, console: &mut Console<'_>) {
    // write <path> <text...>
    let arg = arg.trim();
    if arg.is_empty() {
        console.println("usage: write <path> <text>");
        return;
    }
    let (path_part, text) = split_command(arg);
    if path_part.is_empty() {
        console.println("usage: write <path> <text>");
        return;
    }
    let Some(path) = shell_resolve(path_part) else {
        console.println("write: bad path");
        return;
    };
    match vfs::write_file(&path, text.as_bytes()) {
        Ok(()) => {
            console.print("wrote ");
            print_u64(console, text.len() as u64);
            console.println(" bytes");
        }
        Err(vfs::Error::ReadOnly) => console.println("write: read-only"),
        Err(vfs::Error::Full) => console.println("write: full or too large"),
        Err(vfs::Error::NotFound) => console.println("write: parent missing"),
        Err(vfs::Error::AlreadyExists) => console.println("write: path is a directory"),
        Err(_) => console.println("write: failed"),
    }
}

fn cmd_rm(path: &str, console: &mut Console<'_>) {
    let Some(path) = shell_resolve(path) else {
        console.println("rm: bad path");
        return;
    };
    match vfs::remove(&path) {
        Ok(()) => console.println("removed"),
        Err(vfs::Error::NotFound) => console.println("rm: not found"),
        Err(vfs::Error::ReadOnly) => console.println("rm: refused"),
        Err(vfs::Error::Full) => console.println("rm: directory not empty"),
        Err(_) => console.println("rm: failed"),
    }
}

fn cmd_mkdir(path: &str, console: &mut Console<'_>) {
    let Some(path) = shell_resolve(path) else {
        console.println("mkdir: bad path");
        return;
    };
    match vfs::mkdir(&path) {
        Ok(()) => console.println("ok"),
        Err(vfs::Error::AlreadyExists) => console.println("mkdir: exists"),
        Err(vfs::Error::NotFound) => console.println("mkdir: parent missing"),
        Err(vfs::Error::Full) => console.println("mkdir: vfs full"),
        Err(_) => console.println("mkdir: failed"),
    }
}

fn cmd_stat(path: &str, console: &mut Console<'_>) {
    let Some(path) = shell_resolve(path) else {
        console.println("stat: bad path");
        return;
    };
    if path.starts_with("/mnt/") {
        let _ = storage::ensure_path(&path);
    }
    match vfs::stat(&path) {
        Ok(stat) => {
            console.print("path: ");
            console.println(&path);
            console.print("kind: ");
            console.println(match stat.kind {
                vfs::NodeKind::File => "file",
                vfs::NodeKind::Directory => "directory",
            });
            console.print("size: ");
            print_u64(console, stat.size as u64);
            console.newline();
            console.print("writable: ");
            console.println(if stat.writable { "yes" } else { "no" });
        }
        Err(_) => console.println("stat: not found"),
    }
}

fn cmd_run(path: &str, console: &mut Console<'_>) {
    let Some(path) = shell_resolve(path) else {
        console.println("run: bad path");
        return;
    };
    if path.starts_with("/mnt/") {
        if storage::ensure_path(&path).is_err() {
            console.println("run: load from disk failed");
            return;
        }
    }
    let mut image = [0u8; vfs::NODE_CAPACITY];
    let Ok(len) = vfs::read_all(&path, &mut image) else {
        console.println("run: read failed");
        return;
    };
    let Some(program) = userspace::load_elf_with_argv(&image[..len], &[path.as_str()]) else {
        console.println("run: elf load failed");
        return;
    };
    match task::spawn_user_process("run", program) {
        Ok((id, context)) => {
            console.print("running pid=");
            print_u64(console, id.as_u64());
            console.print(" entry=");
            print_hex_u64(console, context.entry);
            console.newline();
        }
        Err(_) => console.println("run: spawn failed"),
    }
}

fn cmd_sh(console: &mut Console<'_>) {
    let Some(program) = userspace::create_shell_process() else {
        console.println("sh: image failed");
        return;
    };
    match task::spawn_user_process("sh", program) {
        Ok((id, context)) => {
            console.print("userspace sh pid=");
            print_u64(console, id.as_u64());
            console.print(" entry=");
            print_hex_u64(console, context.entry);
            console.newline();
            console.println("(keyboard input goes to the active userspace task)");
        }
        Err(_) => console.println("sh: spawn failed"),
    }
}

fn cmd_init(console: &mut Console<'_>) {
    let Some(program) = userspace::create_init_process() else {
        console.println("init: image failed");
        return;
    };
    match task::spawn_user_process("init", program) {
        Ok((id, context)) => {
            console.print("init pid=");
            print_u64(console, id.as_u64());
            console.print(" entry=");
            print_hex_u64(console, context.entry);
            console.print(" stack=");
            print_hex_u64(console, context.stack_top);
            console.newline();
        }
        Err(_) => console.println("init: spawn failed"),
    }
}

fn cmd_user(console: &mut Console<'_>) {
    let Some(program) = userspace::create_stub_process() else {
        console.println("user: image mapping failed");
        return;
    };
    match task::spawn_user_process("usermode", program) {
        Ok((id, context)) => {
            console.print("user pid=");
            print_u64(console, id.as_u64());
            console.print(" entry=");
            print_hex_u64(console, context.entry);
            console.print(" stack=");
            print_hex_u64(console, context.stack_top);
            console.newline();
        }
        Err(_) => console.println("user: spawn failed"),
    }
}

fn shell_resolve(path: &str) -> Option<alloc::string::String> {
    let absolute = if path.starts_with('/') {
        alloc::string::String::from(path)
    } else {
        let cwd = state().cwd_str();
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
    if absolute == "/mnt" || absolute.starts_with("/mnt/") {
        let _ = storage::ensure_path(&absolute);
    }
    match vfs::stat(&absolute) {
        Ok(stat) if stat.kind == vfs::NodeKind::Directory => state().set_cwd(&absolute),
        _ => false,
    }
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

fn lowercase<'a>(verb: &str, buf: &'a mut [u8; 32]) -> &'a str {
    if verb.len() > buf.len() {
        return verb;
    }
    for (i, b) in verb.bytes().enumerate() {
        buf[i] = if (b'A'..=b'Z').contains(&b) {
            b + 32
        } else {
            b
        };
    }
    core::str::from_utf8(&buf[..verb.len()]).unwrap_or(verb)
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
    console.println("permission denied");
    false
}

fn print_capability(console: &mut Console<'_>, capability: Capability, name: &str) {
    if task::current_has(capability) {
        console.print(name);
    }
}

fn print_u64(console: &mut Console<'_>, mut value: u64) {
    if value == 0 {
        console.print("0");
        return;
    }
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    while value > 0 {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        console.put_char(digits[len] as char);
    }
}

fn print_i64(console: &mut Console<'_>, value: i64) {
    if value < 0 {
        console.print("-");
        print_u64(console, value.unsigned_abs());
    } else {
        print_u64(console, value as u64);
    }
}

fn print_hex_u64(console: &mut Console<'_>, value: u64) {
    console.print("0x");
    let mut started = false;
    for shift in (0..64).step_by(4).rev() {
        let nibble = ((value >> shift) & 0xf) as u8;
        if nibble != 0 || started || shift == 0 {
            started = true;
            let ch = if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + (nibble - 10)
            };
            console.put_char(ch as char);
        }
    }
}
