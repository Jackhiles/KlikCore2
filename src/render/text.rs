//! Text rendering for String objects and text counters.
//!
//! Fusion games reference fonts by Windows LOGFONT (face name, height, weight); the
//! font files themselves are not in the game data. We look the face up among the
//! operating system's installed fonts and fall back to Macroquad's built-in font.

use crate::ctf_loader::fonts::FontEntry;
use crate::ctf_loader::properties::paragraph_flags;
use crate::ctf_loader::GameData;
use macroquad::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

/// Fonts a game loaded from its own files (the Font Embed object): name -> TrueType data.
static EMBEDDED: std::sync::OnceLock<std::sync::Mutex<HashMap<String, std::sync::Arc<Vec<u8>>>>> = std::sync::OnceLock::new();

fn name_key(name: &str) -> String {
    name.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '_').flat_map(char::to_lowercase).collect()
}

/// Make a font file usable by name for the rest of the session. Returns its family name.
pub fn register_font(bytes: Vec<u8>) -> Option<String> {
    let names = ttf_names(&bytes);
    let family = names.first().cloned()?;
    let data = std::sync::Arc::new(bytes);
    let mut map = EMBEDDED.get_or_init(Default::default).lock().ok()?;
    for n in names {
        map.insert(name_key(&n), data.clone());
    }
    Some(family)
}

fn embedded(face: &str) -> Option<std::sync::Arc<Vec<u8>>> {
    EMBEDDED.get()?.lock().ok()?.get(&name_key(face)).cloned()
}

/// Family, full and PostScript names from a TrueType / OpenType name table.
pub fn ttf_names(b: &[u8]) -> Vec<String> {
    let u16_at = |o: usize| b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize);
    let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let mut out = Vec::new();
    let Some(tables) = u16_at(4) else { return out };
    let Some(name) = (0..tables).map(|i| 12 + i * 16).find(|&r| b.get(r..r + 4) == Some(b"name")).and_then(|r| u32_at(r + 8)) else { return out };
    let (Some(count), Some(strings)) = (u16_at(name + 2), u16_at(name + 4)) else { return out };
    for want in [1, 4, 6] {
        for i in 0..count {
            let r = name + 6 + i * 12;
            let (Some(platform), Some(id), Some(len), Some(off)) = (u16_at(r), u16_at(r + 6), u16_at(r + 8), u16_at(r + 10)) else { continue };
            if id != want {
                continue;
            }
            let Some(raw) = b.get(name + strings + off..name + strings + off + len) else { continue };
            let s = if platform == 3 || platform == 0 {
                let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&units)
            } else {
                raw.iter().map(|&c| c as char).collect()
            };
            if !s.trim().is_empty() && !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(w) = std::env::var("WINDIR") {
        dirs.push(PathBuf::from(w).join("Fonts"));
    }
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        dirs.push(
            PathBuf::from(l)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts"),
        );
    }
    dirs.push(PathBuf::from("/usr/share/fonts/truetype"));
    dirs.push(PathBuf::from("/usr/share/fonts/TTF"));
    dirs.push(PathBuf::from("/usr/share/fonts"));
    dirs.push(PathBuf::from("/Library/Fonts"));
    dirs.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
    dirs
}

