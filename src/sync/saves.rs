//! Save files to sync: every file in the mapped areas (`app`, `appdata`, `other`) of each
//! game's Rekindle save folder (`<data dir>/Rekindle/<game>/`). Folders keep the same
//! names on every device, so a game's saves line up wherever it runs.
//!
//! Plan for one device pair: files only one side has are copied over, unless the last
//! sync saw them on both sides with the same content (then the other side deleted it,
//! and the deletion is copied). Files that differ: the newer one wins (clocks corrected by
//! the offset measured in the handshake). Anything replaced or deleted is backed up to
//! `<game>/backups/sync-<time>/` first.

use super::identity::{dir, hex};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

pub const AREAS: [&str; 3] = ["app", "appdata", "other"];
const MAX_SAVE: u64 = 64 * 1024 * 1024;

/// One save file.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveEntry {
    /// The game's save folder name.
    pub game: String,
    /// Path inside it, '/'-separated, starting with an area.
    pub path: String,
    pub mtime_ms: i64,
    pub size: u64,
    pub hash: String,
}

impl SaveEntry {
    pub fn key(&self) -> String {
        format!("{}/{}", self.game, self.path)
    }

    pub fn to_json(&self) -> Value {
        json!({"game": self.game, "path": self.path, "mtime": self.mtime_ms, "size": self.size, "hash": self.hash})
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            game: v["game"].as_str()?.to_string(),
            path: v["path"].as_str()?.to_string(),
            mtime_ms: v["mtime"].as_i64()?,
            size: v["size"].as_u64()?,
            hash: v["hash"].as_str()?.to_string(),
        })
    }
}

fn data_root() -> PathBuf {
    crate::runtime::files::data_root()
}

/// A game folder name or save path from another device, made safe: no absolute paths,
/// no `..`, and paths must start with a save area.
pub fn safe_path(game: &str, path: &str) -> Option<PathBuf> {
    if game.is_empty() || game.contains(['/', '\\', ':']) || game == "." || game == ".." {
        return None;
    }
    let rel = Path::new(path);
    let mut parts = rel.components();
    match parts.next() {
        Some(Component::Normal(a)) if AREAS.contains(&a.to_str()?) => {}
        _ => return None,
    }
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) || path.contains('\\') || path.contains(':') {
        return None;
    }
    Some(data_root().join(game).join(rel))
}

/// Save folders: directories in the data folder holding at least one save area.
pub fn game_folders() -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(data_root()) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && AREAS.iter().any(|a| p.join(a).is_dir()))
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
        .collect();
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

pub fn hash_file(p: &Path) -> Option<String> {
    Some(hex(&Sha256::digest(std::fs::read(p).ok()?)))
}

fn mtime_ms(p: &Path) -> Option<i64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
}

/// All save files on this device (optionally only one game's).
pub fn manifest(only_game: Option<&str>) -> Vec<SaveEntry> {
    let mut out = Vec::new();
    for (game, folder) in game_folders() {
        if only_game.is_some_and(|g| g != game) {
            continue;
        }
        for area in AREAS {
            let mut files = Vec::new();
            walk(&folder.join(area), &mut files);
            for f in files {
                let Ok(meta) = std::fs::metadata(&f) else { continue };
                if meta.len() > MAX_SAVE {
                    continue;
                }
                let Some(rel) = f.strip_prefix(&folder).ok() else { continue };
                let path = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
                let (Some(hash), Some(mtime)) = (hash_file(&f), mtime_ms(&f)) else { continue };
                out.push(SaveEntry { game: game.clone(), path, mtime_ms: mtime, size: meta.len(), hash });
            }
        }
    }
    out
}

/// What to do with one file.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Copy ours to them.
    Push(SaveEntry),
    /// Copy theirs here.
    Pull(SaveEntry),
    /// Delete ours (they deleted it).
    DeleteHere(SaveEntry),
    /// Delete theirs (we deleted it).
    DeleteThere(SaveEntry),
}

/// Plan a sync. `last`: key -> hash from the previous sync with this device.
/// `offset_ms`: their clock minus ours. `skip`: game folders not to touch (running).
pub fn plan(local: &[SaveEntry], remote: &[SaveEntry], last: &HashMap<String, String>, offset_ms: i64, skip: &[String]) -> Vec<Step> {
    let l: BTreeMap<String, &SaveEntry> = local.iter().map(|e| (e.key(), e)).collect();
    let r: BTreeMap<String, &SaveEntry> = remote.iter().map(|e| (e.key(), e)).collect();
    let mut keys: Vec<&String> = l.keys().chain(r.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut steps = Vec::new();
    for k in keys {
        let game = k.split('/').next().unwrap_or("");
        if skip.iter().any(|s| s == game) {
            continue;
        }
        match (l.get(k), r.get(k)) {
            (Some(a), None) => {
                if last.get(k) == Some(&a.hash) {
                    steps.push(Step::DeleteHere((*a).clone()));
                } else {
                    steps.push(Step::Push((*a).clone()));
                }
            }
            (None, Some(b)) => {
                if last.get(k) == Some(&b.hash) {
                    steps.push(Step::DeleteThere((*b).clone()));
                } else {
                    steps.push(Step::Pull((*b).clone()));
                }
            }
            (Some(a), Some(b)) if a.hash != b.hash => {
                // Unchanged here since the last sync: theirs is the edit, whatever the clocks say.
                if last.get(k) == Some(&a.hash) {
                    steps.push(Step::Pull((*b).clone()));
                } else if last.get(k) == Some(&b.hash) {
                    steps.push(Step::Push((*a).clone()));
                } else if b.mtime_ms - offset_ms > a.mtime_ms {
                    steps.push(Step::Pull((*b).clone()));
                } else {
                    steps.push(Step::Push((*a).clone()));
                }
            }
            _ => {}
        }
    }
    steps
}

fn backup_path(game: &str, path: &str, stamp: &str) -> PathBuf {
    data_root().join(game).join("backups").join(format!("sync-{stamp}")).join(path)
}

pub fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string()
}

