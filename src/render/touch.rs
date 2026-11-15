//! On-screen touch controls for keyboard games on phones and tablets.
//!
//! The keys a game needs are worked out from its event sheets: key conditions, player-control
//! conditions and player-controlled movements (through the header's player 1 keys), and
//! key names given to Control X. Arrow keys (or W A S D, or the player's direction keys)
//! become a D-pad at the bottom left; the other keys, most used first, become up to six
//! buttons at the bottom right. Held controls go into the game's input exactly as held keys.
//!
//! While the controls are active, touches are turned into the mouse here rather than by
//! macroquad, so a finger on a control isn't also a click: the first finger elsewhere moves
//! the pointer and holds the left button. A small toggle at the top right hides or shows the
//! controls (remembered per game as `touch_controls` in the game's settings).
//!
//! On by default on Android; on the desktop with `REKINDLE_TOUCH_CONTROLS=1` (the mouse then
//! acts as one finger), `=demo` to show some controls held (for screenshots) or `=debug` to
//! also print the keys found in each frame. `REKINDLE_TOUCH_CONTROLS=0` turns them off.

use super::settings::{Settings, TouchMode};
use crate::ctf_loader::events::{Ace, Events, Param, TokenValue};
use crate::ctf_loader::properties::{MovementKind, ObjectProperties};
use crate::ctf_loader::GameData;
use macroquad::prelude::*;
use std::collections::{HashMap, HashSet};

const ARROWS: [u16; 4] = [0x26, 0x28, 0x25, 0x27];
const WASD: [u16; 4] = [0x57, 0x53, 0x41, 0x44];
const ENTER: u16 = 0x0D;
const MAX_BUTTONS: usize = 6;
/// References a player-controlled movement counts as (it drives the object every tick).
const MOVEMENT_WEIGHT: u32 = 4;

// ---------- which keys the game uses ----------

/// Keys referenced by one event sheet (and its movements), with how often.
#[derive(Debug, Default, Clone)]
pub struct KeyCounts(pub HashMap<u16, u32>);

impl KeyCounts {
    fn add(&mut self, vk: u16, n: u32) {
        if vk != 0 {
            *self.0.entry(vk).or_default() += n;
        }
    }
    fn get(&self, vk: u16) -> u32 {
        self.0.get(&vk).copied().unwrap_or(0)
    }
    fn merge(&mut self, o: &KeyCounts) {
        for (&k, &n) in &o.0 {
            self.add(k, n);
        }
    }
}

/// The integer in a mask-style parameter, if it is a constant.
fn const_int(a: &Ace, i: usize) -> Option<i32> {
    match a.params.get(i).map(|p| &p.value)? {
        Param::Short(v) => Some(*v as i32),
        Param::Int(v) => Some(*v),
        Param::Expression { tokens, .. } => match tokens.first().map(|t| &t.value) {
            Some(TokenValue::Long(v)) => Some(*v),
            _ => None,
        },
        _ => None,
    }
}

/// Is a key a direction or a modifier (fine to hold together with another key)?
fn combinable(vk: u16, controls: &[u16; 8]) -> bool {
    ARROWS.contains(&vk) || controls[..4].contains(&vk) || matches!(vk, 0x10..=0x12 | 0xA0..=0xA5)
}

/// Count key references in an event sheet. `controls` are player 1's keys (up, down, left,
/// right, fire 1-4); `ext_name` names the extension behind an extension object type.
/// Returns whether the sheet waits for "any key".
///
/// Keys an event only tests together with other letter / number keys ("C, D and Delete held")
/// are cheat or debug combinations, not controls, and aren't counted.
fn count_events(events: &Events, controls: &[u16; 8], ext_name: &dyn Fn(i16) -> Option<String>, out: &mut KeyCounts) -> bool {
    let mut any_key = false;
    for g in &events.groups {
        // OR splits a group's conditions into alternatives.
        for block in g.conditions.split(|c| c.object_type == -1 && matches!(c.num, -24 | -25)) {
            let mut keys = Vec::new();
            for c in block {
                any_key |= (c.object_type, c.num) == (-6, -9);
                for (k, positive) in condition_keys(c, controls, ext_name) {
                    keys.push((k, positive && !c.negated()));
                }
            }
            let mut combo: Vec<u16> = keys.iter().filter(|(k, held)| *held && !combinable(*k, controls)).map(|k| k.0).collect();
            combo.sort_unstable();
            combo.dedup();
            for (k, _) in keys {
                if combo.len() < 2 || !combo.contains(&k) {
                    out.add(k, 1);
                }
            }
        }
    }
    any_key
}

