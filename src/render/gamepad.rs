//! Gamepad support for games that only know keyboard and mouse: controller input is turned
//! into the key presses, clicks and mouse movement the game already understands.
//!
//! The layout comes from the game itself, the same way as the touch controls
//! ([`super::touch::GameControls`]): the D-pad and left stick press the game's direction
//! keys, and A, B, X, Y, RB, LB press its other keys, most used first (Enter, if used, on
//! A). In frames played with the pointer (menus, point-and-click screens) A clicks and the
//! left stick moves the cursor too. A legend at the bottom shows the buttons whenever the
//! layout changes. Games with no keys at all get a fixed layout:
//!
//! | Control              | Becomes                                   |
//! |----------------------|-------------------------------------------|
//! | Left stick / D-pad   | arrow keys (or W A S D)                   |
//! | Right stick          | moves an on-screen mouse cursor           |
//! | RT (RB)              | left click                                |
//! | LT (LB)              | right click                               |
//! | A / Start            | Enter                                     |
//! | B                    | Space                                     |
//! | X                    | Ctrl                                      |
//! | Y                    | Shift                                     |
//! | Back / Select        | Escape                                    |
//!
//! The controller is read through [`super::pad_source`], which works with Steam Input.
//! Remaps go in the settings file as `pad_<button>=<key>`, where button is one of
//! `south east west north start select lb rb lt rt l3 r3 up down left right` and the value
//! is a Windows key code (13 = Enter, 32 = Space, 65 = A, ...), `click`, `rclick` or `none`.
//! A remapped button keeps its remap in every frame.

use super::pad_source::{PadReader, PadState};
use super::settings::Settings;
use super::touch::{key_label, GameControls};
use crate::runtime::Input;
use gilrs::Button;
use macroquad::prelude::*;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Key(u16),
    Click(usize),
    None,
}

const BUTTONS: [(&str, Button); 16] = [
    ("south", Button::South),
    ("east", Button::East),
    ("west", Button::West),
    ("north", Button::North),
    ("start", Button::Start),
    ("select", Button::Select),
    ("lb", Button::LeftTrigger),
    ("rb", Button::RightTrigger),
    ("lt", Button::LeftTrigger2),
    ("rt", Button::RightTrigger2),
    ("l3", Button::LeftThumb),
    ("r3", Button::RightThumb),
    ("up", Button::DPadUp),
    ("down", Button::DPadDown),
    ("left", Button::DPadLeft),
    ("right", Button::DPadRight),
];

const ARROWS: [u16; 4] = [0x26, 0x28, 0x25, 0x27];
const WASD: [u16; 4] = [0x57, 0x53, 0x41, 0x44];
const ENTER: u16 = 0x0D;
/// Buttons that take the game's keys, in order.
const KEY_SLOTS: [Button; 6] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::RightTrigger,
    Button::LeftTrigger,
];

fn short_name(b: Button) -> &'static str {
    match b {
        Button::South => "A",
        Button::East => "B",
        Button::West => "X",
        Button::North => "Y",
        Button::RightTrigger => "RB",
        Button::LeftTrigger => "LB",
        Button::RightTrigger2 => "RT",
        Button::LeftTrigger2 => "LT",
        Button::Start => "Start",
        Button::Select => "Back",
        _ => "",
    }
}

/// The fixed layout (games without keys, and buttons the game's keys don't fill).
fn default_action(b: Button, dirs: [u16; 4]) -> Action {
    match b {
        Button::South | Button::Start => Action::Key(ENTER),
        Button::East => Action::Key(32),
        Button::West => Action::Key(17),
        Button::North => Action::Key(16),
        Button::Select => Action::Key(27),
        Button::RightTrigger | Button::RightTrigger2 => Action::Click(0),
        Button::LeftTrigger | Button::LeftTrigger2 => Action::Click(2),
        Button::DPadUp => Action::Key(dirs[0]),
        Button::DPadDown => Action::Key(dirs[1]),
        Button::DPadLeft => Action::Key(dirs[2]),
        Button::DPadRight => Action::Key(dirs[3]),
        _ => Action::None,
    }
}

fn parse_action(v: &str) -> Option<Action> {
    match v.trim() {
        "click" => Some(Action::Click(0)),
        "rclick" => Some(Action::Click(2)),
        "none" => Some(Action::None),
        n => n.parse::<u16>().ok().map(Action::Key),
    }
}

