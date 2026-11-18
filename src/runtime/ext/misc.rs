//! Smaller extensions, each worked out from how games use them:
//!
//! - **Encryption** (`Encryption`): expression 0 encodes and 1 decodes a text with a key
//!   (games keep their save lines encoded). KlikCore's saves only need the two to undo each
//!   other, so the text passes through unchanged. (Saves written by the original runtime
//!   stay encoded, so they don't read back correctly.)
//! - **String Parser** (`parser`): action 0 sets the text; expression 4 (n, ..) is its n-th
//!   piece (from 0), split at the delimiters stored in the object (UTF-16, zero-separated).
//! - **Advanced Direction** (`AdvDir`): expression 1 (x1, y1, x2, y2) is the direction in
//!   degrees from the first point to the second (0 = right, counter-clockwise, y down);
//!   expressions 10 / 11 (direction, distance) are the x / y offsets; 4 (x1, y1, x2, y2)
//!   is the distance between the points.
//! - **The Big Box** (`bigbox`): action 22 opens a web address.
//! - **Font Embed** (`fontembed`): action 0 loads a font file so text can use it by name.

use super::Extension;
use crate::ctf_loader::events::Comparison;
use crate::runtime::files::SaveFs;
use crate::runtime::value::Value;

fn arg_s(args: &[Value], i: usize) -> String {
    args.get(i).map(Value::as_string).unwrap_or_default()
}

fn arg_f(args: &[Value], i: usize) -> f64 {
    args.get(i).map_or(0.0, Value::as_f64)
}

pub struct Encryption;

impl Extension for Encryption {
    fn condition(&self, _: usize, _: &[Value], _: Comparison, _: &SaveFs) -> Option<bool> {
        None
    }
    fn action(&mut self, _: usize, _: &[Value], _: &SaveFs) -> bool {
        true
    }
    fn expression(&self, idx: usize, args: &[Value], _: &SaveFs) -> Option<Value> {
        (idx <= 1).then(|| Value::Str(arg_s(args, 0)))
    }
}

pub struct Parser {
    text: String,
    delimiters: Vec<char>,
}

impl Parser {
    /// Delimiters from the object's settings: UTF-16 strings, each ended by a zero, until
    /// an empty one. A space if none are stored.
    pub fn from_data(data: Option<&[u8]>) -> Self {
        let mut delimiters = Vec::new();
        if let Some(d) = data {
            let units: Vec<u16> = d.chunks_exact(2).take(64).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            for s in units.split(|&u| u == 0) {
                if s.is_empty() {
                    break;
                }
                delimiters.extend(char::decode_utf16(s.iter().copied()).flatten());
            }
        }
        if delimiters.is_empty() {
            delimiters.push(' ');
        }
        Self { text: String::new(), delimiters }
    }

    fn piece(&self, n: i64) -> String {
        if n < 0 {
            return String::new();
        }
        self.text.split(|c| self.delimiters.contains(&c)).nth(n as usize).unwrap_or("").to_string()
    }
}

impl Extension for Parser {
    fn condition(&self, _: usize, _: &[Value], _: Comparison, _: &SaveFs) -> Option<bool> {
        None
    }
    fn action(&mut self, idx: usize, args: &[Value], _: &SaveFs) -> bool {
        if idx == 0 {
            self.text = arg_s(args, 0);
            return true;
        }
        false
    }
    fn expression(&self, idx: usize, args: &[Value], _: &SaveFs) -> Option<Value> {
        (idx == 4).then(|| Value::Str(self.piece(arg_f(args, 0) as i64)))
    }
}

pub struct AdvDir;

/// Direction in degrees from (x1, y1) to (x2, y2): 0 = right, counter-clockwise, y down.
pub fn direction_deg(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    (y1 - y2).atan2(x2 - x1).to_degrees().rem_euclid(360.0)
}

impl Extension for AdvDir {
    fn condition(&self, _: usize, _: &[Value], _: Comparison, _: &SaveFs) -> Option<bool> {
        None
    }
    fn action(&mut self, _: usize, _: &[Value], _: &SaveFs) -> bool {
        false
    }
    fn expression(&self, idx: usize, args: &[Value], _: &SaveFs) -> Option<Value> {
        let a = |i| arg_f(args, i);
        Some(match idx {
            1 => Value::Float(direction_deg(a(0), a(1), a(2), a(3))),
            4 => Value::Float((a(2) - a(0)).hypot(a(3) - a(1))),
            10 => Value::Float(a(0).to_radians().cos() * a(1)),
            11 => Value::Float(-a(0).to_radians().sin() * a(1)),
            _ => return None,
        })
    }
}

