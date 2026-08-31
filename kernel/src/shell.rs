use crate::{console::Console, keyboard::Key, task, timer};

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
                console.println("COMMANDS: HELP CLEAR VERSION TICKS TASKS");
            }
            "clear" => {
                console.clear();
            }
            "version" => {
                console.println("WOVENHAT KERNEL 0.0.6");
            }
            "ticks" => {
                console.print("TIMER TICKS: ");
                print_u64(console, timer::ticks());
                console.newline();
            }
            "tasks" => {
                let summary = task::summary();
                console.print("TASKS: ");
                print_u64(console, summary.task_count as u64);
                console.print(" CURRENT: ");
                print_u64(console, summary.current_id.as_u64());
                console.print(" ");
                console.print(summary.current_name);
                console.print(" SWITCHES: ");
                print_u64(console, summary.context_switches);
                console.print(" IDLE: ");
                print_u64(console, summary.idle_heartbeats);
                console.newline();
            }
            _ => {
                console.print("UNKNOWN COMMAND: ");
                console.println(command);
            }
        }

        self.length = 0;
        self.print_prompt(console);
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
