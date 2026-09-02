use alloc::vec::Vec;

use crate::graphics::{Color, Graphics};

#[derive(Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: usize, height: usize) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains(self, x: i32, y: i32) -> bool {
        let Ok(width) = i32::try_from(self.width) else {
            return false;
        };
        let Ok(height) = i32::try_from(self.height) else {
            return false;
        };
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(width)
            && y < self.y.saturating_add(height)
    }
}

pub enum InputEvent {
    PointerDown { x: i32, y: i32 },
    Key(char),
}

pub struct Button {
    pub bounds: Rect,
    pub color: Color,
    pub pressed: bool,
}

impl Button {
    pub const fn new(bounds: Rect, color: Color) -> Self {
        Self {
            bounds,
            color,
            pressed: false,
        }
    }

    fn render(&self, graphics: &mut Graphics<'_>) {
        let color = if self.pressed { Color::WHITE } else { self.color };
        graphics.fill_rect(
            self.bounds.x,
            self.bounds.y,
            self.bounds.width,
            self.bounds.height,
            color,
        );
    }

    fn handle(&mut self, event: &InputEvent) -> bool {
        match event {
            InputEvent::PointerDown { x, y } if self.bounds.contains(*x, *y) => {
                self.pressed = !self.pressed;
                true
            }
            InputEvent::Key('\n') => {
                self.pressed = !self.pressed;
                true
            }
            _ => false,
        }
    }
}

pub struct Window {
    pub bounds: Rect,
    pub title_bar_height: usize,
    pub title_color: Color,
    pub body_color: Color,
    pub buttons: Vec<Button>,
}

impl Window {
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            title_bar_height: 24,
            title_color: Color::CYAN,
            body_color: Color::WHITE,
            buttons: Vec::new(),
        }
    }

    pub fn add_button(&mut self, button: Button) {
        self.buttons.push(button);
    }

    fn render(&self, graphics: &mut Graphics<'_>) {
        graphics.fill_rect(
            self.bounds.x,
            self.bounds.y,
            self.bounds.width,
            self.title_bar_height,
            self.title_color,
        );
        graphics.fill_rect(
            self.bounds.x,
            self.bounds.y + self.title_bar_height as i32,
            self.bounds.width,
            self.bounds.height.saturating_sub(self.title_bar_height),
            self.body_color,
        );
        for button in &self.buttons {
            button.render(graphics);
        }
    }

    fn handle(&mut self, event: &InputEvent) -> bool {
        self.buttons.iter_mut().any(|button| button.handle(event))
    }
}

pub struct Desktop {
    pub background: Color,
    pub windows: Vec<Window>,
}

impl Desktop {
    pub fn new(background: Color) -> Self {
        Self {
            background,
            windows: Vec::new(),
        }
    }

    pub fn add_window(&mut self, window: Window) {
        self.windows.push(window);
    }

    pub fn render(&self, graphics: &mut Graphics<'_>) {
        graphics.clear(self.background);
        for window in &self.windows {
            window.render(graphics);
        }
    }

    pub fn handle(&mut self, event: &InputEvent) -> bool {
        for window in self.windows.iter_mut().rev() {
            if window.handle(event) {
                return true;
            }
        }
        false
    }
}

pub fn self_test() -> bool {
    let bounds = Rect::new(10, 10, 80, 30);
    let mut desktop = Desktop::new(Color::DARK_BLUE);
    let mut window = Window::new(Rect::new(0, 0, 160, 120));
    window.add_button(Button::new(bounds, Color::CYAN));
    desktop.add_window(window);

    let handled = desktop.handle(&InputEvent::PointerDown { x: 20, y: 20 });
    handled && desktop.windows[0].buttons[0].pressed
}
