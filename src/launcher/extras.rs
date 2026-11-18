//! Launcher pages and jobs beyond the library: collections, a game's screenshots, "report a
//! problem", compatibility checks, save snapshots, play statistics, "what's new" after an
//! update, and the tests page (scenarios and Ctrl+R recordings).

use super::pages::{CARD_PAD, ROW_H, SECTION_GAP};
use super::view::{mix, rrect};
use super::*;
use std::collections::{HashMap, HashSet};

/// The extra pages (one at a time, over the library).
#[derive(Debug, Clone)]
pub(super) enum Extra {
    /// Collections of game `i` (and a field for a new one).
    Collections(usize),
    Screenshots(usize),
    /// "Report a problem" for game `i`: what goes into the report.
    Report(usize),
    /// Release notes of this version, shown once after an update.
    WhatsNew(String),
    Tests,
}

/// The tests page's state.
#[derive(Default)]
pub(super) struct Tests {
    /// Scenario -> (passed, output).
    pub results: HashMap<PathBuf, (bool, String)>,
    pub running: HashSet<PathBuf>,
    /// Scenario whose output is shown.
    pub open: Option<PathBuf>,
}

/// The folder the tests page lists by default: Ctrl+R recordings.
pub(super) fn recordings_dir() -> PathBuf {
    crate::runtime::files::data_root().join("recordings")
}

/// `s` split into lines that fit `max_w` at `size`.
fn wrap(l: &Launcher, s: &str, size: u16, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && l.text_width(&cand, size) > max_w {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = cand;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// Open a web page in the browser.
fn open_url(url: &str) {
    #[cfg(target_os = "android")]
    crate::android::open_url(url);
    #[cfg(not(target_os = "android"))]
    open_path(Path::new(url));
}

fn open_path(p: &Path) {
    let opener = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(p).spawn();
}

/// A game's screenshots (F12 / Ctrl+F12 in game), newest first: those the index says are
/// its own, and older ones named after it.
pub(super) fn screenshots_for(game: &Path, title: &str) -> Vec<PathBuf> {
    let dir = crate::runtime::files::data_root().join("screenshots");
    let mut mine: HashSet<String> = HashSet::new();
    if let Ok(text) = std::fs::read_to_string(dir.join("index.tsv")) {
        let want = std::path::absolute(game).unwrap_or_else(|_| game.to_path_buf()).to_string_lossy().to_lowercase();
        for l in text.lines() {
            if let Some((file, g)) = l.split_once('\t') {
                if g.to_lowercase() == want {
                    mine.insert(file.to_string());
                }
            }
        }
    }
    let clean: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let prefix = clean.trim().to_lowercase();
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            mine.contains(&name) || (!prefix.is_empty() && name.to_lowercase().starts_with(&format!("{prefix} 2")))
        })
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, p)| p).collect()
}

