//! Extension objects (.mfx) reimplemented natively.
//!
//! An extension's conditions, actions and expressions are numbered per extension:
//! condition `num` -81 is its condition 0, action 80 is action 0, expression 80 is
//! expression 0. Each supported extension maps those indices to behaviour here.

mod clock;
pub mod ctrlx;
mod gsx;
mod inert;
mod ini;
mod joystick;
mod kcfile;
mod kcarray;
mod moo;

use super::files::SaveFs;
use super::value::Value;
use crate::ctf_loader::events::Comparison;

pub trait Extension {
    /// `None` = this condition isn't implemented.
    fn condition(&self, idx: usize, args: &[Value], cmp: Comparison, fs: &SaveFs) -> Option<bool>;
    /// `false` = this action isn't implemented.
    fn action(&mut self, idx: usize, args: &[Value], fs: &SaveFs) -> bool;
    fn expression(&self, idx: usize, args: &[Value], fs: &SaveFs) -> Option<Value>;
}

/// State shared by all objects of an extension (per running game).
pub struct Shared {
    gsx: gsx::Store,
    /// Keys held this tick (for extensions that test the keyboard).
    pub keys: ctrlx::Keys,
    /// Ticks run so far (for extensions with events that fire on a later tick).
    pub tick: std::rc::Rc<std::cell::Cell<u64>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self { gsx: gsx::new_store(), keys: Default::default(), tick: Default::default() }
    }
}

/// The System object (`kcsyso`): only the operating-system version so far, reported as a
/// current Windows (games check it to refuse very old systems).
struct SystemInfo;

impl Extension for SystemInfo {
    fn condition(&self, _: usize, _: &[Value], _: Comparison, _: &SaveFs) -> Option<bool> {
        None
    }
    fn action(&mut self, _: usize, _: &[Value], _: &SaveFs) -> bool {
        false
    }
    fn expression(&self, idx: usize, _: &[Value], _: &SaveFs) -> Option<Value> {
        (idx == 5).then_some(Value::Int(10))
    }
}

/// Create the native implementation for an extension file name (e.g. "kcini"), if we have
/// one. `data` is the object's stored settings.
pub fn create(name: &str, data: Option<&[u8]>, shared: &Shared) -> Option<Box<dyn Extension>> {
    match name.to_ascii_lowercase().as_str() {
        "kcini" => Some(Box::new(ini::Ini::default())),
        "kcarray" => Some(Box::new(kcarray::KcArray::from_data(data))),
        "ctrlx" => Some(Box::new(ctrlx::ControlX { keys: shared.keys.clone() })),
        "joystick" => Some(Box::new(joystick::Joypad)),
        // Japanese text input mode: nothing to do.
        "controlime" => Some(Box::new(inert::Inert)),
        "gstorex" => Some(Box::new(gsx::GlobalStore::with_store(shared.gsx.clone()))),
        "kcclock" => Some(Box::new(clock::Clock)),
        "kcfile" => Some(Box::new(kcfile::FileObject)),
        // Cursor shape changes (arrow, text beam, ...): the system pointer stays as it is.
        // (Its "use an object's image" action is handled by the runtime.)
        "kccursor" => Some(Box::new(inert::Inert)),
        "kcsyso" => Some(Box::new(SystemInfo)),
        "moo2" => Some(Box::new(moo::Moo::new(shared.tick.clone()))),
        "kcwctrl" | "drpc" | "hiddenproperties" | "android" | "inappandroid" | "xna" | "androidplus" | "multipletouch" => {
            Some(Box::new(inert::Inert))
        }
        _ => None,
    }
}

/// Argument helpers.
pub(crate) fn arg_i(args: &[Value], i: usize) -> i32 {
    args.get(i).map_or(0, Value::as_i32)
}

pub(crate) fn arg_s(args: &[Value], i: usize) -> String {
    args.get(i).map(Value::as_string).unwrap_or_default()
}