/// The button layout for a game: its direction keys and its other keys on the face
/// buttons and bumpers (the rest keep the fixed layout).
fn auto_layout(controls: &GameControls, wasd: Option<bool>) -> ([u16; 4], Vec<(Button, Action)>) {
    let dirs = match wasd {
        Some(true) => WASD,
        Some(false) => ARROWS,
        None => controls.controls.dpad.unwrap_or(ARROWS),
    };
    let mut keys = controls.controls.buttons.clone();
    if let Some(i) = keys.iter().position(|&k| k == ENTER) {
        let k = keys.remove(i);
        keys.insert(0, k);
    }
    let mut map: Vec<(Button, Action)> = BUTTONS
        .iter()
        .map(|&(_, b)| (b, default_action(b, dirs)))
        .collect();
    if !keys.is_empty() {
        for (slot, b) in KEY_SLOTS.iter().enumerate() {
            let a = match keys.get(slot) {
                Some(&k) => Action::Key(k),
                // Unfilled face buttons do nothing (rather than press keys the game
                // doesn't use); the bumpers keep clicking.
                None if slot < 4 => Action::None,
                None => continue,
            };
            if let Some(e) = map.iter_mut().find(|(x, _)| x == b) {
                e.1 = a;
            }
        }
    }
    (dirs, map)
}

pub struct Gamepad {
    reader: Option<PadReader>,
    map: Vec<(Button, Action)>,
    /// Buttons the player remapped in the settings (left alone in pointer frames).
    remapped: HashSet<Button>,
    dirs: [u16; 4],
    /// Per frame: played with keys (false = pointer frame: A clicks, left stick aims).
    frame_uses_keys: Vec<bool>,
    speed: f32,
    /// Buttons / stick directions held last frame (for press edges).
    held: HashSet<Button>,
    held_stick: HashSet<u16>,
    /// Virtual cursor in game-view coordinates, while a stick drives the mouse.
    pub cursor: Option<Vec2>,
    last_real_mouse: Option<Vec2>,
    /// A controller is connected (the legend is only shown then).
    connected: bool,
    /// Legend: the lines shown, and when they were last changed.
    legend: Vec<(String, String)>,
    legend_since: f64,
}

impl Gamepad {
    pub fn new(s: &Settings, controls: &GameControls) -> Self {
        let (dirs, auto) = auto_layout(controls, s.pad_wasd);
        let mut remapped = HashSet::new();
        let map = BUTTONS
            .iter()
            .map(|&(name, b)| {
                match s
                    .pad_map
                    .iter()
                    .find(|(k, _)| k == name)
                    .and_then(|(_, v)| parse_action(v))
                {
                    Some(a) => {
                        remapped.insert(b);
                        (b, a)
                    }
                    None => auto
                        .iter()
                        .find(|(x, _)| *x == b)
                        .copied()
                        .unwrap_or((b, Action::None)),
                }
            })
            .collect();
        Self {
            reader: s.pad.then(PadReader::new),
            map,
            remapped,
            dirs,
            frame_uses_keys: if controls.controls.is_empty() {
                Vec::new()
            } else {
                controls.frame_uses_keys.clone()
            },
            speed: [350.0, 650.0, 1100.0][(s.pad_speed.clamp(1, 3) - 1) as usize],
            held: HashSet::new(),
            held_stick: HashSet::new(),
            cursor: None,
            last_real_mouse: None,
            connected: false,
            legend: Vec::new(),
            legend_since: -100.0,
        }
    }

    /// Is frame `frame` played with the pointer? (All frames, for games without keys.)
    fn pointer_frame(&self, frame: usize) -> bool {
        !self.frame_uses_keys.get(frame).copied().unwrap_or(false)
    }

    fn action(&self, b: Button, pointer: bool) -> Action {
        if pointer && b == Button::South && !self.remapped.contains(&b) {
            return Action::Click(0);
        }
        self.map
            .iter()
            .find(|(x, _)| *x == b)
            .map_or(Action::None, |(_, a)| *a)
    }