/// Write a file from another device (backing up what it replaces) with its time.
pub fn store(game: &str, path: &str, data: &[u8], mtime_ms: i64, stamp: &str) -> Result<(), String> {
    let target = safe_path(game, path).ok_or("unsafe path")?;
    if target.is_file() {
        let b = backup_path(game, path, stamp);
        b.parent().map(std::fs::create_dir_all).transpose().map_err(|e| e.to_string())?;
        std::fs::copy(&target, &b).map_err(|e| e.to_string())?;
    }
    target.parent().map(std::fs::create_dir_all).transpose().map_err(|e| e.to_string())?;
    std::fs::write(&target, data).map_err(|e| e.to_string())?;
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(mtime_ms.max(0) as u64);
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&target) {
        let _ = f.set_modified(t);
    }
    Ok(())
}

/// Delete a save (backed up first).
pub fn remove(game: &str, path: &str, stamp: &str) -> Result<(), String> {
    let target = safe_path(game, path).ok_or("unsafe path")?;
    if !target.is_file() {
        return Ok(());
    }
    let b = backup_path(game, path, stamp);
    b.parent().map(std::fs::create_dir_all).transpose().map_err(|e| e.to_string())?;
    std::fs::copy(&target, &b).map_err(|e| e.to_string())?;
    std::fs::remove_file(&target).map_err(|e| e.to_string())
}

pub fn read(game: &str, path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(safe_path(game, path).ok_or("unsafe path")?).map_err(|e| e.to_string())
}

fn state_file(peer: &str) -> PathBuf {
    dir().join(format!("state-{peer}.txt"))
}

/// key -> hash as of the last sync with `peer`.
pub fn load_state(peer: &str) -> HashMap<String, String> {
    std::fs::read_to_string(state_file(peer))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(k, h)| (k.to_string(), h.to_string())))
        .collect()
}

pub fn save_state(peer: &str, state: &HashMap<String, String>) {
    let _ = std::fs::create_dir_all(dir());
    let mut lines: Vec<String> = state.iter().map(|(k, h)| format!("{k}\t{h}")).collect();
    lines.sort();
    let _ = std::fs::write(state_file(peer), lines.join("\n") + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(game: &str, path: &str, t: i64, h: &str) -> SaveEntry {
        SaveEntry { game: game.into(), path: path.into(), mtime_ms: t, size: 1, hash: h.into() }
    }

    #[test]
    fn plan_copies_newer_and_follows_deletions() {
        let local = vec![e("G", "app/a", 100, "a1"), e("G", "app/b", 100, "b1"), e("G", "app/c", 500, "c2")];
        let remote = vec![e("G", "app/a", 100, "a1"), e("G", "app/c", 200, "c1"), e("G", "app/d", 50, "d1"), e("G", "app/e", 50, "e1")];
        // Last time both had b (b1) and e (e1); then they deleted b... no: they don't have b now.
        let mut last = HashMap::new();
        last.insert("G/app/b".to_string(), "b1".to_string());
        last.insert("G/app/e".to_string(), "e1".to_string());
        let steps = plan(&local, &remote, &last, 0, &[]);
        assert!(steps.contains(&Step::DeleteHere(e("G", "app/b", 100, "b1"))), "{steps:?}");
        assert!(steps.contains(&Step::Push(e("G", "app/c", 500, "c2"))));
        assert!(steps.contains(&Step::Pull(e("G", "app/d", 50, "d1"))));
        assert!(steps.contains(&Step::DeleteThere(e("G", "app/e", 50, "e1"))));
        assert_eq!(steps.len(), 4);
        // A running game is left alone.
        assert!(plan(&local, &remote, &last, 0, &["G".into()]).is_empty());
    }

    #[test]
    fn unchanged_side_loses_even_with_a_newer_clock() {
        let local = vec![e("G", "app/s", 900, "old")];
        let remote = vec![e("G", "app/s", 100, "new")];
        let mut last = HashMap::new();
        last.insert("G/app/s".to_string(), "old".to_string());
        assert_eq!(plan(&local, &remote, &last, 0, &[]), vec![Step::Pull(e("G", "app/s", 100, "new"))]);
    }

    #[test]
    fn unsafe_paths_are_refused() {
        assert!(safe_path("G", "app/x.sav").is_some());
        assert!(safe_path("G", "../x").is_none());
        assert!(safe_path("G", "app/../../x").is_none());
        assert!(safe_path("G", "backups/x").is_none());
        assert!(safe_path("..", "app/x").is_none());
        assert!(safe_path("G", "/etc/passwd").is_none());
        assert!(safe_path("G", "app\\x").is_none());
    }
}
