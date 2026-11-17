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
mod misc;
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
        match idx {
            // The system's language as a Windows language id (1041 Japanese, 1033 English).
            3 => Some(Value::Int(system_language_id() as i32)),
            5 => Some(Value::Int(10)),
            _ => None,
        }
    }
}

/// The user's interface language as a Windows language id.
fn system_language_id() -> u16 {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        // SAFETY: no arguments; returns a language id.
        unsafe { GetUserDefaultUILanguage() }
    }
    #[cfg(not(windows))]
    {
        let lang = std::env::var("LC_ALL").or_else(|_| std::env::var("LANG")).unwrap_or_default().to_lowercase();
        match lang.get(..2).unwrap_or("") {
            "ja" => 1041,
            "fr" => 1036,
            "de" => 1031,
            "es" => 3082,
            "it" => 1040,
            "pt" => 1046,
            "ru" => 1049,
            "zh" => 2052,
            "ko" => 1042,
            _ => 1033,
        }
    }
}

/// Create the native implementation for an extension file name (e.g. "kcini"), if we have
/// one. `data` is the object's stored settings.
pub fn create(name: &str, data: Option<&[u8]>, shared: &Shared) -> Option<Box<dyn Extension>> {
    match name.to_ascii_lowercase().as_str() {
        "kcini" => Some(Box::new(ini::Ini::default())),
        "ini++15" => Some(Box::new(ini::IniPlus::default())),
        "kcarray" => Some(Box::new(kcarray::KcArray::from_data(data))),
        "ctrlx" => Some(Box::new(ctrlx::ControlX { keys: shared.keys.clone() })),
        "key" => Some(Box::new(ctrlx::KeyObject { keys: shared.keys.clone() })),
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
        "encryption" => Some(Box::new(misc::Encryption)),
        "parser" => Some(Box::new(misc::Parser::from_data(data))),
        "advdir" => Some(Box::new(misc::AdvDir)),
        "bigbox" => Some(Box::new(misc::BigBox)),
        "fontembed" => Some(Box::new(misc::FontEmbed)),
        // Screen capture and image tools (used by a debug screenshot), a slider control,
        // Steam's API: nothing to show or do here.
        "capture" | "imageconv" | "trackbar" | "steamworks" => Some(Box::new(inert::Inert)),
        // Window scaling and controller extensions: KlikCore scales the window itself and
        // reads controllers through its own support, so these report no controller.
        "ultimatefullscreen" | "xboxgamepad" | "joystick2" => Some(Box::new(inert::Inert)),
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
