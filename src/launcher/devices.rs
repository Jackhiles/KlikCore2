//! The Devices page (pair with Rekindle on other devices, sync saves, copy games) and the
//! pairing prompt shown over any screen.

use super::pages::{CARD_PAD, SECTION_GAP};
use super::view::{mix, rrect};
use super::*;
use crate::sync::{identity::now_secs, RemoteGame};

const DEV_ROW: f32 = 76.0;

fn ago(t: u64) -> String {
    if t == 0 {
        return "never synced".into();
    }
    let s = now_secs().saturating_sub(t);
    match s {
        0..=59 => "synced just now".into(),
        60..=3599 => format!("synced {} min ago", s / 60),
        3600..=86_399 => format!("synced {} h ago", s / 3600),
        _ => format!("synced {} days ago", s / 86_400),
    }
}

fn size_text(b: u64) -> String {
    match b {
        0..=1023 => format!("{b} B"),
        1024..=1_048_575 => format!("{:.0} KB", b as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        _ => format!("{:.2} GB", b as f64 / 1_073_741_824.0),
    }
}

impl Launcher {
    /// Keep the sync service informed (library, running games), pick up its messages and
    /// copied games, and sync automatically: once after start, and a game's saves after it
    /// closes.
    pub(super) fn update_sync(&mut self) {
        let Some(svc) = self.sync.clone() else { return };
        let running_folders: Vec<String> = self
            .running
            .iter()
            .filter_map(|(p, _)| {
                let title = self.entries.iter().find(|e| e.path == *p).map(|e| e.title.clone()).unwrap_or_default();
                crate::runtime::files::save_root_for(p, &title).file_name().map(|n| n.to_string_lossy().into_owned())
            })
            .collect();
        let (messages, new_games) = {
            let mut s = svc.shared.lock().unwrap_or_else(|e| e.into_inner());
            s.running = running_folders.clone();
            s.library = self.entries.iter().map(|e| (e.title.clone(), e.path.clone())).collect();
            (std::mem::take(&mut s.messages), std::mem::take(&mut s.new_games))
        };
        for m in messages {
            self.say(m);
        }
        for g in new_games {
            self.add_paths(vec![g], "game".into());
        }
        // A game that just closed: sync its saves.
        for f in &self.sync_running {
            if !running_folders.contains(f) {
                svc.sync_all(Some(f.clone()));
            }
        }
        self.sync_running = running_folders;
        // Once, a few seconds after start (time for discovery to find paired devices).
        if !self.sync_started && get_time() > 5.0 {
            self.sync_started = true;
            svc.sync_all(None);
        }
    }

    /// Android: pin a home-screen shortcut that opens a game, with its icon.
    #[cfg(target_os = "android")]
    pub(super) fn home_screen_shortcut(&mut self, i: usize) {
        let Some(e) = self.entries.get(i).cloned() else { return };
        // A sharp icon: the game's own, at 192 px (the library keeps a small one).
        let icon = library::data_dir().join("icons").join(format!("{}-192.png", library::game_key(&e.path)));
        if !icon.is_file() {
            if let Some(img) = crate::ctf_loader::GameData::load(&e.path).ok().and_then(|g| g.icon) {
                let _ = std::fs::create_dir_all(icon.parent().unwrap_or(std::path::Path::new(".")));
                let _ = library::write_png(&icon, 192, &img.resized(192));
            }
        }
        let id = format!("game-{}", library::game_key(&e.path));
        let ok = crate::android::pin_shortcut(&id, &e.title, icon.is_file().then_some(icon.as_path()), &e.path);
        self.say(if ok { format!("Adding {} to the home screen...", e.title) } else { "This phone's home screen doesn't accept shortcuts.".to_string() });
    }

    /// A game file's save folder name (what the sync service calls the game).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(super) fn save_folder_of(&self, game: &std::path::Path) -> Option<String> {
        let title = self.entries.iter().find(|e| e.path == game).map(|e| e.title.clone()).unwrap_or_default();
        crate::runtime::files::save_root_for(game, &title).file_name().map(|n| n.to_string_lossy().into_owned())
    }

    /// Android: the game running inside the launcher's window (None when it ends).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(super) fn set_sync_running(&self, folder: Option<String>) {
        if let Some(svc) = &self.sync {
            svc.shared.lock().unwrap_or_else(|e| e.into_inner()).running = folder.into_iter().collect();
        }
    }

    /// The pairing prompt, over whatever is on screen. Returns true while it's shown.
    pub(super) fn draw_pair_prompt(&mut self, sw: f32, sh: f32) -> bool {
        let Some(svc) = self.sync.clone() else { return false };
        let Some(p) = svc.shared.lock().unwrap_or_else(|e| e.into_inner()).pair.clone() else { return false };
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.6));
        let w = (sw - 48.0).min(520.0);
        let h = 250.0;
        let (x, y) = ((sw - w) / 2.0, (sh - h) / 2.0);
        rrect(x, y, w, h, 14.0, CARD);
        let title = match (&p.result, p.incoming) {
            (Some(Ok(n)), _) => format!("Paired with {n}"),
            (Some(Err(_)), _) => "Pairing didn't finish".to_string(),
            (None, true) => format!("{} wants to pair", p.peer_name),
            (None, false) => format!("Pairing with {}", p.peer_name),
        };
        let t = self.fit(&title, 22, w - 48.0);
        self.bold_text(&t, x + 24.0, y + 42.0, 22, TEXT);
        let block_was = self.block_input;
        self.block_input = false;
        match (&p.result, &p.code) {
            (Some(r), _) => {
                let msg = match r {
                    Ok(_) => "Saves now sync automatically whenever both devices are on this network.".to_string(),
                    Err(e) => e.clone(),
                };
                let m = self.fit(&msg, 15, w - 48.0);
                self.text(&m, x + 24.0, y + 90.0, 15, MUTED);
                if self.button(Rect::new(x + w - 24.0 - 110.0, y + h - 24.0 - 42.0, 110.0, 42.0), "OK", true, true)
                    || is_key_pressed(KeyCode::Enter)
                    || is_key_pressed(KeyCode::Escape)
                    || self.pad_a
                    || self.pad_back
                {
                    svc.dismiss_pair();
                    if matches!(r, Ok(_)) {
                        svc.sync_all(None);
                    }
                }
            }
            (None, None) => self.text("Connecting...", x + 24.0, y + 90.0, 15, MUTED),
            (None, Some(code)) => {
                self.text("Check that the other device shows the same code:", x + 24.0, y + 80.0, 15, MUTED);
                let cw = self.bold_width(code, 44);
                self.bold_text(code, x + (w - cw) / 2.0, y + 142.0, 44, ACCENT);
                if p.decision.is_some() {
                    self.text("Waiting for the other device...", x + 24.0, y + h - 36.0, 15, MUTED);
                } else {
                    let cancel = Rect::new(x + w - 24.0 - 110.0, y + h - 24.0 - 42.0, 110.0, 42.0);
                    let ok = Rect::new(cancel.x - 12.0 - 180.0, cancel.y, 180.0, 42.0);
                    if self.button(ok, "Codes match", true, true) || self.pad_a {
                        svc.answer_pair(true);
                    } else if self.button(cancel, "Cancel", false, true) || is_key_pressed(KeyCode::Escape) || self.pad_back {
                        svc.answer_pair(false);
                    }
                }
            }
        }
        let _ = block_was;
        self.block_input = true;
        true
    }

    pub(super) fn draw_devices_page(&mut self, top: f32) {
        let Some(svc) = self.sync.clone() else {
            self.devices_page = false;
            return;
        };
        if let Some(peer) = self.remote_games_for.clone() {
            self.draw_remote_games(top, &svc, &peer);
            return;
        }
        if self.games_chooser {
            self.draw_games_chooser(top, &svc);
            return;
        }
        let (mut p, close) = self.page_start(top, "Devices", "Pair Rekindle on your other computers, Steam Deck or phone to keep saves in sync on the same network.");
        let (peers, nearby, status, syncing) = {
            let s = svc.shared.lock().unwrap_or_else(|e| e.into_inner());
            let mut nearby: Vec<_> = s.nearby.values().cloned().collect();
            nearby.sort_by(|a, b| a.name.cmp(&b.name));
            (s.peers.clone(), nearby, s.status.clone(), s.syncing.clone())
        };

        // This device.
        rrect(p.x, p.y, p.w, 70.0, 12.0, CARD);
        self.text("This device", p.x + CARD_PAD, p.y + 30.0, 17, TEXT);
        let vis = if svc.port == 0 { "Can't listen on the network (another Rekindle may be open)".to_string() } else { format!("{}, visible to other devices on this network", svc.me.name) };
        let vis = self.fit(&vis, 14, p.w - 2.0 * CARD_PAD);
        self.text(&vis, p.x + CARD_PAD, p.y + 52.0, 14, MUTED);
        p.y += 70.0 + SECTION_GAP;

        // Paired devices.
        self.section_title(&mut p, "PAIRED");
        if peers.is_empty() {
            rrect(p.x, p.y, p.w, 60.0, 12.0, CARD);
            self.text("No paired devices yet. Pair one from the list below.", p.x + CARD_PAD, p.y + 36.0, 15, MUTED);
            p.y += 60.0;
        } else {
            let y0 = p.y;
            self.card_behind(p.x, y0, p.w, y0 + peers.len() as f32 * DEV_ROW);
            for (k, peer) in peers.iter().enumerate() {
                let y = y0 + k as f32 * DEV_ROW;
                if k > 0 {
                    draw_line(p.x + CARD_PAD, y, p.x + p.w - CARD_PAD, y, 1.0, mix(CARD, TEXT, 0.06));
                }
                let online = nearby.iter().any(|n| n.id == peer.id);
                draw_circle(p.x + CARD_PAD + 5.0, y + 26.0, 5.0, if online { Color::new(0.35, 0.8, 0.45, 1.0) } else { MUTED });
                let unpair_w = 96.0;
                let games_w = 90.0;
                let sync_w = 110.0;
                let bx = p.x + p.w - CARD_PAD - unpair_w - games_w - sync_w - 16.0;
                let name = self.fit(&peer.name, 17, bx - p.x - CARD_PAD - 30.0);
                self.text(&name, p.x + CARD_PAD + 18.0, y + 32.0, 17, TEXT);
                let line = if syncing.contains(&peer.id) {
                    "Syncing...".to_string()
                } else {
                    let st = status.get(&peer.id).cloned().unwrap_or_default();
                    let base = format!("{} - {}", if online { "On this network" } else { "Not on this network" }, ago(peer.last_sync));
                    if st.is_empty() { base } else { format!("{base} - {st}") }
                };
                let line = self.fit(&line, 14, bx - p.x - CARD_PAD - 30.0);
                self.text(&line, p.x + CARD_PAD + 18.0, y + 54.0, 14, MUTED);
                let by = y + (DEV_ROW - 38.0) / 2.0;
                if self.button(Rect::new(bx, by, sync_w, 38.0), "Sync now", false, online && !syncing.contains(&peer.id)) {
                    svc.sync_with(&peer.id, None);
                }
                if self.button(Rect::new(bx + sync_w + 8.0, by, games_w, 38.0), "Games", false, online) {
                    svc.request_library(&peer.id);
                    self.remote_games_for = Some(peer.id.clone());
                    self.scroll = 0.0;
                }
                let confirming = self.unpair_confirm.as_deref() == Some(peer.id.as_str());
                if self.button(Rect::new(bx + sync_w + games_w + 16.0, by, unpair_w, 38.0), if confirming { "Sure?" } else { "Unpair" }, confirming, true) {
                    if confirming {
                        svc.unpair(&peer.id);
                        self.unpair_confirm = None;
                    } else {
                        self.unpair_confirm = Some(peer.id.clone());
                    }
                }
            }
            p.y = y0 + peers.len() as f32 * DEV_ROW;
        }
        p.y += SECTION_GAP;

        // Devices on the network that aren't paired.
        self.section_title(&mut p, "ON THIS NETWORK");
        let unpaired: Vec<_> = nearby.iter().filter(|n| !peers.iter().any(|q| q.id == n.id)).collect();
        if unpaired.is_empty() {
            rrect(p.x, p.y, p.w, 60.0, 12.0, CARD);
            let msg = self.fit("Looking for devices... Open Rekindle on the other device, on the same network.", 15, p.w - 2.0 * CARD_PAD);
            self.text(&msg, p.x + CARD_PAD, p.y + 36.0, 15, MUTED);
            p.y += 60.0;
        } else {
            let y0 = p.y;
            self.card_behind(p.x, y0, p.w, y0 + unpaired.len() as f32 * DEV_ROW);
            for (k, n) in unpaired.iter().enumerate() {
                let y = y0 + k as f32 * DEV_ROW;
                if k > 0 {
                    draw_line(p.x + CARD_PAD, y, p.x + p.w - CARD_PAD, y, 1.0, mix(CARD, TEXT, 0.06));
                }
                let name = self.fit(&n.name, 17, p.w - 2.0 * CARD_PAD - 120.0);
                self.text(&name, p.x + CARD_PAD, y + 32.0, 17, TEXT);
                let os = match n.os.as_str() {
                    "windows" => "Windows",
                    "linux" => "Linux / Steam Deck",
                    "android" => "Android",
                    "macos" => "macOS",
                    o => o,
                };
                self.text(&format!("{os} - {}", n.addr.ip()), p.x + CARD_PAD, y + 54.0, 14, MUTED);
                if self.button(Rect::new(p.x + p.w - CARD_PAD - 100.0, y + (DEV_ROW - 38.0) / 2.0, 100.0, 38.0), "Pair", true, true) {
                    svc.pair_with(&n.id);
                }
            }
            p.y = y0 + unpaired.len() as f32 * DEV_ROW;
        }
        p.y += 12.0;
        self.add_by_address(&svc, &mut p);
        p.y += 16.0;
        let note = self.fit("Saves sync when the launcher opens and when you close a game; the newer save wins and the older one is backed up.", 14, p.w);
        self.text(&note, p.x, p.y + 14.0, 14, MUTED);
        p.y += 30.0;
        self.page_scroll(top, p.y);
        if close {
            self.devices_page = false;
            self.unpair_confirm = None;
            self.scroll = 0.0;
        }
    }

    /// "Add by address": for devices on another subnet, which broadcasts don't reach.
    fn add_by_address(&mut self, svc: &crate::sync::Service, p: &mut super::pages::Page) {
        let h = 74.0;
        rrect(p.x, p.y, p.w, h, 12.0, CARD);
        let mine = crate::sync::service::local_address().map(|a| a.to_string()).unwrap_or_else(|| "unknown".into());
        match self.addr_input.clone() {
            None => {
                self.text("Can't see a device?", p.x + CARD_PAD, p.y + 31.0, 17, TEXT);
                let hint = self.fit(&format!("Add it by its address. This device's address is {mine}."), 14, p.w - 2.0 * CARD_PAD - 190.0);
                self.text(&hint, p.x + CARD_PAD, p.y + 53.0, 14, MUTED);
                if self.button(Rect::new(p.x + p.w - CARD_PAD - 170.0, p.y + (h - 38.0) / 2.0, 170.0, 38.0), "Add by address", false, true) {
                    self.addr_input = Some(String::new());
                    #[cfg(target_os = "android")]
                    macroquad::miniquad::window::show_keyboard(true);
                }
            }
            Some(mut text) => {
                while let Some(c) = get_char_pressed() {
                    if (c.is_ascii_hexdigit() || c == '.' || c == ':') && text.len() < 45 {
                        text.push(c);
                    }
                }
                if is_key_pressed(KeyCode::Backspace) {
                    text.pop();
                }
                let field = Rect::new(p.x + CARD_PAD, p.y + (h - 40.0) / 2.0, (p.w - 2.0 * CARD_PAD - 230.0).max(160.0), 40.0);
                rrect(field.x, field.y, field.w, field.h, 8.0, BG);
                let shown = if text.is_empty() { "e.g. 192.168.1.20".to_string() } else { text.clone() };
                self.text(&shown, field.x + 12.0, field.y + 26.0, 16, if text.is_empty() { MUTED } else { TEXT });
                if (get_time() * 2.0) as i64 % 2 == 0 && !text.is_empty() {
                    draw_rectangle(field.x + 13.0 + self.text_width(&text, 16), field.y + 10.0, 1.5, 20.0, TEXT);
                }
                let ip = text.trim().parse::<std::net::IpAddr>().ok();
                let add = Rect::new(field.x + field.w + 10.0, field.y, 100.0, 40.0);
                let cancel = Rect::new(add.x + add.w + 8.0, field.y, 100.0, 40.0);
                let mut done = false;
                if self.button(add, "Add", true, ip.is_some()) || (ip.is_some() && (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter))) {
                    if let Some(ip) = ip {
                        svc.add_address(ip);
                        self.say(format!("Looking for Rekindle at {ip}..."));
                    }
                    done = true;
                } else if self.button(cancel, "Cancel", false, true) || is_key_pressed(KeyCode::Escape) {
                    done = true;
                } else {
                    self.addr_input = Some(text);
                }
                if done {
                    self.addr_input = None;
                    #[cfg(target_os = "android")]
                    macroquad::miniquad::window::show_keyboard(false);
                }
            }
        }
        p.y += h;
    }

    /// A paired device's games, to copy here.
    fn draw_remote_games(&mut self, top: f32, svc: &crate::sync::Service, peer: &str) {
        let name = svc.shared.lock().unwrap_or_else(|e| e.into_inner()).peers.iter().find(|p| p.id == peer).map(|p| p.name.clone()).unwrap_or_default();
        let (mut p, close) = self.page_start(top, &format!("Games on {name}"), "Copy a game to this device. Its saves then sync like any other game's.");
        let (list, transfers) = {
            let s = svc.shared.lock().unwrap_or_else(|e| e.into_inner());
            (s.remote_library.get(peer).cloned(), s.transfers.clone())
        };
        let have: Vec<String> = self.entries.iter().map(|e| e.title.to_lowercase()).collect();
        match list {
            None => {
                rrect(p.x, p.y, p.w, 60.0, 12.0, CARD);
                self.text("Asking for the list...", p.x + CARD_PAD, p.y + 36.0, 15, MUTED);
                p.y += 60.0;
            }
            Some(Err(e)) => {
                rrect(p.x, p.y, p.w, 60.0, 12.0, CARD);
                let e = self.fit(&e, 15, p.w - 2.0 * CARD_PAD);
                self.text(&e, p.x + CARD_PAD, p.y + 36.0, 15, MUTED);
                p.y += 60.0;
            }
            Some(Ok(games)) => {
                let y0 = p.y;
                let n = games.len().max(1);
                self.card_behind(p.x, y0, p.w, y0 + n as f32 * DEV_ROW);
                if games.is_empty() {
                    self.text("That device's library is empty.", p.x + CARD_PAD, y0 + 42.0, 15, MUTED);
                }
                for (k, g) in games.iter().enumerate() {
                    let y = y0 + k as f32 * DEV_ROW;
                    if k > 0 {
                        draw_line(p.x + CARD_PAD, y, p.x + p.w - CARD_PAD, y, 1.0, mix(CARD, TEXT, 0.06));
                    }
                    let t = self.fit(&g.title, 17, p.w - 2.0 * CARD_PAD - 160.0);
                    self.text(&t, p.x + CARD_PAD, y + 32.0, 17, TEXT);
                    let transfer = transfers.iter().rev().find(|t| t.title == g.title);
                    let line = match transfer {
                        Some(t) if t.result.is_none() => format!("Copying... {} of {}", size_text(t.done), size_text(t.total.max(t.done))),
                        Some(t) if matches!(t.result, Some(Err(_))) => format!("Copy failed: {}", t.result.clone().and_then(|r| r.err()).unwrap_or_default()),
                        _ => format!("{} - {} file{}", size_text(g.size), g.files, if g.files == 1 { "" } else { "s" }),
                    };
                    let line = self.fit(&line, 14, p.w - 2.0 * CARD_PAD - 160.0);
                    self.text(&line, p.x + CARD_PAD, y + 54.0, 14, MUTED);
                    let copying = transfer.is_some_and(|t| t.result.is_none());
                    let done = transfer.is_some_and(|t| matches!(t.result, Some(Ok(_)))) || have.contains(&g.title.to_lowercase());
                    let label = if done { "On this device" } else if copying { "Copying..." } else { "Copy here" };
                    let r = Rect::new(p.x + p.w - CARD_PAD - 140.0, y + (DEV_ROW - 38.0) / 2.0, 140.0, 38.0);
                    if self.button(r, label, !done, !done && !copying) {
                        self.copy_remote_game(svc, peer, g);
                    }
                }
                p.y = y0 + n as f32 * DEV_ROW;
            }
        }
        self.page_scroll(top, p.y);
        if close {
            self.remote_games_for = None;
            self.scroll = 0.0;
            if self.games_from_menu && !self.games_chooser {
                self.devices_page = false;
                self.games_from_menu = false;
            }
        }
    }

    /// "Get games": straight to the game list of the one paired device on the network, or a
    /// choice when there are several (or a pointer to pairing when there are none).
    pub(super) fn open_get_games(&mut self) {
        let Some(svc) = self.sync.clone() else { return };
        let online: Vec<String> = {
            let s = svc.shared.lock().unwrap_or_else(|e| e.into_inner());
            s.peers.iter().filter(|p| s.nearby.contains_key(&p.id)).map(|p| p.id.clone()).collect()
        };
        self.devices_page = true;
        self.steam_panel = false;
        self.settings_panel = None;
        self.show_log = false;
        self.scroll = 0.0;
        self.games_from_menu = true;
        if let [only] = online.as_slice() {
            self.games_chooser = false;
            svc.request_library(only);
            self.remote_games_for = Some(only.clone());
        } else {
            self.games_chooser = true;
            self.remote_games_for = None;
        }
    }

    /// Which paired device to copy games from.
    fn draw_games_chooser(&mut self, top: f32, svc: &crate::sync::Service) {
        let (mut p, close) = self.page_start(top, "Get games", "Copy games from Rekindle on another device. Their saves sync too.");
        let (peers, nearby) = {
            let s = svc.shared.lock().unwrap_or_else(|e| e.into_inner());
            (s.peers.clone(), s.nearby.clone())
        };
        let online: Vec<_> = peers.iter().filter(|p| nearby.contains_key(&p.id)).collect();
        if online.is_empty() {
            rrect(p.x, p.y, p.w, 92.0, 12.0, CARD);
            let msg = if peers.is_empty() {
                "No paired devices yet. Pair this device with Rekindle on your PC first."
            } else {
                "None of your paired devices is on the network right now. Open Rekindle on one of them."
            };
            let msg = self.fit(msg, 15, p.w - 2.0 * CARD_PAD - 170.0);
            self.text(&msg, p.x + CARD_PAD, p.y + 52.0, 15, MUTED);
            if self.button(Rect::new(p.x + p.w - CARD_PAD - 150.0, p.y + 26.0, 150.0, 40.0), "Pair a device", true, true) {
                self.games_chooser = false;
                self.games_from_menu = false;
            }
            p.y += 92.0;
        } else {
            let y0 = p.y;
            self.card_behind(p.x, y0, p.w, y0 + online.len() as f32 * DEV_ROW);
            for (k, peer) in online.iter().enumerate() {
                let y = y0 + k as f32 * DEV_ROW;
                if k > 0 {
                    draw_line(p.x + CARD_PAD, y, p.x + p.w - CARD_PAD, y, 1.0, mix(CARD, TEXT, 0.06));
                }
                let name = self.fit(&peer.name, 17, p.w - 2.0 * CARD_PAD - 170.0);
                self.text(&name, p.x + CARD_PAD, y + 32.0, 17, TEXT);
                self.text("On this network", p.x + CARD_PAD, y + 54.0, 14, MUTED);
                if self.button(Rect::new(p.x + p.w - CARD_PAD - 150.0, y + (DEV_ROW - 40.0) / 2.0, 150.0, 40.0), "Show games", true, true) {
                    svc.request_library(&peer.id);
                    self.remote_games_for = Some(peer.id.clone());
                    self.scroll = 0.0;
                }
            }
            p.y = y0 + online.len() as f32 * DEV_ROW;
        }
        self.page_scroll(top, p.y);
        if close {
            self.games_chooser = false;
            self.devices_page = false;
            self.games_from_menu = false;
            self.scroll = 0.0;
        }
    }

    fn copy_remote_game(&mut self, svc: &crate::sync::Service, peer: &str, g: &RemoteGame) {
        svc.fetch_game(peer, g);
        self.say(format!("Copying {}...", g.title));
    }
}