/// Keys one condition tests (with whether the test is for the key being down).
fn condition_keys(c: &Ace, controls: &[u16; 8], ext_name: &dyn Fn(i16) -> Option<String>) -> Vec<(u16, bool)> {
    let mut keys: Vec<(u16, bool)> = c
        .params
        .iter()
        .filter_map(|p| match p.value {
            Param::Key(k) => Some((k, true)),
            _ => None,
        })
        .collect();
    match (c.object_type, c.num) {
        // Player 1 joystick pressed / held: a mask of up, down, left, right, fire 1-4.
        (-7, -4) | (-7, -6) => {
            let mask = const_int(c, 0).unwrap_or(0x3F) as u32;
            for (b, &k) in controls.iter().enumerate() {
                if mask & (1 << b) != 0 {
                    keys.push((k, true));
                }
            }
        }
        (t, n) if t >= 32 && n <= -81 && ext_name(t).is_some_and(|e| e.eq_ignore_ascii_case("ctrlx")) => {
            for p in &c.params {
                let names: Vec<&str> = match &p.value {
                    Param::String(s) => vec![s.as_str()],
                    Param::Expression { tokens, .. } => tokens
                        .iter()
                        .filter_map(|t| match &t.value {
                            TokenValue::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                keys.extend(names.into_iter().filter_map(crate::runtime::ext::ctrlx::vk_for_name).map(|k| (k, true)));
            }
        }
        _ => {}
    }
    keys
}

/// Count the keys player-controlled movements of object `oi` respond to.
fn count_movements(game: &GameData, oi: u16, controls: &[u16; 8], out: &mut KeyCounts) {
    let Some(ObjectProperties::Common(c)) = game.object(oi).map(|o| &o.props) else { return };
    for m in &c.movements {
        if m.player > 1 {
            continue;
        }
        match m.kind {
            MovementKind::EightDirections { .. } | MovementKind::Race { .. } => {
                for &k in &controls[..4] {
                    out.add(k, MOVEMENT_WEIGHT);
                }
            }
            MovementKind::Platform { control, .. } => {
                for &k in &controls[..4] {
                    out.add(k, MOVEMENT_WEIGHT);
                }
                match control {
                    2 => out.add(controls[4], MOVEMENT_WEIGHT),
                    3 => out.add(controls[5], MOVEMENT_WEIGHT),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Keys used by each frame of a game.
pub fn frame_keys(game: &GameData) -> Vec<KeyCounts> {
    let controls = game.header.controls[0].keys;
    let ext_name = |t: i16| game.extensions.iter().find(|e| e.handle as i32 == t as i32 - 32).map(|e| e.name.clone());
    game.frames
        .iter()
        .map(|f| {
            let mut k = KeyCounts::default();
            let any_key = count_events(&f.event_sheet, &controls, &ext_name, &mut k);
            // Objects placed in the frame or created by its events.
            let mut objects: HashSet<u16> = f.instances.iter().map(|i| i.object_info).collect();
            for a in f.event_sheet.groups.iter().flat_map(|g| &g.actions) {
                for p in &a.params {
                    if let Param::Create { object_info, .. } | Param::Shoot { object_info, .. } = p.value {
                        objects.insert(object_info);
                    }
                }
            }
            for oi in objects {
                count_movements(game, oi, &controls, &mut k);
            }
            if any_key && k.0.is_empty() {
                k.add(ENTER, 1);
            }
            k
        })
        .collect()
}

/// Mouse click and "mouse over object" conditions in all frames.
pub fn mouse_conditions(game: &GameData) -> u32 {
    let all = game.frames.iter().flat_map(|f| f.event_sheet.groups.iter().flat_map(|g| &g.conditions));
    all.filter(|c| c.object_type == -6 && matches!(c.num, -7..=-4)).count() as u32
}

/// Keys that never get a button: mouse buttons, Escape (Android's Back), function keys
/// (the viewer's own), the Windows keys, and Page Up / Down, End, Home, Insert and Delete
/// (in games mostly debug keys or "erase my progress").
fn wants_button(vk: u16) -> bool {
    !matches!(vk, 0 | 1 | 2 | 4 | 0x1B | 0x21..=0x24 | 0x2D | 0x2E | 0x5B | 0x5C | 0x70..=0x87)
}

/// The controls for a game's key counts: a D-pad (up, down, left, right) and the buttons,
/// most used first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlSet {
    pub dpad: Option<[u16; 4]>,
    pub buttons: Vec<u16>,
}

impl ControlSet {
    pub fn is_empty(&self) -> bool {
        self.dpad.is_none() && self.buttons.is_empty()
    }

    pub fn choose(counts: &KeyCounts, player_dirs: [u16; 4]) -> Self {
        let used = |set: &[u16; 4]| set.iter().filter(|&&k| counts.get(k) > 0).count();
        let score = |set: &[u16; 4]| set.iter().map(|&k| counts.get(k)).sum::<u32>();
        let mut sets = vec![ARROWS, WASD];
        if !player_dirs.contains(&0) && player_dirs != ARROWS && player_dirs != WASD {
            sets.push(player_dirs);
        }
        let dpad = sets.iter().filter(|s| used(s) >= 2).max_by_key(|s| (score(s), used(s))).copied();
        let mut rest = counts.clone();
        if let Some(d) = dpad {
            for s in sets.iter().filter(|s| **s == d || used(s) >= 3) {
                for k in s {
                    rest.0.remove(k);
                }
            }
        }
        let mut keys: Vec<(u16, u32)> = rest.0.into_iter().filter(|&(k, n)| n > 0 && wants_button(k)).collect();
        keys.sort_by_key(|&(k, n)| (std::cmp::Reverse(n), k));
        Self { dpad, buttons: keys.into_iter().take(MAX_BUTTONS).map(|(k, _)| k).collect() }
    }
}

/// Short label for a key.
pub fn key_label(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x60..=0x69 => format!("Num{}", vk - 0x60),
        0x08 => "Bksp".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x10 | 0xA0 | 0xA1 => "Shift".into(),
        0x11 | 0xA2 | 0xA3 => "Ctrl".into(),
        0x12 | 0xA4 | 0xA5 => "Alt".into(),
        0x20 => "Space".into(),
        0x21 => "PgUp".into(),
        0x22 => "PgDn".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2D => "Ins".into(),
        0x2E => "Del".into(),
        0x6A => "*".into(),
        0x6B => "+".into(),
        0x6D => "-".into(),
        0xBA => ";".into(),
        0xBB => "=".into(),
        0xBC => ",".into(),
        0xBD => "-".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        _ => format!("#{vk}"),
    }
}

// ---------- layout ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Centre and radius.
    pub dpad: Option<(Vec2, f32)>,
    /// Centre and radius of each button.
    pub buttons: Vec<(Vec2, f32)>,
    pub toggle: Rect,
}

impl Layout {
    /// Controls sized for thumbs relative to the screen's short side: the D-pad in the
    /// bottom-left corner, buttons in staggered rows rising leftwards from the bottom right.
    pub fn new(screen: Vec2, dpad: bool, buttons: usize) -> Self {
        let s = screen.x.min(screen.y);
        let margin = s * 0.045;
        let r = s * 0.075;
        let pitch = r * 2.36;
        let dr = s * 0.2;
        let dpad = dpad.then(|| (vec2(margin + dr, screen.y - margin - dr), dr));
        let anchor = vec2(screen.x - margin - r, screen.y - margin - r);
        let place = |cols: usize| -> Vec<(Vec2, f32)> {
            (0..buttons)
                .map(|i| {
                    let (row, col) = ((i / cols) as f32, (i % cols) as f32);
                    let stagger = if (i / cols) % 2 == 1 { 0.5 } else { 0.0 };
                    (anchor + pitch * vec2(-col - stagger, -row - 0.22 * col), r)
                })
                .collect()
        };
        // Fewer columns if the buttons would run into the D-pad.
        let left_edge = dpad.map_or(0.0, |(c, dr)| c.x + dr + margin * 0.5);
        let mut placed = place(3);
        for cols in [2, 1] {
            if placed.iter().all(|(c, r)| c.x - r >= left_edge) {
                break;
            }
            placed = place(cols);
        }
        let t = (s * 0.075).max(28.0);
        let tm = s * 0.02;
        Self { dpad, buttons: placed, toggle: Rect::new(screen.x - tm - t, tm, t, t) }
    }

    /// The button under a point (generous, nearest wins).
    fn button_at(&self, p: Vec2) -> Option<usize> {
        self.buttons
            .iter()
            .enumerate()
            .map(|(i, (c, r))| (i, c.distance(p) / r))
            .filter(|&(_, d)| d <= 1.2)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn on_dpad(&self, p: Vec2) -> bool {
        self.dpad.is_some_and(|(c, r)| c.distance(p) <= r * 1.3)
    }

    fn on_toggle(&self, p: Vec2) -> bool {
        let t = self.toggle;
        Rect::new(t.x - t.w * 0.3, t.y - t.h * 0.3, t.w * 1.6, t.h * 1.6).contains(p)
    }
}

/// D-pad directions (up, down, left, right) for a finger at `offset` from its centre:
/// eight sectors, so diagonals hold two keys; nothing in the dead zone.
pub fn dpad_dirs(offset: Vec2, radius: f32) -> [bool; 4] {
    if offset.length() < radius * 0.22 {
        return [false; 4];
    }
    // Sector 0 = right, counting clockwise (screen y grows downwards).
    let sector = ((offset.y.atan2(offset.x) / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8);
    let right = matches!(sector, 7 | 0 | 1);
    let down = matches!(sector, 1..=3);
    let left = matches!(sector, 3..=5);
    let up = matches!(sector, 5..=7);
    [up, down, left, right]
}

// ---------- live state ----------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Dpad,
    Button(usize),
    /// Was on a button and slid off: nothing until it lands on another one.
    Buttons,
    Toggle,
    Mouse,
    None,
}

#[derive(Debug, Clone, Copy)]
struct Finger {
    target: Target,
    pos: Vec2,
}

/// What the touch controls add to this frame's input.
#[derive(Debug, Default)]
pub struct TouchInput {
    pub down: HashSet<u16>,
    pub pressed: HashSet<u16>,
    /// Touch-emulated mouse (when macroquad's own emulation is off): window position,
    /// left button held, left button clicked.
    pub mouse: Option<(Vec2, bool, bool)>,
    /// Desktop: the real left button is on a control; the game shouldn't see it.
    pub block_mouse: bool,
    /// The player toggled the controls: the new setting, to save.
    pub mode_changed: Option<TouchMode>,
}

pub struct TouchControls {
    /// Touch mode is on and the game has controls (or the player asked for them).
    active: bool,
    /// Touches are made into the mouse here (macroquad's emulation is off).
    own_mouse: bool,
    /// Turn macroquad's touch-as-mouse back on when the game ends.
    restore_mouse_sim: bool,
    /// Desktop testing: the mouse acts as a finger.
    mouse_finger: bool,
    controls: ControlSet,
    /// Per frame: does it use keys?
    frame_uses_keys: Vec<bool>,
    mode: TouchMode,
    /// More mouse conditions than key tests: "auto" starts hidden.
    mouse_first: bool,
    fingers: HashMap<u64, Finger>,
    held: HashSet<u16>,
    pointer: Option<Vec2>,
    /// Frame shown, for `visible`.
    frame: usize,
    demo: bool,
}

const MOUSE_ID: u64 = u64::MAX;

impl TouchControls {
    pub fn new(game: &GameData, s: &Settings) -> Self {
        let env = std::env::var("REKINDLE_TOUCH_CONTROLS").unwrap_or_default();
        let on = if env.is_empty() { cfg!(target_os = "android") } else { !matches!(env.as_str(), "0" | "off") };
        let per_frame = frame_keys(game);
        let mut all = KeyCounts::default();
        for k in &per_frame {
            all.merge(k);
        }
        let dirs = game.header.controls[0].keys;
        if env == "debug" {
            for (i, k) in per_frame.iter().enumerate() {
                let mut v: Vec<_> = k.0.iter().map(|(k, n)| (key_label(*k), *n)).collect();
                v.sort();
                eprintln!("frame {i}: {v:?}");
            }
        }
        let mut controls = ControlSet::choose(&all, [dirs[0], dirs[1], dirs[2], dirs[3]]);
        if controls.is_empty() && s.touch_controls == TouchMode::On {
            controls = ControlSet { dpad: Some(ARROWS), buttons: vec![ENTER, 0x20] };
        }
        let active = on && !controls.is_empty();
        // Games played mostly with the mouse (with a few keyboard shortcuts) start with the
        // controls hidden; the toggle shows them.
        let in_set = |vk: &u16| controls.buttons.contains(vk) || controls.dpad.is_some_and(|d| d.contains(vk));
        let key_refs: u32 = all.0.iter().filter(|(k, _)| in_set(k)).map(|(_, n)| n).sum();
        let mouse_first = mouse_conditions(game) > key_refs;
        if on && !cfg!(target_os = "android") {
            let names: Vec<String> = controls.buttons.iter().map(|&k| key_label(k)).collect();
            let shown = if mouse_first { " (hidden at first: mouse game)" } else { "" };
            eprintln!("Touch controls: d-pad {:?}, buttons {names:?}{shown}", controls.dpad.map(|d| d.map(key_label)));
        }
        let own_mouse = active && cfg!(target_os = "android");
        if own_mouse {
            simulate_mouse_with_touch(false);
        }
        // Frames whose keys all lack controls (Escape, cheats) show none.
        let frame_uses_keys = per_frame
            .iter()
            .map(|k| k.0.keys().any(in_set))
            .collect();
        Self {
            active,
            own_mouse,
            restore_mouse_sim: own_mouse,
            mouse_finger: active && !cfg!(target_os = "android"),
            controls,
            frame_uses_keys,
            mode: s.touch_controls,
            mouse_first,
            fingers: HashMap::new(),
            held: HashSet::new(),
            pointer: None,
            frame: 0,
            demo: env == "demo",
        }
    }

    /// Are the controls shown on frame `frame`?
    fn visible(&self) -> bool {
        self.active
            && match self.mode {
                TouchMode::Off => false,
                TouchMode::On => true,
                TouchMode::Auto => !self.mouse_first && self.frame_uses_keys.get(self.frame).copied().unwrap_or(true),
            }
    }

    /// Pointer position from touches (window coordinates), when touches drive the mouse here.
    pub fn pointer(&self) -> Option<Vec2> {
        if self.own_mouse {
            self.pointer
        } else {
            None
        }
    }

    fn layout(&self) -> Layout {
        Layout::new(vec2(screen_width(), screen_height()), self.controls.dpad.is_some(), self.controls.buttons.len())
    }

    fn target_at(&self, l: &Layout, p: Vec2, mouse_taken: bool) -> Target {
        if l.on_toggle(p) {
            return Target::Toggle;
        }
        if self.visible() {
            if l.on_dpad(p) {
                return Target::Dpad;
            }
            if let Some(b) = l.button_at(p) {
                return Target::Button(b);
            }
        }
        if mouse_taken {
            Target::None
        } else {
            Target::Mouse
        }
    }

    /// Keys a finger holds.
    fn finger_keys(&self, l: &Layout, f: &Finger, out: &mut HashSet<u16>) {
        match f.target {
            Target::Dpad => {
                if let (Some((c, r)), Some(keys)) = (l.dpad, self.controls.dpad) {
                    for (on, k) in dpad_dirs(f.pos - c, r).into_iter().zip(keys) {
                        if on {
                            out.insert(k);
                        }
                    }
                }
            }
            Target::Button(b) => {
                if let Some(&k) = self.controls.buttons.get(b) {
                    out.insert(k);
                }
            }
            _ => {}
        }
    }

    fn toggle(&mut self) -> TouchMode {
        self.mode = if self.visible() { TouchMode::Off } else { TouchMode::On };
        self.fingers.retain(|_, f| !matches!(f.target, Target::Dpad | Target::Button(_) | Target::Buttons));
        self.mode
    }

    /// Read the touches for this frame.
    pub fn update(&mut self, frame: usize) -> TouchInput {
        self.frame = frame;
        if !self.active {
            return TouchInput::default();
        }
        let dpi = macroquad::miniquad::window::dpi_scale();
        let mut touches: Vec<(u64, TouchPhase, Vec2)> = touches().into_iter().map(|t| (t.id, t.phase, t.position / dpi)).collect();
        if self.mouse_finger && touches.is_empty() {
            let m = Vec2::from(mouse_position());
            let (pressed, down) = (is_mouse_button_pressed(MouseButton::Left), is_mouse_button_down(MouseButton::Left));
            let phase = match (pressed, down, self.fingers.contains_key(&MOUSE_ID)) {
                (true, false, _) => Some(TouchPhase::Ended),
                (true, true, _) => Some(TouchPhase::Started),
                (false, true, _) => Some(TouchPhase::Moved),
                (false, false, true) => Some(TouchPhase::Ended),
                _ => None,
            };
            if let Some(p) = phase {
                touches.push((MOUSE_ID, p, m));
            }
        }
        let l = self.layout();
        self.step(&l, touches)
    }

    /// Apply this frame's touches (id, phase, window position).
    fn step(&mut self, l: &Layout, touches: Vec<(u64, TouchPhase, Vec2)>) -> TouchInput {
        let mut out = TouchInput::default();
        let live: HashSet<u64> = touches.iter().map(|t| t.0).collect();
        self.fingers.retain(|id, _| live.contains(id));

        let mut taps: HashSet<u16> = HashSet::new();
        for (id, phase, pos) in touches {
            let ended = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);
            let mouse_taken = self.fingers.iter().any(|(i, f)| *i != id && f.target == Target::Mouse);
            let Some(f) = self.fingers.get_mut(&id) else {
                // New finger.
                let target = self.target_at(l, pos, mouse_taken);
                let finger = Finger { target, pos };
                if target == Target::Mouse {
                    self.pointer = Some(pos);
                    out.mouse = Some((pos, !ended, true));
                }
                if id == MOUSE_ID && target != Target::Mouse {
                    out.block_mouse = true;
                }
                if ended {
                    // Down and up within one frame: a tap.
                    match target {
                        Target::Toggle if phase == TouchPhase::Ended => out.mode_changed = Some(self.toggle()),
                        _ => self.finger_keys(l, &finger, &mut taps),
                    }
                } else {
                    self.fingers.insert(id, finger);
                }
                continue;
            };
            f.pos = pos;
            if id == MOUSE_ID && f.target != Target::Mouse {
                out.block_mouse = true;
            }
            match f.target {
                Target::Mouse => {
                    self.pointer = Some(pos);
                    out.mouse = Some((pos, !ended, false));
                }
                // Slide between buttons; off them, nothing.
                Target::Button(_) | Target::Buttons => {
                    f.target = l.button_at(pos).map_or(Target::Buttons, Target::Button);
                }
                _ => {}
            }
            if ended {
                let f = self.fingers.remove(&id).expect("present");
                if f.target == Target::Toggle && phase == TouchPhase::Ended && l.on_toggle(pos) {
                    out.mode_changed = Some(self.toggle());
                }
            }
        }

        let mut down = HashSet::new();
        if self.visible() {
            for f in self.fingers.values() {
                self.finger_keys(l, f, &mut down);
            }
        }
        out.pressed = down.iter().chain(&taps).filter(|k| !self.held.contains(k)).copied().collect();
        out.down = down.clone();
        self.held = down;
        if !self.own_mouse {
            // Desktop: the real mouse is the pointer.
            out.mouse = None;
        } else if out.mouse.is_none() {
            out.mouse = self.pointer.map(|p| (p, false, false));
        }
        out
    }

    /// Draw the controls over the game.
    pub fn draw(&self, font: Option<&Font>) {
        if !self.active {
            return;
        }
        let l = self.layout();
        let visible = self.visible();
        let mut held = self.held.clone();
        let mut dpad_finger = self.fingers.values().find(|f| f.target == Target::Dpad).map(|f| f.pos);
        if self.demo {
            if let Some(&k) = self.controls.buttons.first() {
                held.insert(k);
            }
            if let (Some((c, r)), Some(d)) = (l.dpad, self.controls.dpad) {
                let p = c + vec2(0.55, -0.55) * r;
                dpad_finger = Some(p);
                for (on, k) in dpad_dirs(p - c, r).into_iter().zip(d) {
                    if on {
                        held.insert(k);
                    }
                }
            }
        }
        let fill = Color::new(0.0, 0.0, 0.0, 0.3);
        let fill_on = Color::new(1.0, 1.0, 1.0, 0.5);
        let edge = Color::new(1.0, 1.0, 1.0, 0.5);
        let ink = Color::new(1.0, 1.0, 1.0, 0.85);
        let ink_on = Color::new(0.0, 0.0, 0.0, 0.85);

        // Toggle: a small D-pad glyph, dimmer while the controls are hidden.
        let t = l.toggle;
        let a = if visible { 0.45 } else { 0.3 };
        draw_rectangle(t.x, t.y, t.w, t.h, Color::new(0.0, 0.0, 0.0, a * 0.6));
        draw_rectangle_lines(t.x, t.y, t.w, t.h, 1.5, Color::new(1.0, 1.0, 1.0, a));
        let (c, u) = (t.center(), t.w * 0.1);
        let glyph = Color::new(1.0, 1.0, 1.0, a + 0.2);
        let cross = c - vec2(u * 1.6, 0.0);
        draw_rectangle(cross.x - u * 0.5, cross.y - u * 1.75, u, u * 3.5, glyph);
        draw_rectangle(cross.x - u * 1.75, cross.y - u * 0.5, u * 3.5, u, glyph);
        draw_circle(c.x + u * 1.5, c.y + u * 0.6, u * 0.65, glyph);
        draw_circle(c.x + u * 2.6, c.y - u * 0.6, u * 0.65, glyph);
        if !visible {
            draw_line(t.x + t.w * 0.2, t.y + t.h * 0.8, t.x + t.w * 0.8, t.y + t.h * 0.2, 2.0, glyph);
        }
        if !visible {
            return;
        }

        if let (Some((c, r)), Some(keys)) = (l.dpad, self.controls.dpad) {
            draw_circle(c.x, c.y, r, fill);
            draw_circle_lines(c.x, c.y, r, 2.0, edge);
            // Arrows: up, down, left, right.
            for (i, dir) in [vec2(0.0, -1.0), vec2(0.0, 1.0), vec2(-1.0, 0.0), vec2(1.0, 0.0)].into_iter().enumerate() {
                let on = held.contains(&keys[i]);
                let side = vec2(-dir.y, dir.x);
                let tip = c + dir * r * 0.86;
                let base = c + dir * r * 0.5;
                let col = if on { fill_on } else { Color::new(1.0, 1.0, 1.0, 0.18) };
                // Highlight the pressed quarter, then the arrow head.
                if on {
                    draw_circle(c.x + dir.x * r * 0.62, c.y + dir.y * r * 0.62, r * 0.34, col);
                }
                draw_triangle(tip, base + side * r * 0.2, base - side * r * 0.2, if on { ink_on } else { ink });
            }
            if let Some(p) = dpad_finger {
                let off = p - c;
                let k = off.clamp_length_max(r * 0.75);
                draw_circle(c.x + k.x, c.y + k.y, r * 0.24, Color::new(1.0, 1.0, 1.0, 0.35));
            }
        }

        for (i, &(c, r)) in l.buttons.iter().enumerate() {
            let Some(&k) = self.controls.buttons.get(i) else { continue };
            let on = held.contains(&k);
            draw_circle(c.x, c.y, r, if on { fill_on } else { fill });
            draw_circle_lines(c.x, c.y, r, 2.0, edge);
            let label = key_label(k);
            let mut size = r * if label.chars().count() <= 2 { 0.8 } else { 0.5 };
            let mut dim = measure_text(&label, font, size.max(1.0) as u16, 1.0);
            if dim.width > r * 1.6 {
                size *= r * 1.6 / dim.width;
                dim = measure_text(&label, font, size.max(1.0) as u16, 1.0);
            }
            let params = TextParams {
                font,
                font_size: size.max(1.0) as u16,
                color: if on { ink_on } else { ink },
                ..Default::default()
            };
            draw_text_ex(&label, c.x - dim.width * 0.5, c.y + dim.offset_y * 0.5 - (dim.height - dim.offset_y) * 0.5, params);
        }
    }
}

impl Drop for TouchControls {
    fn drop(&mut self) {
        // Back to the launcher, which uses macroquad's touch-as-mouse.
        if self.restore_mouse_sim {
            simulate_mouse_with_touch(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctf_loader::events::{EventGroup, ParamEntry};

    fn cond(object_type: i16, num: i16, params: Vec<Param>) -> Ace {
        Ace {
            object_type,
            num,
            object_info: 0,
            object_info_list: 0,
            flags: 0,
            other_flags: 0,
            def_type: 0,
            identifier: 0,
            params: params.into_iter().map(|value| ParamEntry { code: 0, value }).collect(),
        }
    }

    fn sheet(conds: Vec<Ace>) -> Events {
        Events { groups: conds.into_iter().map(|c| EventGroup { flags: 0, conditions: vec![c], actions: vec![] }).collect() }
    }

    const CONTROLS: [u16; 8] = [0x26, 0x28, 0x25, 0x27, 0x10, 0x11, 0x20, 0x0D];

    #[test]
    fn detects_keys_player_controls_and_ctrlx() {
        let events = sheet(vec![
            cond(-6, -1, vec![Param::Key(0x5A)]),
            cond(-6, -2, vec![Param::Key(0x5A)]),
            cond(-6, -2, vec![Param::Key(0x25)]),
            cond(-6, -2, vec![Param::Key(0x27)]),
            // Player 1 holds up + fire 2.
            cond(-7, -6, vec![Param::Short(0x21)]),
            // Control X "X" key (extension type 33 = extension handle 1).
            cond(33, -81, vec![Param::String("X".into())]),
        ]);
        let ext = |t: i16| (t == 33).then(|| "CtrlX".to_string());
        let mut k = KeyCounts::default();
        assert!(!count_events(&events, &CONTROLS, &ext, &mut k));
        assert_eq!(k.get(0x5A), 2);
        assert_eq!(k.get(0x26), 1);
        assert_eq!(k.get(0x11), 1);
        assert_eq!(k.get(0x10), 0);
        assert_eq!(k.get(0x58), 1);
        let set = ControlSet::choose(&k, [0x26, 0x28, 0x25, 0x27]);
        assert_eq!(set.dpad, Some(ARROWS));
        assert_eq!(set.buttons, vec![0x5A, 0x11, 0x58]);
    }

    #[test]
    fn cheat_combinations_are_not_controls() {
        let held = |k| cond(-6, -2, vec![Param::Key(k)]);
        let or = cond(-1, -24, vec![]);
        let mut combo = EventGroup { flags: 0, conditions: vec![held(0x43), held(0x44), held(0x2E)], actions: vec![] };
        let alternatives = EventGroup { flags: 0, conditions: vec![held(0x5A), or.clone(), held(0x20)], actions: vec![] };
        let run = EventGroup { flags: 0, conditions: vec![held(0x10), held(0x27)], actions: vec![] };
        let mut k = KeyCounts::default();
        count_events(&Events { groups: vec![combo.clone(), alternatives, run] }, &CONTROLS, &|_| None, &mut k);
        assert_eq!((k.get(0x43), k.get(0x2E)), (0, 0));
        assert_eq!((k.get(0x5A), k.get(0x20), k.get(0x10), k.get(0x27)), (1, 1, 1, 1));
        // "C held and D not held" is not a combination.
        combo.conditions[1].other_flags = crate::ctf_loader::events::ACE_NOT;
        combo.conditions.pop();
        let mut k = KeyCounts::default();
        count_events(&Events { groups: vec![combo] }, &CONTROLS, &|_| None, &mut k);
        assert_eq!((k.get(0x43), k.get(0x44)), (1, 1));
    }

    #[test]
    fn mouse_only_games_get_nothing_and_any_key_gets_enter() {
        let events = sheet(vec![cond(-6, -5, vec![Param::Click { button: 0, double: false }]), cond(-6, -2, vec![Param::Key(1)])]);
        let mut k = KeyCounts::default();
        count_events(&events, &CONTROLS, &|_| None, &mut k);
        assert!(ControlSet::choose(&k, [0x26, 0x28, 0x25, 0x27]).is_empty());
        let mut k = KeyCounts::default();
        assert!(count_events(&sheet(vec![cond(-6, -9, vec![])]), &CONTROLS, &|_| None, &mut k));
    }

    #[test]
    fn dpad_choice() {
        let mut k = KeyCounts::default();
        // W A S D for moving, arrows aliased; Escape and F1 get no buttons; a lone Up is a button.
        for vk in WASD {
            k.add(vk, 3);
        }
        for vk in ARROWS {
            k.add(vk, 1);
        }
        k.add(0x1B, 5);
        k.add(0x70, 5);
        k.add(0x20, 2);
        let set = ControlSet::choose(&k, ARROWS);
        assert_eq!(set.dpad, Some(WASD));
        assert_eq!(set.buttons, vec![0x20]);

        let mut k = KeyCounts::default();
        k.add(0x26, 1);
        k.add(0x0D, 1);
        let set = ControlSet::choose(&k, ARROWS);
        assert_eq!(set.dpad, None);
        assert_eq!(set.buttons, vec![0x0D, 0x26]);

        let mut k = KeyCounts::default();
        for i in 0..10 {
            k.add(0x41 + 4 + i, 10 - i as u32);
        }
        assert_eq!(ControlSet::choose(&k, ARROWS).buttons.len(), MAX_BUTTONS);
    }

    #[test]
    fn dpad_sectors() {
        assert_eq!(dpad_dirs(vec2(0.0, -50.0), 100.0), [true, false, false, false]);
        assert_eq!(dpad_dirs(vec2(50.0, 50.0), 100.0), [false, true, false, true]);
        assert_eq!(dpad_dirs(vec2(-50.0, -48.0), 100.0), [true, false, true, false]);
        assert_eq!(dpad_dirs(vec2(-80.0, 5.0), 100.0), [false, false, true, false]);
        assert_eq!(dpad_dirs(vec2(5.0, 5.0), 100.0), [false; 4]);
    }

    fn controls(own_mouse: bool) -> TouchControls {
        TouchControls {
            active: true,
            own_mouse,
            restore_mouse_sim: false,
            mouse_finger: false,
            controls: ControlSet { dpad: Some(ARROWS), buttons: vec![0x5A, 0x58] },
            frame_uses_keys: vec![true, false],
            mode: TouchMode::Auto,
            mouse_first: false,
            fingers: HashMap::new(),
            held: HashSet::new(),
            pointer: None,
            frame: 0,
            demo: false,
        }
    }

    #[test]
    fn fingers_hold_keys_and_the_rest_is_the_mouse() {
        let mut t = controls(true);
        let l = Layout::new(vec2(1600.0, 900.0), true, 2);
        let (dc, dr) = l.dpad.unwrap();
        let (z, x) = (l.buttons[0].0, l.buttons[1].0);
        let step = |t: &mut TouchControls, touches: Vec<(u64, TouchPhase, Vec2)>| t.step(&l, touches);
        let set = |keys: &[u16]| keys.iter().copied().collect::<HashSet<u16>>();

        // D-pad up-right and Z at once, plus a finger on the game.
        let o = step(&mut t, vec![(1, TouchPhase::Started, dc + vec2(0.5, -0.5) * dr), (2, TouchPhase::Started, z), (3, TouchPhase::Started, vec2(800.0, 300.0))]);
        assert_eq!(o.down, set(&[0x26, 0x27, 0x5A]));
        assert_eq!(o.pressed, o.down);
        assert_eq!(o.mouse, Some((vec2(800.0, 300.0), true, true)));

        // Held: no new presses. Sliding Z's finger onto X swaps the key.
        let o = step(&mut t, vec![(1, TouchPhase::Stationary, dc + vec2(0.5, -0.5) * dr), (2, TouchPhase::Moved, x), (3, TouchPhase::Moved, vec2(820.0, 300.0))]);
        assert_eq!(o.down, set(&[0x26, 0x27, 0x58]));
        assert_eq!(o.pressed, set(&[0x58]));
        assert_eq!(o.mouse, Some((vec2(820.0, 300.0), true, false)));

        // Off the buttons: released. D-pad finger lifted; mouse finger lifted.
        let o = step(&mut t, vec![(1, TouchPhase::Ended, dc), (2, TouchPhase::Moved, vec2(800.0, 450.0)), (3, TouchPhase::Ended, vec2(820.0, 300.0))]);
        assert!(o.down.is_empty());
        assert_eq!(o.mouse, Some((vec2(820.0, 300.0), false, false)));

        // A tap that starts and ends between two frames still presses the key once.
        let o = step(&mut t, vec![(4, TouchPhase::Ended, z)]);
        assert!(o.down.is_empty());
        assert_eq!(o.pressed, set(&[0x5A]));

        // Toggle hides the controls; then touches on them are the mouse.
        let o = step(&mut t, vec![(5, TouchPhase::Ended, l.toggle.center())]);
        assert_eq!(o.mode_changed, Some(TouchMode::Off));
        let o = step(&mut t, vec![(6, TouchPhase::Started, z)]);
        assert!(o.down.is_empty());
        assert_eq!(o.mouse, Some((z, true, true)));

        // Frames that use no keys hide them in "auto".
        let mut t = controls(false);
        t.frame = 1;
        let o = t.step(&l, vec![(1, TouchPhase::Started, z)]);
        assert!(o.down.is_empty() && o.mouse.is_none());
    }

    #[test]
    fn layout_fits_without_overlaps() {
        for screen in [vec2(1920.0, 1080.0), vec2(2400.0, 1080.0), vec2(1024.0, 768.0), vec2(1080.0, 2400.0), vec2(640.0, 480.0)] {
            for n in 0..=MAX_BUTTONS {
                let l = Layout::new(screen, true, n);
                let (dc, dr) = l.dpad.expect("d-pad");
                assert!(dc.x - dr >= 0.0 && dc.y + dr <= screen.y);
                for (i, &(c, r)) in l.buttons.iter().enumerate() {
                    assert!(c.x - r >= 0.0 && c.x + r <= screen.x && c.y + r <= screen.y && c.y - r >= 0.0, "{screen} {n}");
                    assert!(c.distance(dc) > r + dr, "button {i} on the d-pad at {screen} with {n}");
                    assert!(r >= screen.x.min(screen.y) * 0.07);
                    for &(c2, r2) in &l.buttons[i + 1..] {
                        assert!(c.distance(c2) > r + r2, "buttons overlap at {screen} with {n}");
                    }
                    assert!(!l.toggle.overlaps(&Rect::new(c.x - r, c.y - r, r * 2.0, r * 2.0)));
                }
                assert_eq!(l.button_at(l.buttons.first().map_or(Vec2::ZERO, |b| b.0)), (n > 0).then_some(0));
            }
        }
    }
}
