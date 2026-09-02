use bootloader_api::info::{FrameBufferInfo, PixelFormat};

const BG_R: u8 = 4;
const BG_G: u8 = 12;
const BG_B: u8 = 28;

const FG_R: u8 = 40;
const FG_G: u8 = 220;
const FG_B: u8 = 230;

pub struct Console<'a> {
    buffer: &'a mut [u8],
    info: FrameBufferInfo,

    cursor_x: usize,
    cursor_y: usize,

    start_x: usize,
    scale: usize,
}

impl<'a> Console<'a> {
    pub fn new(
        buffer: &'a mut [u8],
        info: FrameBufferInfo,
        x: usize,
        y: usize,
        scale: usize,
    ) -> Self {
        Self {
            buffer,
            info,
            cursor_x: x,
            cursor_y: y,
            start_x: x,
            scale,
        }
    }

    pub fn clear(&mut self) {
        for y in 0..self.info.height {
            for x in 0..self.info.width {
                self.pixel(x, y, BG_R, BG_G, BG_B);
            }
        }

        self.cursor_x = self.start_x;
        self.cursor_y = 40;
    }

    pub fn print(&mut self, text: &str) {
        for c in text.chars() {
            self.put_char(c);
        }
    }

    pub fn println(&mut self, text: &str) {
        self.print(text);
        self.newline();
    }

    pub fn put_char(&mut self, character: char) {
        if character == '\n' {
            self.newline();
            return;
        }

        self.draw_char(self.cursor_x, self.cursor_y, character);

        self.cursor_x += 6 * self.scale;

        if self.cursor_x + 6 * self.scale >= self.info.width {
            self.newline();
        }
    }

    pub fn newline(&mut self) {
        self.cursor_x = self.start_x;
        self.cursor_y += 9 * self.scale;
    }

    pub fn backspace(&mut self) {
        let char_width = 6 * self.scale;

        if self.cursor_x <= self.start_x {
            return;
        }

        self.cursor_x -= char_width;

        let width = char_width;
        let height = 8 * self.scale;

        for y in self.cursor_y..self.cursor_y + height {
            for x in self.cursor_x..self.cursor_x + width {
                self.pixel(x, y, BG_R, BG_G, BG_B);
            }
        }
    }

    fn draw_char(&mut self, x: usize, y: usize, character: char) {
        let glyph = glyph(character);

        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    for sy in 0..self.scale {
                        for sx in 0..self.scale {
                            self.pixel(
                                x + col * self.scale + sx,
                                y + row * self.scale + sy,
                                FG_R,
                                FG_G,
                                FG_B,
                            );
                        }
                    }
                }
            }
        }
    }

    fn pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }

        let offset = (y * self.info.stride + x) * self.info.bytes_per_pixel;

        if offset + self.info.bytes_per_pixel > self.buffer.len() {
            return;
        }

        match self.info.pixel_format {
            PixelFormat::Rgb => {
                self.buffer[offset] = r;

                if self.info.bytes_per_pixel > 1 {
                    self.buffer[offset + 1] = g;
                }

                if self.info.bytes_per_pixel > 2 {
                    self.buffer[offset + 2] = b;
                }
            }

            PixelFormat::Bgr => {
                self.buffer[offset] = b;

                if self.info.bytes_per_pixel > 1 {
                    self.buffer[offset + 1] = g;
                }

                if self.info.bytes_per_pixel > 2 {
                    self.buffer[offset + 2] = r;
                }
            }

            _ => {}
        }
    }
}

fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0F, 0x10, 0x10, 0x10, 0x10, 0x10, 0x0F],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0F, 0x10, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1F],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],

        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x10, 0x1E, 0x01, 0x01, 0x1E],
        '6' => [0x0E, 0x10, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x01, 0x0E],

        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        '>' => [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10],
        '<' => [0x01, 0x02, 0x04, 0x08, 0x04, 0x02, 0x01],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '=' => [0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08],
        ' ' => [0; 7],

        _ => [0x1F, 0x11, 0x02, 0x04, 0x04, 0x00, 0x04],
    }
}