/// Likely file names (lowercase, without extension) for a face name.
fn candidates(face: &str, bold: bool, italic: bool) -> Vec<String> {
    let f = face.to_lowercase();
    // Common faces whose file names don't follow the face name.
    let base: &[&str] = match f.as_str() {
        "times new roman" => &["times"],
        "arial" => &["arial"],
        "arial narrow" => &["arialn", "arial"],
        "arial black" => &["ariblk", "arial"],
        "courier new" => &["cour"],
        "consolas" => &["consola", "cour"],
        "trebuchet ms" => &["trebuc"],
        "comic sans ms" => &["comic"],
        "segoe ui" => &["segoeui"],
        "lucida console" => &["lucon"],
        "palatino linotype" => &["pala"],
        "georgia" => &["georgia"],
        "verdana" => &["verdana"],
        "tahoma" => &["tahoma"],
        "impact" => &["impact"],
        "corbel" => &["corbel"],
        // Bitmap faces have no TrueType file; use a monospace stand-in.
        "terminal" | "system" | "fixedsys" | "courier" => &["consola", "cour", "lucon"],
        _ => &[],
    };
    let squashed = f.replace(' ', "");
    let mut out = Vec::new();
    for b in base
        .iter()
        .map(|s| s.to_string())
        .chain([squashed.clone(), f.clone()])
    {
        let style = match (bold, italic) {
            (true, true) => vec![format!("{b}bi"), format!("{b}z")],
            (true, false) => vec![format!("{b}bd"), format!("{b}b"), format!("{b}-bold")],
            (false, true) => vec![format!("{b}i"), format!("{b}-italic")],
            _ => vec![],
        };
        out.extend(style);
        out.push(b);
    }
    out
}