pub struct BigBox;

impl Extension for BigBox {
    fn condition(&self, _: usize, _: &[Value], _: Comparison, _: &SaveFs) -> Option<bool> {
        None
    }
    fn action(&mut self, idx: usize, args: &[Value], _: &SaveFs) -> bool {
        if idx != 22 {
            return false;
        }
        let url = arg_s(args, 0);
        // Only web addresses (a game can't start programs this way).
        if url.starts_with("http://") || url.starts_with("https://") {
            let _ = crate::runtime::files::open_web(&url);
        }
        true
    }
    fn expression(&self, _: usize, _: &[Value], _: &SaveFs) -> Option<Value> {
        None
    }
}

pub struct FontEmbed;

impl Extension for FontEmbed {
    fn condition(&self, _: usize, _: &[Value], _: Comparison, _: &SaveFs) -> Option<bool> {
        None
    }
    fn action(&mut self, idx: usize, args: &[Value], fs: &SaveFs) -> bool {
        if idx != 0 {
            return false;
        }
        if let Some(bytes) = fs.read_asset(&arg_s(args, 0)) {
            crate::render::text::register_font(bytes);
        }
        true
    }
    fn expression(&self, _: usize, _: &[Value], _: &SaveFs) -> Option<Value> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_pieces_and_delimiters() {
        let mut p = Parser::from_data(Some(&[b' ', 0, 0, 0]));
        let fs = SaveFs::new(std::path::Path::new("g.exe"), std::env::temp_dir());
        p.action(0, &[Value::Str("a b c".into())], &fs);
        assert_eq!(p.expression(4, &[Value::Int(1), Value::Int(1)], &fs).map(|v| v.as_string()), Some("b".into()));
        assert_eq!(p.piece(5), "");
        assert_eq!(Parser::from_data(None).delimiters, vec![' ']);
    }

    #[test]
    fn directions() {
        assert_eq!(direction_deg(0.0, 0.0, 10.0, 0.0), 0.0);
        assert_eq!(direction_deg(0.0, 0.0, 0.0, -10.0), 90.0);
        assert_eq!(direction_deg(0.0, 0.0, -10.0, 0.0), 180.0);
        let fs = SaveFs::new(std::path::Path::new("g.exe"), std::env::temp_dir());
        let up = AdvDir.expression(11, &[Value::Float(90.0), Value::Float(10.0)], &fs).unwrap().as_f64();
        assert!((up + 10.0).abs() < 1e-9);
    }

    #[test]
    fn encryption_round_trips() {
        let fs = SaveFs::new(std::path::Path::new("g.exe"), std::env::temp_dir());
        let e = Encryption.expression(0, &[Value::Str("1".into()), Value::Str("k".into())], &fs).unwrap();
        assert_eq!(Encryption.expression(1, &[e, Value::Str("k".into())], &fs).map(|v| v.as_string()), Some("1".into()));
    }
}

/// "String Tokenizer" (`StringTokenizer`), inferred from use: action 0 splits a text at any
/// of the delimiter characters (empty pieces dropped, as strtok does); expression 0 is the
/// number of pieces, 1 the piece at a 0-based index ("" past the end).
#[derive(Default)]
pub struct Tokenizer {
    pieces: Vec<String>,
}

impl Extension for Tokenizer {
    fn condition(&self, _: usize, _: &[Value], _: Comparison, _: &SaveFs) -> Option<bool> {
        None
    }

    fn action(&mut self, idx: usize, args: &[Value], _fs: &SaveFs) -> bool {
        if idx != 0 {
            return false;
        }
        let (text, delims) = (arg_s(args, 0), arg_s(args, 1));
        let delims: Vec<char> = if delims.is_empty() { vec![' '] } else { delims.chars().collect() };
        self.pieces = text.split(|c| delims.contains(&c)).filter(|p| !p.is_empty()).map(str::to_string).collect();
        true
    }

    fn expression(&self, idx: usize, args: &[Value], _fs: &SaveFs) -> Option<Value> {
        match idx {
            0 => Some(Value::Int(self.pieces.len() as i32)),
            1 => Some(Value::Str(usize::try_from(super::arg_i(args, 0)).ok().and_then(|i| self.pieces.get(i)).cloned().unwrap_or_default())),
            _ => None,
        }
    }
}
