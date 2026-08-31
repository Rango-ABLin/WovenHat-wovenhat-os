use core::arch::asm;

pub enum Key {
    Char(char),
    Enter,
    Backspace,
}

pub struct Keyboard {
    shift: bool,
}

impl Keyboard {
    pub const fn new() -> Self {
        Self { shift: false }
    }

    pub fn poll(&mut self) -> Option<Key> {
        // PS/2 controller status register:
        // bit 0 == output buffer contains keyboard data.
        let status = unsafe { inb(0x64) };

        if status & 1 == 0 {
            return None;
        }

        let scancode = unsafe { inb(0x60) };

        match scancode {
            // Shift pressed
            0x2A | 0x36 => {
                self.shift = true;
                None
            }

            // Shift released
            0xAA | 0xB6 => {
                self.shift = false;
                None
            }

            // Enter
            0x1C => Some(Key::Enter),

            // Backspace
            0x0E => Some(Key::Backspace),

            code => decode_scancode(code, self.shift).map(Key::Char),
        }
    }
}

/// Read one byte from an x86 I/O port.
///
/// # Safety
///
/// Port I/O is privileged hardware access. This function must only be used
/// after WovenHat OS owns the machine and only with known-valid device ports.
unsafe fn inb(port: u16) -> u8 {
    let value: u8;

    unsafe {
        asm!(
            "in al, dx",
            out("al") value,
            in("dx") port,
            options(nomem, nostack, preserves_flags)
        );
    }

    value
}

fn decode_scancode(code: u8, shift: bool) -> Option<char> {
    let normal = match code {
        0x02 => '1',
        0x03 => '2',
        0x04 => '3',
        0x05 => '4',
        0x06 => '5',
        0x07 => '6',
        0x08 => '7',
        0x09 => '8',
        0x0A => '9',
        0x0B => '0',

        0x10 => 'q',
        0x11 => 'w',
        0x12 => 'e',
        0x13 => 'r',
        0x14 => 't',
        0x15 => 'y',
        0x16 => 'u',
        0x17 => 'i',
        0x18 => 'o',
        0x19 => 'p',

        0x1E => 'a',
        0x1F => 's',
        0x20 => 'd',
        0x21 => 'f',
        0x22 => 'g',
        0x23 => 'h',
        0x24 => 'j',
        0x25 => 'k',
        0x26 => 'l',

        0x2C => 'z',
        0x2D => 'x',
        0x2E => 'c',
        0x2F => 'v',
        0x30 => 'b',
        0x31 => 'n',
        0x32 => 'm',

        0x39 => ' ',

        0x0C => '-',
        0x0D => '=',
        0x33 => ',',
        0x34 => '.',
        0x35 => '/',

        _ => return None,
    };

    if !shift {
        return Some(normal);
    }

    Some(match normal {
        'a'..='z' => normal.to_ascii_uppercase(),

        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',

        '-' => '_',
        '=' => '+',
        ',' => '<',
        '.' => '>',
        '/' => '?',

        other => other,
    })
}