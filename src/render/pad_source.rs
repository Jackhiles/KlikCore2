//! Reading the controller, shared by the launcher and games.
//!
//! Steam Input (Steam Deck, and any controller on a PC when a game is started from Steam)
//! hands the game a virtual Xbox-style controller. On Windows it does that through XInput,
//! hooked into the process, and hides the real device; a game that reads controllers
//! another way may see nothing, or the raw device with Steam's remapping missing. So on
//! Windows XInput is read first, and gilrs (Windows.Gaming.Input, which also knows
//! PlayStation and other controllers when Steam isn't involved) only when XInput has no
//! controller. On Linux Steam adds its virtual pad next to the real one; the virtual one
//! is preferred so Steam's layout applies.

use gilrs::{Axis, Button, Gilrs};
use macroquad::prelude::*;
use std::collections::HashSet;

/// One frame of controller state. Stick y points up.
#[derive(Debug, Default, Clone)]
pub struct PadState {
    pub buttons: HashSet<Button>,
    pub left: Vec2,
    pub right: Vec2,
}

impl PadState {
    pub fn pressed(&self, b: Button) -> bool {
        self.buttons.contains(&b)
    }
}

pub struct PadReader {
    gilrs: Option<Gilrs>,
    #[cfg(windows)]
    xinput: xinput::Slots,
}

impl PadReader {
    pub fn new() -> Self {
        Self {
            gilrs: Gilrs::new().ok(),
            #[cfg(windows)]
            xinput: xinput::Slots::default(),
        }
    }

    /// The first connected controller's state, if any.
    pub fn poll(&mut self) -> Option<PadState> {
        #[cfg(windows)]
        if let Some(s) = self.xinput.poll() {
            // Keep gilrs' event queue from growing while it isn't used.
            if let Some(g) = self.gilrs.as_mut() {
                while g.next_event().is_some() {}
            }
            return Some(s);
        }
        let g = self.gilrs.as_mut()?;
        while g.next_event().is_some() {}
        let pads: Vec<_> = g.gamepads().filter(|(_, p)| p.is_connected()).collect();
        let virtual_pad = |name: &str| {
            let n = name.to_ascii_lowercase();
            n.contains("steam") && n.contains("virtual")
        };
        let (_, p) = pads
            .iter()
            .find(|(_, p)| virtual_pad(p.name()) || virtual_pad(p.os_name()))
            .or(pads.first())?;
        let mut s = PadState {
            left: vec2(p.value(Axis::LeftStickX), p.value(Axis::LeftStickY)),
            right: vec2(p.value(Axis::RightStickX), p.value(Axis::RightStickY)),
            ..Default::default()
        };
        for b in ALL_BUTTONS {
            if p.is_pressed(b) {
                s.buttons.insert(b);
            }
        }
        Some(s)
    }
}

impl Default for PadReader {
    fn default() -> Self {
        Self::new()
    }
}

const ALL_BUTTONS: [Button; 16] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::Start,
    Button::Select,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger2,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
];

#[cfg(windows)]
mod xinput {
    use super::PadState;
    use gilrs::Button;
    use macroquad::prelude::*;

    #[repr(C)]
    #[derive(Default)]
    struct Gamepad {
        buttons: u16,
        left_trigger: u8,
        right_trigger: u8,
        lx: i16,
        ly: i16,
        rx: i16,
        ry: i16,
    }

    #[repr(C)]
    #[derive(Default)]
    struct State {
        packet: u32,
        pad: Gamepad,
    }

    #[link(name = "xinput")]
    extern "system" {
        fn XInputGetState(user: u32, state: *mut State) -> u32;
    }

    fn get(slot: u32) -> Option<State> {
        let mut st = State::default();
        // SAFETY: `st` is a valid XINPUT_STATE for the call to fill in.
        (unsafe { XInputGetState(slot, &mut st) } == 0).then_some(st)
    }

    const BITS: [(u16, Button); 14] = [
        (0x0001, Button::DPadUp),
        (0x0002, Button::DPadDown),
        (0x0004, Button::DPadLeft),
        (0x0008, Button::DPadRight),
        (0x0010, Button::Start),
        (0x0020, Button::Select),
        (0x0040, Button::LeftThumb),
        (0x0080, Button::RightThumb),
        (0x0100, Button::LeftTrigger),
        (0x0200, Button::RightTrigger),
        (0x1000, Button::South),
        (0x2000, Button::East),
        (0x4000, Button::West),
        (0x8000, Button::North),
    ];

    /// Polls the controller in use every frame. Asking an empty slot is slow, so the
    /// others are only looked at every two seconds while none is connected.
    #[derive(Default)]
    pub struct Slots {
        current: Option<u32>,
        next_scan: f64,
    }

    impl Slots {
        pub fn poll(&mut self) -> Option<PadState> {
            let st = match self.current.and_then(|s| get(s).map(|st| (s, st))) {
                Some((_, st)) => st,
                None => {
                    self.current = None;
                    let now = get_time();
                    if now < self.next_scan {
                        return None;
                    }
                    self.next_scan = now + 2.0;
                    let (slot, st) = (0..4).find_map(|s| get(s).map(|st| (s, st)))?;
                    self.current = Some(slot);
                    st
                }
            };
            let p = st.pad;
            let axis = |v: i16| (v as f32 / 32767.0).clamp(-1.0, 1.0);
            let mut s = PadState {
                left: vec2(axis(p.lx), axis(p.ly)),
                right: vec2(axis(p.rx), axis(p.ry)),
                ..Default::default()
            };
            for (bit, b) in BITS {
                if p.buttons & bit != 0 {
                    s.buttons.insert(b);
                }
            }
            if p.left_trigger > 90 {
                s.buttons.insert(Button::LeftTrigger2);
            }
            if p.right_trigger > 90 {
                s.buttons.insert(Button::RightTrigger2);
            }
            Some(s)
        }
    }
}