/// A zip archive of `files` (name inside, contents), deflated.
fn write_zip(path: &Path, files: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data)?;
        let packed = enc.finish()?;
        let offset = out.len() as u32;
        let head = |sig: &[u8], central: bool| -> Vec<u8> {
            let mut h = sig.to_vec();
            if central {
                h.extend_from_slice(&20u16.to_le_bytes()); // made by
            }
            h.extend_from_slice(&20u16.to_le_bytes()); // needed
            h.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
            h.extend_from_slice(&8u16.to_le_bytes()); // deflate
            h.extend_from_slice(&[0, 0, 0x21, 0]); // time, date (1980-01-01)
            h.extend_from_slice(&crc.sum().to_le_bytes());
            h.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // extra
            if central {
                h.extend_from_slice(&[0; 6]); // comment length, disk, internal attributes
                h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
                h.extend_from_slice(&offset.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend_from_slice(&head(b"PK\x03\x04", false));
        out.extend_from_slice(&packed);
        central.extend_from_slice(&head(b"PK\x01\x02", true));
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    std::fs::write(path, out)
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Launcher {
    fn close_extra(&mut self) {
        self.extra = None;
        self.collection_input = None;
        self.scroll = 0.0;
    }

    pub(super) fn open_extra(&mut self, e: Extra) {
        self.devices_page = false;
        self.steam_panel = false;
        self.settings_panel = None;
        self.app_settings = None;
        self.show_log = false;
        self.collection_input = None;
        self.extra = Some(e);
        self.scroll = 0.0;
    }

    pub(super) fn draw_extra(&mut self, top: f32) {
        match self.extra.clone() {
            Some(Extra::Collections(i)) => self.draw_collections(top, i),
            Some(Extra::Screenshots(i)) => self.draw_screenshots(top, i),
            Some(Extra::Report(i)) => self.draw_report(top, i),
            Some(Extra::WhatsNew(notes)) => self.draw_whats_new(top, &notes),
            Some(Extra::Tests) => self.draw_tests(top),
            None => {}
        }
    }

    // --- Collections -------------------------------------------------------------------

    fn draw_collections(&mut self, top: f32, i: usize) {
        let Some(e) = self.entries.get(i).cloned() else { return self.close_extra() };
        let (mut p, close) = self.page_start(top, &format!("Collections: {}", e.title), "Group games your way. The library's chips show one collection at a time.");
        let all = self.meta.collections();
        let rows: Vec<(String, usize)> = all.iter().map(|c| (c.clone(), self.meta.in_collection(&e.path, c) as usize)).collect();
        if !rows.is_empty() {
            let opts: [&str; 2] = ["Out", "In"];
            let cards: Vec<(&str, &str, &[&str], usize)> = rows.iter().map(|(c, k)| (c.as_str(), "", &opts[..], *k)).collect();
            if let Some((row, k)) = self.settings_card(&mut p, "COLLECTIONS", &cards) {
                let name = rows[row].0.clone();
                self.meta.set_collection(&e.path, &name, k == 1);
            }
        }
        // A new collection.
        self.section_title(&mut p, "NEW COLLECTION");
        let h = 74.0;
        rrect(p.x, p.y, p.w, h, 12.0, CARD());
        let mut text = self.collection_input.clone().unwrap_or_default();
        while let Some(c) = get_char_pressed() {
            if !c.is_control() && text.chars().count() < 40 {
                text.push(c);
            }
        }
        if is_key_pressed(KeyCode::Backspace) {
            text.pop();
        }
        let field = Rect::new(p.x + CARD_PAD, p.y + (h - 40.0) / 2.0, (p.w - 2.0 * CARD_PAD - 120.0).max(160.0), 40.0);
        rrect(field.x, field.y, field.w, field.h, 8.0, BG());
        let shown = if text.is_empty() { "Type a name, e.g. Favourites from the archive".to_string() } else { text.clone() };
        let shown = self.fit(&shown, 16, field.w - 24.0);
        self.text(&shown, field.x + 12.0, field.y + 26.0, 16, if text.is_empty() { MUTED() } else { TEXT() });
        if (get_time() * 2.0) as i64 % 2 == 0 {
            draw_rectangle(field.x + 13.0 + self.text_width(&text, 16), field.y + 10.0, 1.5, 20.0, TEXT());
        }
        let ok = !text.trim().is_empty();
        let add = Rect::new(field.x + field.w + 10.0, field.y, 100.0, 40.0);
        if (self.button(add, "Add", true, ok) || (ok && is_key_pressed(KeyCode::Enter))) && ok {
            self.meta.set_collection(&e.path, text.trim(), true);
            self.say(format!("Added {} to \"{}\".", e.title, text.trim()));
            text.clear();
        }
        self.collection_input = Some(text);
        p.y += h;
        self.page_scroll(top, p.y);
        if close {
            self.close_extra();
        }
    }

    // --- Screenshots -------------------------------------------------------------------

    fn shot_texture(&mut self, path: &Path) -> Option<Texture2D> {
        if let Some(t) = self.shot_textures.get(path) {
            return t.clone();
        }
        let t = std::fs::read(path).ok().and_then(|b| Texture2D::from_file_with_format(&b, None).into());
        self.shot_textures.insert(path.to_path_buf(), t.clone());
        t
    }

    fn draw_screenshots(&mut self, top: f32, i: usize) {
        let Some(e) = self.entries.get(i).cloned() else { return self.close_extra() };
        let shots = screenshots_for(&e.path, &e.title);
        let subtitle = format!("{} screenshot{}. Ctrl+F12 in game takes one; click one to open it.", shots.len(), if shots.len() == 1 { "" } else { "s" });
        let (mut p, close) = self.page_start(top, &format!("Screenshots: {}", e.title), &subtitle);
        if shots.is_empty() {
            rrect(p.x, p.y, p.w, 80.0, 12.0, CARD());
            self.text("None yet. Press Ctrl+F12 while playing to take one.", p.x + CARD_PAD, p.y + 46.0, 16, MUTED());
            p.y += 80.0;
        }
        let cols = if p.w > 640.0 { 3 } else { 2 };
        let gap = 14.0;
        let tw = (p.w - gap * (cols - 1) as f32) / cols as f32;
        let th = (tw * 9.0 / 16.0).round();
        let mut loaded_now = 0;
        let mut delete = None;
        for (k, s) in shots.iter().enumerate() {
            let (x, y) = (p.x + (k % cols) as f32 * (tw + gap), p.y + (k / cols) as f32 * (th + 44.0 + gap));
            if y + th + 44.0 < top || y > sh() {
                continue;
            }
            rrect(x, y, tw, th, 8.0, CARD());
            // A few new thumbnails per frame (each is a full PNG to decode).
            let tex = if self.shot_textures.contains_key(s) || loaded_now < 4 {
                loaded_now += 1;
                self.shot_texture(s)
            } else {
                None
            };
            if let Some(t) = tex {
                let (iw, ih) = (t.width(), t.height());
                let scale = (tw / iw).min(th / ih);
                let (dw, dh) = (iw * scale, ih * scale);
                draw_texture_ex(&t, x + (tw - dw) / 2.0, y + (th - dh) / 2.0, WHITE, DrawTextureParams { dest_size: Some(vec2(dw, dh)), ..Default::default() });
            }
            if self.tap(Rect::new(x, y, tw, th)) {
                open_path(s);
            }
            let name = s.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let when = name.rsplit_once(' ').map(|(a, b)| format!("{} {}", a.rsplit(' ').next().unwrap_or(""), b.replace('-', ":"))).unwrap_or(name);
            let when = self.fit(&when, 14, tw - 90.0);
            self.text(&when, x + 4.0, y + th + 26.0, 14, MUTED());
            let confirming = self.shot_delete.as_ref() == Some(s);
            if self.button(Rect::new(x + tw - 84.0, y + th + 8.0, 84.0, 30.0), if confirming { "Sure?" } else { "Delete" }, confirming, true) {
                if confirming {
                    delete = Some(s.clone());
                } else {
                    self.shot_delete = Some(s.clone());
                }
            }
        }
        p.y += shots.len().div_ceil(cols) as f32 * (th + 44.0 + gap);
        if let Some(s) = delete {
            let _ = std::fs::remove_file(&s);
            self.shot_textures.remove(&s);
            self.shot_delete = None;
        }
        self.page_scroll(top, p.y);
        if close {
            self.shot_textures.clear();
            self.close_extra();
        }
    }

    // --- Report a problem --------------------------------------------------------------

    fn draw_report(&mut self, top: f32, i: usize) {
        let Some(e) = self.entries.get(i).cloned() else { return self.close_extra() };
        let (mut p, close) = self.page_start(top, &format!("Report a problem: {}", e.title), "Saves a zip with what's needed to look into it. Nothing is sent anywhere by itself.");
        let save_root = crate::runtime::files::save_root_for(&e.path, &e.title);
        let shot = screenshots_for(&e.path, &e.title).into_iter().next();
        let recording = latest_recording(&e.path);
        let compat = self.meta.get(&e.path).compat;
        let items = [
            ("The game's log", save_root.join("klikcore.log").is_file()),
            ("Its compatibility check (unsupported features)", compat.is_some()),
            ("Its newest screenshot", shot.is_some()),
            ("Your newest Ctrl+R recording of it", recording.is_some()),
            ("KlikCore's version and your system (Windows / Linux, no personal details)", true),
        ];
        let h = CARD_PAD * 2.0 + items.len() as f32 * 28.0 + 8.0;
        rrect(p.x, p.y, p.w, h, 12.0, CARD());
        for (k, (label, have)) in items.iter().enumerate() {
            let y = p.y + CARD_PAD + 18.0 + k as f32 * 28.0;
            self.text(if *have { "+" } else { "-" }, p.x + CARD_PAD, y, 16, if *have { ACCENT() } else { MUTED() });
            let l = if *have { label.to_string() } else { format!("{label} (none yet)") };
            let l = self.fit(&l, 16, p.w - 2.0 * CARD_PAD - 24.0);
            self.text(&l, p.x + CARD_PAD + 22.0, y, 16, if *have { TEXT() } else { MUTED() });
        }
        p.y += h + 16.0;
        let hint = "Tip: press Ctrl+R in game, do what goes wrong, then Ctrl+R again: the recording lets it be replayed exactly.";
        for l in wrap(self, hint, 15, p.w) {
            self.text(&l, p.x, p.y + 14.0, 15, MUTED());
            p.y += 22.0;
        }
        p.y += 12.0;
        let idle = self.busy.is_none();
        let save = Rect::new(p.x, p.y, 200.0, 44.0);
        let issue = Rect::new(p.x + 212.0, p.y, 300.0, 44.0);
        let mut go = None;
        if self.button(save, "Save the report", true, idle) {
            go = Some(false);
        }
        if self.button(issue, "Save it and open a GitHub issue", false, idle) {
            go = Some(true);
        }
        p.y += 60.0;
        if let Some(issue) = go {
            self.make_report(i, issue);
        }
        self.page_scroll(top, p.y);
        if close {
            self.close_extra();
        }
    }

    /// Build the report zip in the background; with `issue`, also open a pre-filled GitHub
    /// issue (title and system only; the zip is attached by hand).
    fn make_report(&mut self, i: usize, issue: bool) {
        let Some(e) = self.entries.get(i).cloned() else { return };
        let compat = self.meta.get(&e.path).compat;
        let tx = self.tx.clone();
        self.busy = Some("Saving the report...".into());
        std::thread::spawn(move || {
            let save_root = crate::runtime::files::save_root_for(&e.path, &e.title);
            let mut files: Vec<(String, Vec<u8>)> = Vec::new();
            if let Ok(b) = std::fs::read(save_root.join("klikcore.log")) {
                files.push(("klikcore.log".into(), b));
            }
            if let Ok(b) = std::fs::read(save_root.join("events.log")) {
                files.push(("events.log".into(), b));
            }
            if let Some(s) = screenshots_for(&e.path, &e.title).into_iter().next() {
                if let Ok(b) = std::fs::read(&s) {
                    files.push((format!("screenshot/{}", s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), b));
                }
            }
            if let Some(r) = latest_recording(&e.path) {
                if let Ok(b) = std::fs::read(&r) {
                    files.push((format!("recording/{}", r.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), b));
                }
            }
            let system = format!(
                "KlikCore {}\nSystem: {} {}\nGame: {}\nGame file: {}\nCompatibility check: {}\n",
                update::current_version(),
                std::env::consts::OS,
                std::env::consts::ARCH,
                e.title,
                e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                compat.as_ref().map_or("not run".into(), |c| format!("{} unsupported ({}) on {}", c.unsupported, c.details, c.checked)),
            );
            files.push(("about.txt".into(), system.clone().into_bytes()));
            let dir = crate::runtime::files::data_root().join("reports");
            let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
            let stem: String = e.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
            let path = dir.join(format!("{} {stamp}.zip", stem.trim()));
            let r = std::fs::create_dir_all(&dir).and_then(|_| write_zip(&path, &files)).map_err(|e| e.to_string());
            // Android: the app's own folder can't be opened, so the zip is copied where the
            // player (and the browser, to attach it) can find it.
            #[cfg(target_os = "android")]
            let shown = r.is_ok().then(|| {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                crate::android::save_to_downloads(&path, &name, "application/zip")
            }).flatten();
            #[cfg(not(target_os = "android"))]
            let shown: Option<String> = None;
            if r.is_ok() {
                if !cfg!(target_os = "android") {
                    open_path(&dir);
                }
                if issue {
                    let title = url_encode(&format!("Problem in {}", e.title));
                    let body = url_encode(&format!("What happens:\n\nWhat should happen:\n\n(Attach the report zip: {})\n\n{system}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                    open_url(&format!("https://github.com/Jackhiles/KlikCore2/issues/new?title={title}&body={body}"));
                }
            }
            let _ = tx.send(Msg::Done(match (r, shown) {
                (Ok(()), Some(place)) => format!("Report saved to {place}"),
                (Ok(()), None) => format!("Report saved: {}", path.display()),
                (Err(err), _) => format!("Could not save the report: {err}"),
            }));
        });
    }

    // --- Compatibility check -----------------------------------------------------------

    /// Run every frame of game `i` for a while in the background (a separate KlikCore
    /// process, so a crash can't take the launcher down) and keep the result.
    pub(super) fn check_compat(&mut self, i: usize) {
        let Some(e) = self.entries.get(i).cloned() else { return };
        if !self.checking.insert(e.path.clone()) {
            return;
        }
        let tx = self.tx.clone();
        self.say(format!("Checking {}: running every screen for a few seconds...", e.title));
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Compat(e.path.clone(), sweep(&e.path)));
        });
    }

    /// Check every game in the library, two at a time, then say how they did.
    pub(super) fn check_all_compat(&mut self) {
        if self.compat_batch.is_some() {
            self.say("Already checking every game.");
            return;
        }
        let paths: Vec<PathBuf> = self
            .entries
            .iter()
            .map(|e| e.path.clone())
            .filter(|p| p.is_file() && !self.checking.contains(p))
            .collect();
        if paths.is_empty() {
            self.say("No games to check.");
            return;
        }
        self.checking.extend(paths.iter().cloned());
        self.compat_batch = Some(CompatBatch { total: paths.len(), ..Default::default() });
        self.say(format!("Checking {} games (each runs every screen for a few seconds)...", paths.len()));
        let queue = std::sync::Arc::new(std::sync::Mutex::new(paths));
        for _ in 0..if cfg!(target_os = "android") { 1 } else { 2 } {
            let (queue, tx) = (queue.clone(), self.tx.clone());
            std::thread::spawn(move || loop {
                let Some(p) = queue.lock().ok().and_then(|mut q| q.pop()) else { break };
                let r = sweep(&p);
                if tx.send(Msg::Compat(p, r)).is_err() {
                    break;
                }
            });
        }
    }

    /// A compatibility result arrived while checking every game: count it; at the end,
    /// sum up. Returns false if no such check is running.
    pub(super) fn batch_result(&mut self, c: Option<&super::meta::Compat>, title: &str) -> bool {
        let Some(b) = self.compat_batch.as_mut() else { return false };
        b.done += 1;
        match c {
            Some(c) if c.unsupported == 0 => b.clean += 1,
            Some(_) => b.partial.push(title.to_string()),
            None => b.failed.push(title.to_string()),
        }
        if b.done < b.total {
            let msg = format!("Checking games: {} of {} done...", b.done, b.total);
            self.say(msg);
            return true;
        }
        let b = self.compat_batch.take().expect("checked");
        let mut msg = format!("Checked {} games: {} fully supported", b.total, b.clean);
        if !b.partial.is_empty() {
            msg += &format!(", {} with unsupported features ({})", b.partial.len(), b.partial.join(", "));
        }
        if !b.failed.is_empty() {
            msg += &format!(", {} didn't load ({})", b.failed.len(), b.failed.join(", "));
        }
        self.say(msg + ".");
        true
    }

    // --- What's new --------------------------------------------------------------------

    /// After an update: fetch this version's release notes once (in the background).
    pub(super) fn check_whats_new(&self) {
        let v = update::current_version().to_string();
        if self.config.last_seen_version == v || update::is_dev_build() && std::env::var_os("KLIKCORE_WHATS_NEW").is_none() {
            return;
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if let Ok(notes) = update::release_notes(&v) {
                if !notes.trim().is_empty() {
                    let _ = tx.send(Msg::WhatsNew(notes));
                }
            }
        });
    }

    fn draw_whats_new(&mut self, top: f32, notes: &str) {
        let (mut p, close) = self.page_start(top, &format!("What's new in KlikCore {}", update::current_version()), "");
        let inner = p.w - 2.0 * CARD_PAD;
        // Lay out first (for the card's height), then draw.
        let mut lines: Vec<(String, u16, Color, f32)> = Vec::new();
        for raw in notes.lines() {
            let l = raw.trim_end();
            if l.trim().is_empty() {
                lines.push((String::new(), 8, MUTED(), 0.0));
                continue;
            }
            let (text, size, color, indent) = if let Some(h) = l.strip_prefix("## ").or_else(|| l.strip_prefix("# ")) {
                (h.to_string(), 19, ACCENT(), 0.0)
            } else if let Some(b) = l.trim_start().strip_prefix("- ") {
                (format!("\u{2022} {b}"), 16, TEXT(), 12.0)
            } else {
                (l.to_string(), 16, TEXT(), 0.0)
            };
            let text = text.replace("**", "").replace('`', "");
            for (k, w) in wrap(self, &text, size, inner - indent - 14.0).into_iter().enumerate() {
                lines.push((w, size, color, indent + if k > 0 { 14.0 } else { 0.0 }));
            }
        }
        let h: f32 = lines.iter().map(|l| if l.1 == 8 { 10.0 } else { l.1 as f32 + 10.0 }).sum::<f32>() + 2.0 * CARD_PAD;
        rrect(p.x, p.y, p.w, h, 12.0, CARD());
        let mut y = p.y + CARD_PAD;
        for (t, size, color, indent) in &lines {
            if *size == 8 {
                y += 10.0;
                continue;
            }
            y += *size as f32 + 10.0;
            if *size == 19 {
                self.bold_text(t, p.x + CARD_PAD + indent, y - 6.0, *size, *color);
            } else {
                self.text(t, p.x + CARD_PAD + indent, y - 6.0, *size, *color);
            }
        }
        p.y += h + 16.0;
        let ok = Rect::new(p.x, p.y, 140.0, 44.0);
        let done = self.button(ok, "Got it", true, true);
        p.y += 60.0;
        self.page_scroll(top, p.y);
        if close || done {
            self.config.last_seen_version = update::current_version().to_string();
            self.config.save();
            self.close_extra();
        }
    }

    // --- Tests -------------------------------------------------------------------------

    fn tests_dir(&self) -> PathBuf {
        self.config.tests_dir.clone().unwrap_or_else(recordings_dir)
    }

    fn scenarios(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(self.tests_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("scn")))
            .collect();
        v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        v
    }

    /// Run every scenario in the tests folder.
    pub(super) fn run_all_tests(&mut self) {
        let files = self.scenarios();
        self.run_tests(files);
    }

    fn run_tests(&mut self, files: Vec<PathBuf>) {
        let files: Vec<PathBuf> = files.into_iter().filter(|f| self.tests.running.insert(f.clone())).collect();
        if files.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            for f in files {
                let out = std::env::current_exe().and_then(|exe| {
                    let mut c = std::process::Command::new(exe);
                    c.arg("--scenario").arg(&f);
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        c.creation_flags(0x0800_0000);
                    }
                    c.output()
                });
                let (ok, text) = match out {
                    Ok(o) => {
                        let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                        (o.status.success() && text.contains("PASS") && !text.contains("FAIL"), text)
                    }
                    Err(e) => (false, format!("could not run: {e}")),
                };
                let _ = tx.send(Msg::TestResult(f, ok, text));
            }
        });
    }

    fn draw_tests(&mut self, top: f32) {
        let dir = self.tests_dir();
        let files = self.scenarios();
        let passed = files.iter().filter(|f| self.tests.results.get(*f).is_some_and(|r| r.0)).count();
        let failed = files.iter().filter(|f| self.tests.results.get(*f).is_some_and(|r| !r.0)).count();
        let subtitle = format!("{} scenario{}  -  {passed} passed, {failed} failed", files.len(), if files.len() == 1 { "" } else { "s" });
        let (mut p, close) = self.page_start(top, "Tests", &subtitle);

        // Folder row.
        rrect(p.x, p.y, p.w, ROW_H, 12.0, CARD());
        self.text("Folder", p.x + CARD_PAD, p.y + 31.0, 17, TEXT());
        let choose = Rect::new(p.x + p.w - CARD_PAD - 120.0, p.y + (ROW_H - 38.0) / 2.0, 120.0, 38.0);
        let all = Rect::new(choose.x - 130.0, choose.y, 120.0, 38.0);
        let shown = self.fit_left(&dir.display().to_string(), 14, all.x - p.x - CARD_PAD - 16.0);
        self.text(&shown, p.x + CARD_PAD, p.y + 53.0, 14, MUTED());
        if self.button(all, "Run all", true, !files.is_empty() && self.tests.running.is_empty()) {
            self.run_tests(files.clone());
        }
        if self.button(choose, "Choose...", false, true) {
            if let Some(d) = dialogs::pick_folder() {
                self.config.tests_dir = Some(d);
                self.config.save();
            }
        }
        p.y += ROW_H + SECTION_GAP;

        if files.is_empty() {
            rrect(p.x, p.y, p.w, 96.0, 12.0, CARD());
            self.text("No scenarios here yet.", p.x + CARD_PAD, p.y + 38.0, 16, TEXT());
            let hint = self.fit("Record one in game: Ctrl+R, play, Ctrl+R again. Or choose your scenarios folder.", 14, p.w - 2.0 * CARD_PAD);
            self.text(&hint, p.x + CARD_PAD, p.y + 64.0, 14, MUTED());
            p.y += 96.0;
        }
        let row = 52.0;
        let h = files.len() as f32 * row + 8.0;
        if !files.is_empty() {
            rrect(p.x, p.y, p.w, h, 12.0, CARD());
        }
        let mut run = None;
        let mut toggle = None;
        for (k, f) in files.iter().enumerate() {
            let y = p.y + 4.0 + k as f32 * row;
            if k > 0 {
                draw_line(p.x + CARD_PAD, y, p.x + p.w - CARD_PAD, y, 1.0, mix(CARD(), TEXT(), 0.05));
            }
            let (label, color) = if self.tests.running.contains(f) {
                ("running", ACCENT())
            } else {
                match self.tests.results.get(f) {
                    Some((true, _)) => ("PASS", ACCENT()),
                    Some((false, _)) => ("FAIL", DANGER()),
                    None => ("-", MUTED()),
                }
            };
            self.bold_text(label, p.x + CARD_PAD, y + 32.0, 14, color);
            let name = f.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let name = self.fit(&name, 16, p.w - 2.0 * CARD_PAD - 80.0 - 190.0);
            self.text(&name, p.x + CARD_PAD + 80.0, y + 32.0, 16, TEXT());
            let rb = Rect::new(p.x + p.w - CARD_PAD - 80.0, y + 8.0, 80.0, 36.0);
            let lb = Rect::new(rb.x - 90.0, y + 8.0, 80.0, 36.0);
            if self.button(rb, "Run", false, !self.tests.running.contains(f)) {
                run = Some(f.clone());
            }
            if self.tests.results.contains_key(f) && self.button(lb, if self.tests.open.as_ref() == Some(f) { "Hide" } else { "Output" }, false, true) {
                toggle = Some(f.clone());
            }
        }
        p.y += h;
        if let Some(f) = run {
            self.run_tests(vec![f]);
        }
        if let Some(f) = toggle {
            self.tests.open = if self.tests.open.as_ref() == Some(&f) { None } else { Some(f) };
        }
        // Output of the opened scenario.
        if let Some((_, text)) = self.tests.open.as_ref().and_then(|f| self.tests.results.get(f)).cloned() {
            p.y += SECTION_GAP;
            self.section_title(&mut p, "OUTPUT");
            let lines: Vec<&str> = text.lines().take(60).collect();
            let h = lines.len() as f32 * 20.0 + 2.0 * CARD_PAD;
            rrect(p.x, p.y, p.w, h, 12.0, CARD());
            for (k, l) in lines.iter().enumerate() {
                let l = self.fit(l, 13, p.w - 2.0 * CARD_PAD);
                let c = if l.contains("FAIL") { DANGER() } else if l.contains("PASS") { ACCENT() } else { mix(TEXT(), MUTED(), 0.3) };
                self.text(&l, p.x + CARD_PAD, p.y + CARD_PAD + 14.0 + k as f32 * 20.0, 13, c);
            }
            p.y += h;
        }
        self.page_scroll(top, p.y);
        if close {
            self.close_extra();
        }
    }

    // --- Save snapshots (on a game's settings page) -----------------------------------

    pub(super) fn snapshots_card(&mut self, i: usize, p: &mut super::pages::Page) {
        use crate::runtime::save_sync;
        let Some(entry) = self.entries.get(i).cloned() else { return };
        let fs = crate::runtime::files::SaveFs::new(&entry.path, crate::runtime::files::save_root_for(&entry.path, &entry.title))
            .with_prefix(super::steam_shortcuts::proton_prefix(&entry.path));
        let snaps = save_sync::snapshots(&fs);
        let shown = snaps.len().min(8);
        let row = 52.0;
        let h = ROW_H + shown as f32 * row + if shown > 0 { 8.0 } else { 0.0 };
        rrect(p.x, p.y, p.w, h, 12.0, CARD());
        self.text("Snapshots", p.x + CARD_PAD, p.y + 31.0, 17, TEXT());
        let hint = self.fit("A copy of KlikCore's saves to go back to. Copies between games appear here too.", 14, p.w - 2.0 * CARD_PAD - 190.0);
        self.text(&hint, p.x + CARD_PAD, p.y + 53.0, 14, MUTED());
        let take = Rect::new(p.x + p.w - CARD_PAD - 170.0, p.y + (ROW_H - 38.0) / 2.0, 170.0, 38.0);
        if self.button(take, "Take snapshot", true, true) {
            match save_sync::take_snapshot(&fs) {
                Ok(Some(_)) => self.say(format!("Snapshot of {}'s saves taken.", entry.title)),
                Ok(None) => self.say("Nothing to snapshot yet: the game hasn't saved anything in KlikCore."),
                Err(e) => self.say(format!("Snapshot failed: {e}")),
            }
        }
        let mut restore = None;
        for (k, s) in snaps.iter().take(shown).enumerate() {
            let y = p.y + ROW_H + k as f32 * row;
            draw_line(p.x + CARD_PAD, y, p.x + p.w - CARD_PAD, y, 1.0, mix(CARD(), TEXT(), 0.05));
            let synced = s.stamp.starts_with("sync-");
            let when = s.stamp.trim_start_matches("sync-");
            let when = match when.split_once(' ') {
                Some((d, t)) => format!("{d}  {}", t.replace('-', ":").trim_end_matches('+')),
                None => when.to_string(),
            };
            let when = if synced { format!("{when}  (before a sync)") } else { when };
            self.text(&when, p.x + CARD_PAD, y + 31.0, 16, TEXT());
            let what = match (s.klikcore_files, s.original_files) {
                (k, 0) => format!("{k} KlikCore file{}", if k == 1 { "" } else { "s" }),
                (0, o) => format!("{o} original game file{}", if o == 1 { "" } else { "s" }),
                (k, o) => format!("{k} KlikCore + {o} original"),
            };
            self.text(&what, p.x + CARD_PAD + if synced { 330.0 } else { 200.0 }, y + 31.0, 14, MUTED());
            let confirming = self.snapshot_confirm.as_ref() == Some(&s.dir);
            let r = Rect::new(p.x + p.w - CARD_PAD - 120.0, y + 8.0, 120.0, 36.0);
            if self.button(r, if confirming { "Sure?" } else { "Restore" }, confirming, true) {
                if confirming {
                    restore = Some(s.dir.clone());
                } else {
                    self.snapshot_confirm = Some(s.dir.clone());
                }
            }
        }
        p.y += h;
        if let Some(dir) = restore {
            self.snapshot_confirm = None;
            if self.is_running(&entry.path) {
                self.say(format!("Close {} first, then restore its saves.", entry.title));
                return;
            }
            let r = save_sync::restore(&fs, &dir);
            self.saves_status = None;
            if r.errors.is_empty() {
                self.say(format!("Restored {} file{} (your saves before that are in a new snapshot).", r.copied, if r.copied == 1 { "" } else { "s" }));
            } else {
                self.say(format!("Restored {}, but {} failed: {}", r.copied, r.errors.len(), r.errors[0]));
            }
        }
    }

    // --- Play statistics (top of the play log) -------------------------------------------

    /// Totals, the most played games and the last 30 days, as a card at `p`.
    pub(super) fn stats_card(&mut self, p: &mut super::pages::Page) {
        if self.sessions.is_empty() {
            return;
        }
        let total: u64 = self.sessions.iter().map(|s| s.seconds).sum();
        let mut per_game: Vec<(String, u64)> = {
            let mut m: HashMap<String, (String, u64)> = HashMap::new();
            for s in &self.sessions {
                let e = m.entry(s.path.to_string_lossy().to_lowercase()).or_insert_with(|| (s.title.clone(), 0));
                e.1 += s.seconds;
            }
            m.into_values().collect()
        };
        per_game.sort_by(|a, b| b.1.cmp(&a.1));
        let today = chrono::Local::now().date_naive();
        let mut days = [0u64; 30];
        for s in &self.sessions {
            if let Ok(d) = chrono::NaiveDate::parse_from_str(s.started.get(..10).unwrap_or(""), "%Y-%m-%d") {
                let ago = (today - d).num_days();
                if (0..30).contains(&ago) {
                    days[29 - ago as usize] += s.seconds;
                }
            }
        }
        let top = per_game.len().min(6);
        let bar_row = 30.0;
        let chart_h = 90.0;
        let h = CARD_PAD + 30.0 + top as f32 * bar_row + 24.0 + chart_h + 40.0 + CARD_PAD;
        rrect(p.x, p.y, p.w, h, 12.0, CARD());
        let x0 = p.x + CARD_PAD;
        let inner = p.w - 2.0 * CARD_PAD;
        let summary = format!("{} played  -  {} games  -  {} sessions", playlog::duration(total), per_game.len(), self.sessions.len());
        self.bold_text(&summary, x0, p.y + CARD_PAD + 16.0, 17, TEXT());
        let mut y = p.y + CARD_PAD + 36.0;
        let most = per_game.first().map_or(1, |g| g.1.max(1));
        let label_w = (inner * 0.38).min(300.0);
        for (title, secs) in per_game.iter().take(top) {
            let t = self.fit(title, 15, label_w - 10.0);
            self.text(&t, x0, y + 17.0, 15, TEXT());
            let bw = (inner - label_w - 80.0) * (*secs as f32 / most as f32);
            rrect(x0 + label_w, y + 5.0, bw.max(4.0), 16.0, 6.0, ACCENT());
            self.text(&playlog::duration(*secs), x0 + label_w + bw + 10.0, y + 18.0, 14, MUTED());
            y += bar_row;
        }
        y += 14.0;
        self.text("Last 30 days", x0, y + 10.0, 14, MUTED());
        y += 18.0;
        let peak = days.iter().copied().max().unwrap_or(0).max(60);
        let bw = inner / 30.0;
        for (k, &d) in days.iter().enumerate() {
            let bh = (chart_h - 10.0) * d as f32 / peak as f32;
            let c = if d > 0 { ACCENT() } else { mix(CARD(), TEXT(), 0.08) };
            let bh = if d > 0 { bh.max(3.0) } else { 2.0 };
            draw_rectangle(x0 + k as f32 * bw + 2.0, y + chart_h - bh, bw - 4.0, bh, c);
        }
        self.text("30 days ago", x0, y + chart_h + 20.0, 13, MUTED());
        let t = "today";
        self.text(t, x0 + inner - self.text_width(t, 13), y + chart_h + 20.0, 13, MUTED());
        p.y += h + SECTION_GAP;
    }
}

