use crate::{
    capability::Capability, console::Console, heap, keyboard::Key, memory, paging, syscall, task,
    timer, userspace, vfs,
};

const PROMPT: &str = "WOVENHAT> ";
const COMMAND_CAPACITY: usize = 128;

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

        match command {
            "" => {}
            "help" => {
                console.println("COMMANDS: HELP CLEAR VERSION TICKS TASKS CAPS MEMORY PAGING HEAP FS CAT SPAWN SYSCALL USER RING3");
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
            "cat" => {
                if !authorize(Capability::FileRead, console) {
                    self.finish(console);
                    return;
                }
                let Ok(mut file) = vfs::open("/etc/motd") else {
                    console.println("VFS OPEN FAILED");
                    self.finish(console);
                    return;
                };
                let mut buffer = [0_u8; 64];
                match vfs::read(&mut file, &mut buffer) {
                    Ok(length) => match core::str::from_utf8(&buffer[..length]) {
                        Ok(text) => console.print(text),
                        Err(_) => console.println("VFS DATA IS NOT UTF-8"),
                    },
                    Err(_) => console.println("VFS READ FAILED"),
                }
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
                console.println(command);
            }
        }

        self.finish(console);
    }

    fn finish(&mut self, console: &mut Console<'_>) {
        self.length = 0;
        self.print_prompt(console);
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
