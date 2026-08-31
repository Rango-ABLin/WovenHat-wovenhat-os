use crate::{
    capability::Capability, console::Console, heap, keyboard::Key, memory, paging, syscall, task,
    timer,
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
                console.println("COMMANDS: HELP CLEAR VERSION TICKS TASKS CAPS MEMORY PAGING HEAP SPAWN SYSCALL USER RING3");
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
            "user" => {
                if !authorize(Capability::TaskControl, console) {
                    self.finish(console);
                    return;
                }

                match task::launch_user_task("usermode", 0x1000, 0x2000000) {
                    Ok(id) => {
                        console.print("USER TASK REGISTERED: ");
                        print_u64(console, id.as_u64());
                        console.newline();
                    }
                    Err(_) => {
                        console.println("USER TASK REGISTRATION FAILED: PROCESS TABLE FULL");
                    }
                }
            }
            "ring3" => {
                if !authorize(Capability::TaskControl, console) {
                    self.finish(console);
                    return;
                }

                let entry = 0x1000usize;
                let stack_top = 0x2000000usize;
                if task::validate_user_task(entry, stack_top) {
                    console.println("RING3 ENTRY STUB: READY");
                } else {
                    console.println("RING3 ENTRY STUB: INVALID");
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
        task::yield_now();
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
