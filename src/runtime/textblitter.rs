//! The Text Blitter extension (`txtblt`): text drawn with a bitmap font, an image holding
//! a grid of character cells.
//!
//! Its stored settings (worked out from games' data): width and height at 4 / 6 (u16);
//! the text at 140 (zero-terminated); the cell width and height at 1424 / 1428 (i32); the
//! font image's handle at 1468 (u16), followed by the characters in cell order
//! (zero-terminated, usually starting at the space); and from 2496 one 16-byte record per
//! cell whose first two i32 are the cell's position in the image.
//!
//! The text is laid out from the top-left: lines break at line breaks, and words wrap at
//! the object's width. Action 0 sets the text.

use std::collections::HashMap;

const TEXT_AT: usize = 140;
const CELL_SIZE_AT: usize = 1424;
const IMAGE_AT: usize = 1468;
const CHARS_AT: usize = 1470;
const CELLS_AT: usize = 2496;

#[derive(Debug, Clone, PartialEq)]
pub struct BlitFont {
    /// The font image (image bank handle).
    pub image: u32,
    pub cell_w: f32,
    pub cell_h: f32,
    /// The object's box.
    pub w: f32,
    pub h: f32,
    /// Character -> its cell's top-left corner in the image.
    cells: HashMap<char, (f32, f32)>,
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn i32_at(d: &[u8], o: usize) -> Option<i32> {
    d.get(o..o + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Zero-terminated single-byte text (Windows-1252 as Latin-1, which covers the fonts' range).
fn cstr(d: &[u8], o: usize, max: usize) -> String {
    d.get(o..(o + max).min(d.len())).unwrap_or(&[]).iter().take_while(|&&b| b != 0).map(|&b| b as char).collect()
}

impl BlitFont {
    /// The font and the initial text from the object's settings.
    pub fn from_data(d: &[u8]) -> Option<(Self, String)> {
        let (w, h) = (u16_at(d, 4)? as f32, u16_at(d, 6)? as f32);
        let (cw, ch) = (i32_at(d, CELL_SIZE_AT)?, i32_at(d, CELL_SIZE_AT + 4)?);
        if cw <= 0 || ch <= 0 {
            return None;
        }
        let image = u16_at(d, IMAGE_AT)? as u32;
        let order = cstr(d, CHARS_AT, CELLS_AT - CHARS_AT);
        let mut cells = HashMap::new();
        for (k, c) in order.chars().enumerate().take(256) {
            let at = CELLS_AT + k * 16;
            let (x, y) = (i32_at(d, at)?, i32_at(d, at + 4)?);
            cells.entry(c).or_insert((x as f32, y as f32));
        }
        let text = cstr(d, TEXT_AT, CELL_SIZE_AT - TEXT_AT);
        Some((Self { image, cell_w: cw as f32, cell_h: ch as f32, w, h, cells }, text))
    }

    /// Where each character goes: (x, y) in the image, (x, y) in the object.
    pub fn layout(&self, text: &str) -> Vec<[f32; 4]> {
        let per_line = ((self.w / self.cell_w).floor() as usize).max(1);
        let mut lines: Vec<Vec<char>> = Vec::new();
        for raw in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
            // Word wrap at the object's width (a word longer than a line is cut).
            let mut line: Vec<char> = Vec::new();
            for word in raw.split_inclusive(' ') {
                let chars: Vec<char> = word.chars().collect();
                let len = chars.iter().filter(|&&c| c != ' ').count().max(if word.trim().is_empty() { 0 } else { 1 });
                if !line.is_empty() && line.len() + len > per_line {
                    lines.push(std::mem::take(&mut line));
                }
                for c in chars {
                    if line.len() == per_line {
                        lines.push(std::mem::take(&mut line));
                    }
                    if !(line.is_empty() && c == ' ' && !lines.is_empty()) {
                        line.push(c);
                    }
                }
            }
            lines.push(line);
        }
        let mut out = Vec::new();
        for (row, line) in lines.iter().enumerate() {
            let y = row as f32 * self.cell_h;
            for (col, c) in line.iter().enumerate() {
                if let Some(&(sx, sy)) = self.cells.get(c) {
                    if *c != ' ' {
                        out.push([sx, sy, col as f32 * self.cell_w, y]);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(text: &str, w: u16) -> Vec<u8> {
        let mut d = vec![0u8; CELLS_AT + 256 * 16];
        d[4..6].copy_from_slice(&w.to_le_bytes());
        d[6..8].copy_from_slice(&40u16.to_le_bytes());
        d[TEXT_AT..TEXT_AT + text.len()].copy_from_slice(text.as_bytes());
        d[CELL_SIZE_AT..CELL_SIZE_AT + 4].copy_from_slice(&8i32.to_le_bytes());
        d[CELL_SIZE_AT + 4..CELL_SIZE_AT + 8].copy_from_slice(&10i32.to_le_bytes());
        d[IMAGE_AT..IMAGE_AT + 2].copy_from_slice(&914u16.to_le_bytes());
        let order = " ABab";
        d[CHARS_AT..CHARS_AT + order.len()].copy_from_slice(order.as_bytes());
        for k in 0..order.len() {
            let at = CELLS_AT + k * 16;
            d[at..at + 4].copy_from_slice(&((k as i32) * 8).to_le_bytes());
        }
        d
    }

    #[test]
    fn reads_settings_and_lays_out() {
        let (f, text) = BlitFont::from_data(&data("Ab", 80)).unwrap();
        assert_eq!((f.image, f.cell_w, f.cell_h, f.w), (914, 8.0, 10.0, 80.0));
        assert_eq!(text, "Ab");
        // "A" is cell 1 (x 8), "b" cell 4 (x 32); placed side by side.
        assert_eq!(f.layout("Ab"), vec![[8.0, 0.0, 0.0, 0.0], [32.0, 0.0, 8.0, 0.0]]);
        // Line breaks start a new row.
        assert_eq!(f.layout("A\r\nB")[1], [16.0, 0.0, 0.0, 10.0]);
    }

    #[test]
    fn words_wrap_at_the_width() {
        // 3 characters per line: "ab ab" -> "ab" / "ab".
        let (f, _) = BlitFont::from_data(&data("", 24)).unwrap();
        let cells = f.layout("ab ab");
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[2][3], 10.0);
        assert_eq!(cells[2][2], 0.0);
    }
}
