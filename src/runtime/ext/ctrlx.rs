//! "Control X" (ctrlx.mfx): keyboard tests by key name, so games can let players rebind
//! keys by storing names like "X", "Space" or "Down".
//!
//! Implemented: condition 0, "key is pressed (by name)"; 5, the same by virtual-key code;
//! 4 and 9, "any key is pressed" (inferred from use: games test them for keyboard activity).
//! Other conditions answer false. Actions ("force keys up" and the like) change nothing:
//! a key is down here only while the player holds it.

use super::{arg_i, arg_s, Extension};
use crate::ctf_loader::events::Comparison;
use crate::runtime::files::SaveFs;
use crate::runtime::value::Value;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Keys held this tick, shared with the runtime.
pub type Keys = Rc<RefCell<HashSet<u16>>>;

pub struct ControlX {
    pub keys: Keys,
}

/// Windows virtual-key code for a key name as Windows' key-name API spells it.
pub fn vk_for_name(name: &str) -> Option<u16> {
    let n = name.trim();
    let upper = n.to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u16);
        }
    }
    if let Some(f) = upper.strip_prefix('F').and_then(|d| d.parse::<u16>().ok()) {
        if (1..=24).contains(&f) {
            return Some(0x6F + f);
        }
    }
    if let Some(d) = upper.strip_prefix("NUM ").and_then(|d| d.parse::<u16>().ok()) {
        if d <= 9 {
            return Some(0x60 + d);
        }
    }
    Some(match upper.as_str() {
        "SPACE" => 0x20,
        "ENTER" | "RETURN" => 0x0D,
        "ESC" | "ESCAPE" => 0x1B,
        "TAB" => 0x09,
        "BACKSPACE" => 0x08,
        "SHIFT" | "LEFT SHIFT" | "RIGHT SHIFT" => 0x10,
        "CTRL" | "CONTROL" | "LEFT CTRL" | "RIGHT CTRL" => 0x11,
        "ALT" | "LEFT ALT" | "RIGHT ALT" => 0x12,
        "UP" => 0x26,
        "DOWN" => 0x28,
        "LEFT" => 0x25,
        "RIGHT" => 0x27,
        "INSERT" | "INS" => 0x2D,
        "DELETE" | "DEL" => 0x2E,
        "HOME" => 0x24,
        "END" => 0x23,
        "PAGE UP" | "PGUP" => 0x21,
        "PAGE DOWN" | "PGDN" => 0x22,
        "CAPS LOCK" => 0x14,
        _ => return None,
    })
}

/// A key's name as Windows spells it (the reverse of `vk_for_name`).
pub fn name_for_vk(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x20 => "Space".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x09 => "Tab".into(),
        0x08 => "Backspace".into(),
        0x10 | 0xA0 | 0xA1 => "Shift".into(),
        0x11 | 0xA2 | 0xA3 => "Ctrl".into(),
        0x12 | 0xA4 | 0xA5 => "Alt".into(),
        0x26 => "Up".into(),
        0x28 => "Down".into(),
        0x25 => "Left".into(),
        0x27 => "Right".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "Page Up".into(),
        0x22 => "Page Down".into(),
        0x14 => "Caps Lock".into(),
        _ => String::new(),
    }
}

impl Extension for ControlX {
    fn condition(&self, idx: usize, args: &[Value], _cmp: Comparison, _fs: &SaveFs) -> Option<bool> {
        match idx {
            0 => Some(vk_for_name(&arg_s(args, 0)).is_some_and(|vk| {
                let keys = self.keys.borrow();
                // Shift / Ctrl / Alt by name match either side.
                keys.contains(&vk) || matches!(vk, 0x10..=0x12) && keys.iter().any(|k| (0xA0..=0xA5).contains(k) && (k - 0xA0) / 2 == vk - 0x10)
            })),
            4 | 9 => Some(self.keys.borrow().iter().any(|&k| k > 6)),
            5 => Some(self.keys.borrow().contains(&(arg_i(args, 0).clamp(0, 255) as u16))),
            _ => Some(false),
        }
    }

    fn action(&mut self, _idx: usize, _args: &[Value], _fs: &SaveFs) -> bool {
        true
    }

    fn expression(&self, idx: usize, _args: &[Value], _fs: &SaveFs) -> Option<Value> {
        // 0: the name of a key being held (key setup screens), "" if none.
        (idx == 0).then(|| {
            let keys = self.keys.borrow();
            let mut held: Vec<u16> = keys.iter().copied().filter(|&k| k > 4).collect();
            held.sort_unstable();
            Value::Str(held.first().map_or_else(String::new, |&k| name_for_vk(k)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::vk_for_name;

    #[test]
    fn key_names() {
        assert_eq!(vk_for_name("X"), Some(0x58));
        assert_eq!(vk_for_name("space"), Some(0x20));
        assert_eq!(vk_for_name("Down"), Some(0x28));
        assert_eq!(vk_for_name("F5"), Some(0x74));
        assert_eq!(vk_for_name("nonsense"), None);
    }
}