/// Files found not to be games in watched folders (not inspected again), one path per line.
fn not_games_file() -> PathBuf {
    crate::runtime::files::data_root().join("not-games.txt")
}

impl Launcher {
    /// A one-line text field named `name`: click to type into it; Enter or a click
    /// elsewhere ends typing. Returns true when the text changed.
    fn text_field(&mut self, name: &'static str, value: &mut String, r: Rect, placeholder: &str, accept: fn(char) -> bool) -> bool {
        let focused = self.focused_field == Some(name);
        rrect(r.x, r.y, r.w, r.h, 8.0, if focused { mix(BG(), ACCENT(), 0.12) } else { BG() });
        let mut changed = false;
        if focused {
            while let Some(c) = get_char_pressed() {
                if accept(c) && value.chars().count() < 200 {
                    value.push(c);
                    changed = true;
                }
            }
            if is_key_pressed(KeyCode::Backspace) && value.pop().is_some() {
                changed = true;
            }
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Escape) {
                self.focused_field = None;
            }
        }
        if self.tap(r) {
            self.focused_field = Some(name);
        } else if focused && is_mouse_button_pressed(MouseButton::Left) && !r.contains(mouse()) {
            self.focused_field = None;
        }
        let shown = if value.is_empty() && !focused { placeholder.to_string() } else { value.clone() };
        let shown = self.fit_left(&shown, 15, r.w - 24.0);
        self.text(&shown, r.x + 12.0, r.y + r.h / 2.0 + 6.0, 15, if value.is_empty() && !focused { MUTED() } else { TEXT() });
        if focused && (get_time() * 2.0) as i64 % 2 == 0 {
            draw_rectangle(r.x + 13.0 + self.text_width(&shown, 15), r.y + 9.0, 1.5, r.h - 18.0, TEXT());
        }
        changed
    }

    /// A game's launch options (its settings page). Returns true when something changed.
    pub(super) fn launch_card(&mut self, p: &mut super::pages::Page, s: &mut Settings) -> bool {
        let mut changed = false;
        self.section_title(p, "LAUNCH");
        let rows = 4.0;
        rrect(p.x, p.y, p.w, rows * ROW_H, 12.0, CARD());
        let field_w = (p.w * 0.42).min(320.0);
        let fx = p.x + p.w - CARD_PAD - field_w;
        // Start frame.
        self.text("Start on frame", p.x + CARD_PAD, p.y + 31.0, 17, TEXT());
        self.text("0 starts the game normally; a number skips straight to that frame", p.x + CARD_PAD, p.y + 53.0, 14, MUTED());
        let mut frame = if s.start_frame == 0 { String::new() } else { s.start_frame.to_string() };
        if self.text_field("start_frame", &mut frame, Rect::new(fx, p.y + 17.0, field_w, 40.0), "0", |c| c.is_ascii_digit()) {
            s.start_frame = frame.parse().unwrap_or(0).min(9999);
            changed = true;
        }
        p.y += ROW_H;
        draw_line(p.x + CARD_PAD, p.y, p.x + p.w - CARD_PAD, p.y, 1.0, mix(CARD(), TEXT(), 0.06));
        // Extra options.
        self.text("Extra options", p.x + CARD_PAD, p.y + 31.0, 17, TEXT());
        let hint = self.fit("Command-line options added when it starts, e.g. --mute or --fullscreen", 14, fx - p.x - CARD_PAD - 12.0);
        self.text(&hint, p.x + CARD_PAD, p.y + 53.0, 14, MUTED());
        let mut args = s.launch_args.clone();
        if self.text_field("launch_args", &mut args, Rect::new(fx, p.y + 17.0, field_w, 40.0), "none", |c| !c.is_control()) {
            s.launch_args = args;
            changed = true;
        }
        p.y += ROW_H;
        // Debug panels at the start.
        let opts: [&str; 4] = ["Off", "Variables", "Events", "Both"];
        let row: [(&str, &str, &[&str], usize); 1] = [("Debug panels at start", "Ctrl+G / Ctrl+E in game", &opts, s.panels.min(3) as usize)];
        if let Some(k) = self.setting(p, false, row[0].0, row[0].1, row[0].2, row[0].3) {
            s.panels = k as u8;
            changed = true;
        }
        // Detailed event log.
        let opts: [&str; 4] = ["Off", "1 min", "5 min", "Session"];
        let hint = "Every event and what it changed, saved as events.log (Ctrl+L saves it now)";
        if let Some(k) = self.setting(p, false, "Event log", hint, &opts, s.event_log.min(3) as usize) {
            s.event_log = k as u8;
            changed = true;
        }
        p.y += SECTION_GAP;
        changed
    }

    /// Settings page: folders scanned for new games when the launcher starts.
    pub(super) fn watched_card(&mut self, p: &mut super::pages::Page) {
        self.section_title(p, "WATCHED FOLDERS");
        let dirs = self.config.watch_dirs.clone();
        let row = 52.0;
        let h = ROW_H + dirs.len() as f32 * row + if dirs.is_empty() { 0.0 } else { 8.0 };
        rrect(p.x, p.y, p.w, h, 12.0, CARD());
        self.text("New games appear on their own", p.x + CARD_PAD, p.y + 31.0, 17, TEXT());
        let hint = self.fit("Games added to these folders join the library when KlikCore starts", 14, p.w - 2.0 * CARD_PAD - 290.0);
        self.text(&hint, p.x + CARD_PAD, p.y + 53.0, 14, MUTED());
        let add = Rect::new(p.x + p.w - CARD_PAD - 140.0, p.y + (ROW_H - 38.0) / 2.0, 140.0, 38.0);
        let scan = Rect::new(add.x - 130.0, add.y, 120.0, 38.0);
        if self.button(scan, "Scan now", false, !dirs.is_empty() && self.busy.is_none()) {
            self.scan_watched(true);
        }
        if self.button(add, "Add folder...", true, true) {
            if let Some(d) = dialogs::pick_folder() {
                if !self.config.watch_dirs.contains(&d) {
                    self.config.watch_dirs.push(d);
                    self.config.save();
                    self.scan_watched(true);
                }
            }
        }
        let mut remove = None;
        for (k, d) in dirs.iter().enumerate() {
            let y = p.y + ROW_H + k as f32 * row;
            draw_line(p.x + CARD_PAD, y, p.x + p.w - CARD_PAD, y, 1.0, mix(CARD(), TEXT(), 0.05));
            let shown = self.fit_left(&d.display().to_string(), 15, p.w - 2.0 * CARD_PAD - 130.0);
            self.text(&shown, p.x + CARD_PAD, y + 31.0, 15, if d.is_dir() { TEXT() } else { MUTED() });
            if self.button(Rect::new(p.x + p.w - CARD_PAD - 110.0, y + 8.0, 110.0, 36.0), "Remove", false, true) {
                remove = Some(k);
            }
        }
        p.y += h + SECTION_GAP;
        if let Some(k) = remove {
            self.config.watch_dirs.remove(k);
            self.config.save();
        }
    }

    /// Look through the watched folders for games not in the library yet, in the
    /// background. `announce`: say what happened even when nothing was found.
    pub(super) fn scan_watched(&mut self, announce: bool) {
        let dirs = self.config.watch_dirs.clone();
        if dirs.is_empty() {
            return;
        }
        let known: HashSet<String> = self.entries.iter().map(|e| e.path.to_string_lossy().to_lowercase()).collect();
        let tx = self.tx.clone();
        if announce {
            self.busy = Some("Looking for new games in your watched folders...".into());
        }
        std::thread::spawn(move || {
            let skip: HashSet<String> = std::fs::read_to_string(not_games_file()).unwrap_or_default().lines().map(|l| l.trim().to_lowercase()).collect();
            let mut not_games = Vec::new();
            let mut found = 0;
            for d in &dirs {
                for f in library::find_games(d) {
                    let key = f.to_string_lossy().to_lowercase();
                    if known.contains(&key) || skip.contains(&key) {
                        continue;
                    }
                    match library::inspect(&f) {
                        Some(e) => {
                            found += 1;
                            let _ = tx.send(Msg::Found(e));
                        }
                        None => not_games.push(f.display().to_string()),
                    }
                }
            }
            if !not_games.is_empty() {
                use std::io::Write as _;
                if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(not_games_file()) {
                    for n in not_games {
                        let _ = writeln!(out, "{n}");
                    }
                }
            }
            if found > 0 || announce {
                let _ = tx.send(Msg::Done(match found {
                    0 => "No new games in your watched folders.".into(),
                    1 => "Added 1 new game from your watched folders.".into(),
                    k => format!("Added {k} new games from your watched folders."),
                }));
            }
        });
    }
}

