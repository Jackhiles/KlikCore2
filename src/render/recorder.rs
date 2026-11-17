//! Ctrl+R: record what the player does into a scenario file, to replay a bug with
//! `--scenario` (or turn into a regression test with `expect` lines).
//!
//! Recording starts by restarting the current frame, so the replay (which starts the frame
//! fresh) begins in the same state; random numbers repeat because the runtime's generator
//! always starts from the same seed. Ticks are counted from the restart.

use crate::runtime::input::Input;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub struct Recorder {
    game: PathBuf,
    frame: usize,
    /// Runtime tick the recording started at.
    start: u64,
    lines: Vec<String>,
    /// Keys held: key -> tick it went down.
    held: HashMap<u16, u64>,
    last_mouse: (i64, i64),
}

impl Recorder {
    pub fn new(game: &Path, frame: usize, tick: u64) -> Self {
        Self { game: game.to_path_buf(), frame, start: tick, lines: Vec::new(), held: HashMap::new(), last_mouse: (i64::MIN, i64::MIN) }
    }

    /// Note this tick's input (before the game consumes it). `scroll` turns the frame
    /// position of the mouse back into the window position scripts use.
    pub fn observe(&mut self, tick: u64, input: &Input, scroll: (f64, f64)) {
        let t = tick.saturating_sub(self.start);
        let m = ((input.mouse.0 - scroll.0).round() as i64, (input.mouse.1 - scroll.1).round() as i64);
        if m != self.last_mouse {
            self.last_mouse = m;
            self.lines.push(format!("mouse {t} {} {}", m.0, m.1));
        }
        for (b, cmd) in [(0usize, "click"), (2, "rclick")] {
            if input.clicked[b] || b == 0 && input.double_clicked[0] {
                let cmd = if b == 0 && input.double_clicked[0] { "dclick" } else { cmd };
                self.lines.push(format!("{cmd} {t} {} {}", m.0, m.1));
            }
        }
        // Keyboard keys (mouse buttons are VK 1, 2, 4 and come from the clicks above).
        for &k in input.down.iter().filter(|k| !matches!(k, 1 | 2 | 4)) {
            self.held.entry(k).or_insert(t);
        }
        let released: Vec<u16> = self.held.keys().copied().filter(|k| !input.down.contains(k)).collect();
        for k in released {
            if let Some(from) = self.held.remove(&k) {
                self.push_key(k, from, t);
            }
        }
        if !input.typed.is_empty() {
            let text: String = input.typed.iter().collect();
            self.lines.push(format!("type {t} {text}"));
        }
    }

    fn push_key(&mut self, k: u16, from: u64, to: u64) {
        if to <= from + 3 {
            self.lines.push(format!("press {from} {k}"));
        } else {
            self.lines.push(format!("hold {from} {to} {k}"));
        }
    }

    /// Finish at runtime tick `tick` and write the scenario into `dir`. Returns its path.
    pub fn save(mut self, dir: &Path, tick: u64) -> std::io::Result<PathBuf> {
        let end = tick.saturating_sub(self.start);
        let held: Vec<(u16, u64)> = self.held.drain().collect();
        for (k, from) in held {
            self.push_key(k, from, end);
        }
        let mut out = String::new();
        let _ = writeln!(out, "# Recorded by KlikCore (Ctrl+R). Replay: klikcore --scenario <this file>");
        let _ = writeln!(out, "# Add `expect ...` lines to turn it into a test (see the README).");
        let _ = writeln!(out, "game {}", self.game.display());
        let _ = writeln!(out, "frame {}", self.frame);
        for l in &self.lines {
            let _ = writeln!(out, "{l}");
        }
        let _ = writeln!(out, "run {end}");
        std::fs::create_dir_all(dir)?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let stem = self.game.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "game".into());
        let path = dir.join(format!("{stem}-frame{}-{stamp}.scn", self.frame));
        std::fs::write(&path, out)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_holds_presses_and_clicks() {
        let mut r = Recorder::new(Path::new("g.exe"), 3, 100);
        let mut i = Input::default();
        i.down.insert(39);
        r.observe(100, &i, (0.0, 0.0));
        r.observe(101, &i, (0.0, 0.0));
        r.observe(110, &i, (0.0, 0.0));
        i.down.clear();
        i.clicked[0] = true;
        i.mouse = (50.0, 60.0);
        r.observe(111, &i, (10.0, 0.0));
        assert!(r.lines.contains(&"hold 0 11 39".to_string()), "{:?}", r.lines);
        assert!(r.lines.contains(&"click 11 40 60".to_string()), "{:?}", r.lines);
    }
}