    /// Add the gamepad's input to `input` for frame `frame`. `view` is the game view size;
    /// `to_view` maps a window position into it and `scroll` is the frame's current scroll.
    pub fn apply(
        &mut self,
        input: &mut Input,
        frame: usize,
        view: Vec2,
        to_view: impl Fn(Vec2) -> Vec2,
        scroll: Vec2,
    ) {
        let Some(pad) = self.reader.as_mut().and_then(|r| r.poll()) else {
            self.connected = false;
            self.cursor = None;
            return;
        };
        self.connected = true;
        let pointer = self.pointer_frame(frame);
        self.update_legend(pointer);
        self.buttons(&pad, input, pointer);

        // Left stick acts like the D-pad (in pointer frames it moves the cursor instead).
        let mut stick = HashSet::new();
        if !pointer {
            let (lx, ly) = (pad.left.x, pad.left.y);
            let k = self.dirs;
            for (on, k) in [
                (ly > 0.45, k[0]),
                (ly < -0.45, k[1]),
                (lx < -0.45, k[2]),
                (lx > 0.45, k[3]),
            ] {
                if on {
                    stick.insert(k);
                    input.down.insert(k);
                    if !self.held_stick.contains(&k) {
                        input.pressed.insert(k);
                    }
                }
            }
        }
        self.held_stick = stick;

        // The right stick (and in pointer frames the left one) moves a virtual cursor;
        // touching the real mouse hands control back.
        let real = Vec2::from(mouse_position());
        if self.last_real_mouse.is_some_and(|m| (real - m).length() > 1.0) {
            self.cursor = None;
        }
        self.last_real_mouse = Some(real);
        let mut v = vec2(pad.right.x, -pad.right.y);
        if pointer && pad.left.length() > v.length() {
            v = vec2(pad.left.x, -pad.left.y);
        }
        let mag = v.length();
        if mag > 0.18 {
            // Ease in: small tilts move slowly for precise aiming.
            let scaled = v / mag * ((mag.min(1.0) - 0.18) / 0.82).powf(1.6);
            let start = self.cursor.unwrap_or_else(|| to_view(real));
            let next = start + scaled * self.speed * get_frame_time();
            self.cursor = Some(next.clamp(Vec2::ZERO, view - Vec2::ONE));
        }
        if let Some(c) = self.cursor {
            let m = c + scroll;
            input.mouse = (m.x as f64, m.y as f64);
        }
    }

    fn buttons(&mut self, pad: &PadState, input: &mut Input, pointer: bool) {
        let mut now = HashSet::new();
        for &(b, _) in &self.map {
            if !pad.pressed(b) {
                continue;
            }
            now.insert(b);
            let edge = !self.held.contains(&b);
            match self.action(b, pointer) {
                Action::Key(k) => {
                    input.down.insert(k);
                    if edge {
                        input.pressed.insert(k);
                    }
                }
                Action::Click(i) => {
                    input.down.insert([1u16, 4, 2][i.min(2)]);
                    if edge {
                        input.clicked[i.min(2)] = true;
                    }
                }
                Action::None => {}
            }
        }
        self.held = now;
    }

