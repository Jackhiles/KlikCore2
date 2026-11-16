//! The MOO online object (`Moo2`): multiplayer through a game server. The servers of the
//! games that use it are gone, so it behaves as when the server can't be reached: the
//! connect action fails, the error event fires once on the next tick with a message
//! (games print it in their chat box), and nothing is ever connected, signed on, joined
//! or received. Other actions are accepted and do nothing.
//!
//! As the games use it: action 0 connects (host, port); condition 1 is "error" with the
//! message in expression 8; conditions 0 / 7 connected / signed on; 2 a message on a
//! sub-channel (text in expression 3, sender in 5); 4 / 5 a player joined / is present,
//! 6 left (their name in expression 7).

use super::Extension;
use crate::ctf_loader::events::Comparison;
use crate::runtime::files::SaveFs;
use crate::runtime::value::Value;
use std::cell::Cell;
use std::rc::Rc;

pub const NO_SERVER: &str = "Could not connect to the server.";

pub struct Moo {
    /// The running game's tick count (shared, advanced by the runtime).
    pub tick: Rc<Cell<u64>>,
    /// The tick on which the error event fires.
    error_at: Cell<Option<u64>>,
}

impl Moo {
    pub fn new(tick: Rc<Cell<u64>>) -> Self {
        Self { tick, error_at: Cell::new(None) }
    }
}

impl Extension for Moo {
    fn condition(&self, idx: usize, _: &[Value], _: Comparison, _: &SaveFs) -> Option<bool> {
        Some(idx == 1 && self.error_at.get() == Some(self.tick.get()))
    }

    fn action(&mut self, idx: usize, _: &[Value], _: &SaveFs) -> bool {
        if idx == 0 {
            self.error_at.set(Some(self.tick.get() + 1));
        }
        true
    }

    fn expression(&self, idx: usize, _: &[Value], _: &SaveFs) -> Option<Value> {
        Some(match idx {
            8 => Value::Str(NO_SERVER.into()),
            3 | 5 | 7 => Value::Str(String::new()),
            _ => Value::Int(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connecting_fails_once_on_the_next_tick() {
        let tick = Rc::new(Cell::new(10));
        let mut m = Moo::new(tick.clone());
        let fs = SaveFs::new(std::path::Path::new("game.exe"), std::env::temp_dir());
        assert_eq!(m.condition(1, &[], Comparison::Equal, &fs), Some(false));
        assert!(m.action(0, &[Value::Str("host".into()), Value::Int(1200)], &fs));
        assert_eq!(m.condition(1, &[], Comparison::Equal, &fs), Some(false));
        tick.set(11);
        assert_eq!(m.condition(1, &[], Comparison::Equal, &fs), Some(true));
        assert_eq!(m.condition(0, &[], Comparison::Equal, &fs), Some(false));
        tick.set(12);
        assert_eq!(m.condition(1, &[], Comparison::Equal, &fs), Some(false));
        assert_eq!(m.expression(8, &[], &fs).map(|v| v.as_string()), Some(NO_SERVER.to_string()));
    }
}