/// The installed font file for a face name, if any.
pub fn find_font_file(face: &str, bold: bool, italic: bool) -> Option<PathBuf> {
    let dirs = font_dirs();
    for name in candidates(face, bold, italic) {
        for d in &dirs {
            for ext in ["ttf", "otf", "TTF"] {
                let p = d.join(format!("{name}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

#[derive(Default)]
pub struct Fonts {
    /// Bank font handle -> loaded font (None = use the default font).
    loaded: HashMap<u32, Option<Font>>,
    by_file: HashMap<PathBuf, Font>,
    /// A font with Japanese / Chinese / Korean characters, once looked for.
    cjk: Option<Option<Font>>,
}

/// Japanese, Chinese or Korean characters (which Latin fonts don't have).
pub fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| matches!(c as u32, 0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF))
}

impl Fonts {
    fn entry(g: &GameData, handle: u32) -> Option<&FontEntry> {
        g.fonts.fonts.iter().find(|f| f.handle == handle)
    }

    /// A system font by face name (falls back to Arial, then the built-in font).
    pub fn by_face(&mut self, face: &str, bold: bool) -> Option<Font> {
        if let Some(f) = self.embedded_font(face) {
            return Some(f);
        }
        let path = find_font_file(face, bold, false)
            .or_else(|| find_font_file(face, false, false))
            .or_else(|| find_font_file("arial", bold, false))?;
        if let Some(f) = self.by_file.get(&path) {
            return Some(f.clone());
        }
        let f = load_ttf_font_from_bytes(&std::fs::read(&path).ok()?).ok()?;
        self.by_file.insert(path, f.clone());
        Some(f)
    }

    /// An installed font with Japanese / Chinese / Korean characters: Windows draws text
    /// in such characters with one even when the chosen font lacks them (font linking).
    pub fn cjk(&mut self) -> Option<Font> {
        if let Some(f) = &self.cjk {
            return f.clone();
        }
        const NAMES: [&str; 8] = ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc", "msmincho.ttc", "NotoSansCJK-Regular.ttc", "NotoSansCJKjp-Regular.otf", "NotoSansJP-Regular.otf", "DroidSansFallbackFull.ttf"];
        let mut dirs = font_dirs();
        // Linux keeps CJK fonts a folder or two deeper.
        for d in font_dirs() {
            for sub in ["noto-cjk", "opentype/noto", "google-noto-cjk", "truetype/droid", "OTF"] {
                dirs.push(d.join(sub));
            }
        }
        let found = NAMES.iter().flat_map(|n| dirs.iter().map(move |d| d.join(n))).find(|p| p.is_file());
        let font = found.and_then(|p| std::fs::read(p).ok()).and_then(|b| load_ttf_font_from_bytes(&b).ok());
        self.cjk = Some(font.clone());
        font
    }

    /// A font the game loaded itself (Font Embed), by name.
    fn embedded_font(&mut self, face: &str) -> Option<Font> {
        let data = embedded(face)?;
        let key = PathBuf::from(format!("embedded:{}", name_key(face)));
        if let Some(f) = self.by_file.get(&key) {
            return Some(f.clone());
        }
        let f = load_ttf_font_from_bytes(&data).ok()?;
        self.by_file.insert(key, f.clone());
        Some(f)
    }

    /// A font by face name only if it is installed (no substitute).
    pub fn installed(&mut self, face: &str) -> Option<Font> {
        if let Some(f) = self.embedded_font(face) {
            return Some(f);
        }
        find_font_file(face, false, false)?;
        self.by_face(face, false)
    }

    /// A font with bold / italic set by the game: the game's face (or the bank font's) in
    /// that style, if installed.
    pub fn styled(&mut self, g: &GameData, handle: Option<u32>, face: Option<&str>, bold: Option<bool>, italic: Option<bool>) -> Option<Font> {
        let entry = handle.and_then(|h| Self::entry(g, h));
        let name = face.map(str::to_string).or_else(|| entry.map(|e| e.face_name.clone())).unwrap_or_else(|| "arial".into());
        let b = bold.unwrap_or(entry.is_some_and(|e| e.weight >= 600));
        let i = italic.unwrap_or(entry.is_some_and(|e| e.italic));
        let path = find_font_file(&name, b, i).or_else(|| find_font_file(&name, b, false)).or_else(|| find_font_file(&name, false, false))?;
        if let Some(f) = self.by_file.get(&path) {
            return Some(f.clone());
        }
        let f = load_ttf_font_from_bytes(&std::fs::read(&path).ok()?).ok()?;
        self.by_file.insert(path, f.clone());
        Some(f)
    }

    /// (font, pixel height) for a bank font handle.
    pub fn get(&mut self, g: &GameData, handle: Option<u32>) -> (Option<Font>, f32) {
        let Some(h) = handle else { return (None, 16.0) };
        let entry = Self::entry(g, h);
        let size = entry.map_or(16.0, |e| (e.height.unsigned_abs() as f32).max(6.0));
        if !self.loaded.contains_key(&h) {
            let font = entry.and_then(|e| {
                let path = find_font_file(&e.face_name, e.weight >= 600, e.italic)
                    .or_else(|| find_font_file(&e.face_name, false, false))
                    .or_else(|| find_font_file("arial", false, false))?;
                if let Some(f) = self.by_file.get(&path) {
                    return Some(f.clone());
                }
                let bytes = std::fs::read(&path).ok()?;
                let f = load_ttf_font_from_bytes(&bytes).ok()?;
                self.by_file.insert(path, f.clone());
                Some(f)
            });
            self.loaded.insert(h, font);
        }
        (self.loaded[&h].clone(), size)
    }
}

/// Greedy word wrap of `text` to `max_w` pixels; explicit newlines are kept.
fn wrap(text: &str, font: Option<&Font>, size: u16, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.replace("\r\n", "\n").split(['\n', '\r']) {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && measure_text(&candidate, font, size, 1.0).width > max_w {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

/// Draw text inside the screen-space box `(x, y, w, h)`; `scale` maps game pixels to screen pixels.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_box(
    text: &str,
    font: Option<&Font>,
    px: f32,
    color: Color,
    flags: u16,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    let size = px.round().clamp(4.0, 400.0) as u16;
    let lines = wrap(text, font, size, w.max(1.0));
    let line_h = px * 1.15;
    let total_h = line_h * lines.len() as f32;
    let top = if flags & paragraph_flags::BOTTOM != 0 {
        y + h - total_h
    } else if flags & paragraph_flags::V_CENTER != 0 {
        y + (h - total_h) / 2.0
    } else {
        y
    };
    let ascent = px * 0.85;
    for (i, line) in lines.iter().enumerate() {
        let lw = measure_text(line, font, size, 1.0).width;
        let lx = if flags & paragraph_flags::RIGHT != 0 {
            x + w - lw
        } else if flags & paragraph_flags::H_CENTER != 0 {
            x + (w - lw) / 2.0
        } else {
            x
        };
        let params = TextParams {
            font,
            font_size: size,
            font_scale: 1.0,
            color,
            ..Default::default()
        };
        draw_text_ex(line, lx, top + ascent + line_h * i as f32, params);
    }
}