    /// The legend for the current layout: button, what it does.
    fn legend_lines(&self, pointer: bool) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let dirs = if self.dirs == ARROWS {
            "Arrows".to_string()
        } else if self.dirs == WASD {
            "WASD".to_string()
        } else {
            self.dirs.map(key_label).join(" ")
        };
        out.push((
            "Stick".to_string(),
            if pointer {
                "Cursor".to_string()
            } else {
                dirs.clone()
            },
        ));
        if pointer {
            out.push(("D-pad".to_string(), dirs));
        }
        for b in [
            Button::South,
            Button::East,
            Button::West,
            Button::North,
            Button::RightTrigger,
            Button::LeftTrigger,
            Button::RightTrigger2,
        ] {
            let what = match self.action(b, pointer) {
                Action::Key(k) => key_label(k),
                Action::Click(0) => "Click".into(),
                Action::Click(_) => "R-click".into(),
                Action::None => continue,
            };
            out.push((short_name(b).to_string(), what));
        }
        out
    }

    fn update_legend(&mut self, pointer: bool) {
        let lines = self.legend_lines(pointer);
        if lines != self.legend {
            self.legend = lines;
            self.legend_since = get_time();
        }
    }

    /// Draw the legend along the bottom of the window for a few seconds after the layout
    /// changes (and the first time a controller is used).
    pub fn draw_legend(&self, font: Option<&Font>) {
        const SHOW: f64 = 4.0;
        const FADE: f64 = 0.6;
        let age = get_time() - self.legend_since;
        if !self.connected || self.legend.is_empty() || age > SHOW + FADE {
            return;
        }
        let a = if age > SHOW {
            (1.0 - (age - SHOW) / FADE) as f32
        } else {
            1.0
        };
        let size = (screen_height() * 0.032).clamp(14.0, 26.0) as u16;
        let measure = |t: &str| measure_text(t, font, size, 1.0).width;
        let gap = size as f32 * 0.5;
        let pill = |t: &str| measure(t) + size as f32 * 0.7;
        let total: f32 = self
            .legend
            .iter()
            .map(|(b, w)| pill(b) + gap * 0.4 + measure(w) + gap * 1.6)
            .sum::<f32>()
            - gap * 1.6;
        let h = size as f32 * 1.9;
        let (mut x, y) = (
            (screen_width() - total) / 2.0,
            screen_height() - h - size as f32 * 0.8,
        );
        draw_rectangle(
            x - gap * 1.4,
            y,
            total + gap * 2.8,
            h,
            Color::new(0.04, 0.05, 0.07, 0.8 * a),
        );
        let text_y = y + h / 2.0 + size as f32 * 0.35;
        for (b, what) in &self.legend {
            let pw = pill(b);
            let ph = size as f32 * 1.25;
            draw_rectangle(
                x,
                y + (h - ph) / 2.0,
                pw,
                ph,
                Color::new(0.35, 0.92, 0.74, 0.95 * a),
            );
            draw_text_ex(
                b,
                x + (pw - measure(b)) / 2.0,
                text_y,
                TextParams {
                    font,
                    font_size: size,
                    color: Color::new(0.06, 0.16, 0.13, a),
                    ..Default::default()
                },
            );
            x += pw + gap * 0.4;
            draw_text_ex(
                what,
                x,
                text_y,
                TextParams {
                    font,
                    font_size: size,
                    color: Color::new(0.94, 0.95, 0.97, a),
                    ..Default::default()
                },
            );
            x += measure(what) + gap * 1.6;
        }
    }

    /// Draw the virtual cursor at a window position.
    pub fn draw_cursor(&self, at: Vec2) {
        let (a, b, c) = (at, at + vec2(0.0, 18.0), at + vec2(12.0, 13.0));
        draw_triangle(
            a + vec2(1.5, 1.5),
            b + vec2(1.5, 1.5),
            c + vec2(1.5, 1.5),
            Color::new(0.0, 0.0, 0.0, 0.6),
        );
        draw_triangle(a, b, c, WHITE);
        draw_triangle_lines(a, b, c, 1.5, BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::touch::ControlSet;

    fn controls(dpad: Option<[u16; 4]>, buttons: &[u16], modes: &[bool]) -> GameControls {
        GameControls {
            controls: ControlSet {
                dpad,
                buttons: buttons.to_vec(),
            },
            frame_uses_keys: modes.to_vec(),
            ..Default::default()
        }
    }

    fn act(map: &[(Button, Action)], b: Button) -> Action {
        map.iter().find(|(x, _)| *x == b).unwrap().1
    }

    #[test]
    fn remaps_parse() {
        assert_eq!(parse_action("65"), Some(Action::Key(65)));
        assert_eq!(parse_action("click"), Some(Action::Click(0)));
        assert_eq!(parse_action("none"), Some(Action::None));
        assert_eq!(default_action(Button::DPadUp, WASD), Action::Key(87));
        assert_eq!(default_action(Button::DPadUp, ARROWS), Action::Key(38));
    }

    #[test]
    fn game_keys_go_on_the_buttons() {
        // A game using W A S D, Space, Z and Enter: Enter on A, then the rest by use.
        let (dirs, map) = auto_layout(&controls(Some(WASD), &[0x20, 0x5A, ENTER], &[true]), None);
        assert_eq!(dirs, WASD);
        assert_eq!(act(&map, Button::DPadLeft), Action::Key(0x41));
        assert_eq!(act(&map, Button::South), Action::Key(ENTER));
        assert_eq!(act(&map, Button::East), Action::Key(0x20));
        assert_eq!(act(&map, Button::West), Action::Key(0x5A));
        assert_eq!(act(&map, Button::North), Action::None);
        assert_eq!(act(&map, Button::RightTrigger), Action::Click(0));
        // The player's choice of keys wins over the game's.
        assert_eq!(
            auto_layout(&controls(Some(WASD), &[], &[]), Some(false)).0,
            ARROWS
        );
        // No keys at all: the fixed layout.
        let (dirs, map) = auto_layout(&controls(None, &[], &[]), None);
        assert_eq!(dirs, ARROWS);
        assert_eq!(act(&map, Button::South), Action::Key(ENTER));
        assert_eq!(act(&map, Button::East), Action::Key(32));
    }

    #[test]
    fn pointer_frames_click_with_a() {
        let s = Settings::default();
        let g = Gamepad::new(&s, &controls(Some(ARROWS), &[0x20], &[true, false]));
        assert_eq!(
            g.action(Button::South, g.pointer_frame(0)),
            Action::Key(0x20)
        );
        assert_eq!(
            g.action(Button::South, g.pointer_frame(1)),
            Action::Click(0)
        );
        // A game without keys is all pointer.
        let g = Gamepad::new(&s, &controls(None, &[], &[]));
        assert!(g.pointer_frame(0));
        // A remapped A stays as the player set it.
        let mut s = Settings::default();
        s.pad_map.push(("south".into(), "65".into()));
        let g = Gamepad::new(&s, &controls(Some(ARROWS), &[0x20], &[false]));
        assert_eq!(g.action(Button::South, true), Action::Key(65));
    }
}
