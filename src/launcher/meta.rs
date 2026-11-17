//! Per-game information the launcher keeps besides the library list: favourites,
//! collections and the last compatibility check, in `<data dir>\game-info.tsv`
//! (`path<TAB>favourite<TAB>collections (|-separated)<TAB>unsupported<TAB>checked<TAB>details`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The result of a compatibility check (every frame run for a while).
#[derive(Debug, Clone, Default)]
pub struct Compat {
    /// Unsupported features hit (0 = everything ran).
    pub unsupported: u32,
    /// When it was checked, "YYYY-MM-DD".
    pub checked: String,
    /// The features, most hit first ("act ... x12; cond ... x3").
    pub details: String,
}

#[derive(Debug, Clone, Default)]
pub struct GameMeta {
    pub favourite: bool,
    pub collections: Vec<String>,
    pub compat: Option<Compat>,
}

/// Everything, keyed by lower-cased game path.
#[derive(Debug, Default)]
pub struct Meta {
    games: HashMap<String, GameMeta>,
}

fn key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

fn file() -> PathBuf {
    crate::runtime::files::data_root().join("game-info.tsv")
}

impl Meta {
    pub fn load() -> Self {
        let mut m = Self::default();
        let Ok(text) = std::fs::read_to_string(file()) else { return m };
        for l in text.lines() {
            let f: Vec<&str> = l.split('\t').collect();
            let Some(&path) = f.first() else { continue };
            let compat = f.get(3).and_then(|n| n.parse::<u32>().ok()).map(|unsupported| Compat {
                unsupported,
                checked: f.get(4).unwrap_or(&"").to_string(),
                details: f.get(5).unwrap_or(&"").to_string(),
            });
            let collections = f.get(2).map(|c| c.split('|').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default();
            m.games.insert(path.to_lowercase(), GameMeta { favourite: f.get(1) == Some(&"1"), collections, compat });
        }
        m
    }

    pub fn save(&self) {
        let clean = |s: &str| s.replace(['\t', '\n', '\r', '|'], " ");
        let mut out = String::new();
        let mut keys: Vec<&String> = self.games.keys().collect();
        keys.sort();
        for k in keys {
            let g = &self.games[k];
            if !g.favourite && g.collections.is_empty() && g.compat.is_none() {
                continue;
            }
            let cols: Vec<String> = g.collections.iter().map(|c| clean(c)).collect();
            let (n, when, details) = match &g.compat {
                Some(c) => (c.unsupported.to_string(), clean(&c.checked), clean(&c.details)),
                None => ("-".into(), String::new(), String::new()),
            };
            out += &format!("{k}\t{}\t{}\t{n}\t{when}\t{details}\n", g.favourite as u8, cols.join("|"));
        }
        let _ = std::fs::write(file(), out);
    }

    pub fn get(&self, p: &Path) -> GameMeta {
        self.games.get(&key(p)).cloned().unwrap_or_default()
    }

    fn entry(&mut self, p: &Path) -> &mut GameMeta {
        self.games.entry(key(p)).or_default()
    }

    pub fn is_favourite(&self, p: &Path) -> bool {
        self.games.get(&key(p)).is_some_and(|g| g.favourite)
    }

    pub fn toggle_favourite(&mut self, p: &Path) -> bool {
        let g = self.entry(p);
        g.favourite = !g.favourite;
        let on = g.favourite;
        self.save();
        on
    }

    pub fn in_collection(&self, p: &Path, c: &str) -> bool {
        self.games.get(&key(p)).is_some_and(|g| g.collections.iter().any(|x| x == c))
    }

    /// Put the game in collection `c`, or take it out.
    pub fn set_collection(&mut self, p: &Path, c: &str, on: bool) {
        let g = self.entry(p);
        g.collections.retain(|x| x != c);
        if on {
            g.collections.push(c.to_string());
        }
        self.save();
    }

    /// Every collection name in use, sorted.
    pub fn collections(&self) -> Vec<String> {
        let mut v: Vec<String> = self.games.values().flat_map(|g| g.collections.iter().cloned()).collect();
        v.sort_by_key(|c| c.to_lowercase());
        v.dedup();
        v
    }

    pub fn set_compat(&mut self, p: &Path, c: Compat) {
        self.entry(p).compat = Some(c);
        self.save();
    }
}

/// Read a `--sweep` run's output into a compatibility result.
pub fn parse_sweep(output: &str) -> Option<Compat> {
    let mut lines = output.lines().skip_while(|l| !l.starts_with("unimplemented features hit"));
    let head = lines.next()?;
    let n: u32 = head.split('(').nth(1)?.split(')').next()?.trim().parse().ok()?;
    let mut details: Vec<String> = Vec::new();
    for l in lines.take(8) {
        // "   12  act Active/42"; anything else (log lines on the same stream) ends the list.
        let mut w = l.split_whitespace();
        let (Some(count), what) = (w.next().filter(|c| c.parse::<u64>().is_ok()), w.collect::<Vec<_>>().join(" ")) else { break };
        if what.is_empty() {
            break;
        }
        details.push(format!("{what} x{count}"));
    }
    Some(Compat { unsupported: n, checked: chrono::Local::now().format("%Y-%m-%d").to_string(), details: details.join("; ") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_output_is_read() {
        let out = "Loaded x\nswept 3 frames\nunimplemented features hit (2):\n     12  act Active/42\n      3  expr Active/0\nLoaded y in 4 ms\n";
        let c = parse_sweep(out).unwrap();
        assert_eq!(c.unsupported, 2);
        assert_eq!(c.details, "act Active/42 x12; expr Active/0 x3");
        assert_eq!(parse_sweep("unimplemented features hit (0):\n").unwrap().unsupported, 0);
        assert!(parse_sweep("no sweep here").is_none());
    }
}