/// The newest Ctrl+R recording of a game (by file name).
pub(super) fn latest_recording(game: &Path) -> Option<PathBuf> {
    let stem = game.file_stem()?.to_string_lossy().to_lowercase();
    std::fs::read_dir(recordings_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with(&format!("{stem}-frame"))))
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_options_split_with_quotes() {
        assert_eq!(super::super::split_args("--frame 3  --mute"), vec!["--frame", "3", "--mute"]);
        assert_eq!(super::super::split_args("--save-dir \"C:/My Saves\" -v"), vec!["--save-dir", "C:/My Saves", "-v"]);
        assert!(super::super::split_args("   ").is_empty());
    }

    #[test]
    fn zip_is_readable() {
        let dir = std::env::temp_dir().join(format!("klikcore-zip-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("r.zip");
        write_zip(&path, &[("a.txt".into(), b"hello hello hello".to_vec()), ("b/c.txt".into(), b"x".to_vec())]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let files = super::super::upscale::unzip_for_tests(&bytes);
        assert_eq!(files, vec![("a.txt".to_string(), b"hello hello hello".to_vec()), ("b/c.txt".to_string(), b"x".to_vec())]);
        assert_eq!(url_encode("a b&c"), "a%20b%26c");
    }
}

/// Progress of "Check all games".
#[derive(Default)]
pub(super) struct CompatBatch {
    pub total: usize,
    pub done: usize,
    pub clean: usize,
    pub partial: Vec<String>,
    pub failed: Vec<String>,
}

/// Run every frame of a game for a while in a separate KlikCore process (so a crash can't
/// take the launcher down) and read the result.
#[cfg(target_os = "android")]
fn sweep(game: &Path) -> Option<super::meta::Compat> {
    // No separate processes on Android: load and run the game here (in the worker thread),
    // catching a crash so it only fails this check.
    let game = game.to_path_buf();
    let text = std::panic::catch_unwind(move || {
        let data = crate::ctf_loader::GameData::load(&game).ok()?;
        let dir = crate::runtime::files::data_root().join("compat-check");
        let text = crate::runtime::scenario::sweep(data, 600, dir.clone());
        let _ = std::fs::remove_dir_all(&dir);
        Some(text)
    })
    .ok()
    .flatten()?;
    super::meta::parse_sweep(&text)
}

#[cfg(not(target_os = "android"))]
fn sweep(game: &Path) -> Option<super::meta::Compat> {
    let out = std::env::current_exe().and_then(|exe| {
        let mut c = std::process::Command::new(exe);
        c.arg(game).args(["--sweep", "600", "--mute"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        c.output()
    });
    let text = out.map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))).unwrap_or_default();
    super::meta::parse_sweep(&text)
}
