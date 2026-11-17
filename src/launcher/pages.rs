//! The launcher's other screens (a game's settings, Steam games, the play log, the
//! "restart Steam?" question): a centred column of rounded cards under a page title.

use super::view::{mix, rrect};
use super::*;

pub(super) const ROW_H: f32 = 74.0;
pub(super) const CARD_PAD: f32 = 22.0;
pub(super) const SECTION_GAP: f32 = 30.0;
const MAX_W: f32 = 860.0;

fn draw_back_arrow(cx: f32, cy: f32, c: Color) {
    draw_line(cx - 7.0, cy, cx + 7.0, cy, 2.0, c);
    draw_line(cx - 7.0, cy, cx - 1.0, cy - 6.0, 2.0, c);
    draw_line(cx - 7.0, cy, cx - 1.0, cy + 6.0, 2.0, c);
}

/// Layout of a page: the content column and where the next element goes.
pub(super) struct Page {
    pub x: f32,
    pub w: f32,
    pub y: f32,
}

impl Launcher {
    /// Page title with a back arrow; scrolls with the content. Returns the layout and
    /// whether the page should close (arrow, Escape or the controller's B).
    pub(super) fn page_start(&mut self, top: f32, title: &str, subtitle: &str) -> (Page, bool) {
        let sw = sw();
        let w = (sw - 48.0).min(MAX_W);
        let x = (sw - w) / 2.0;
        let y = top + 24.0 - self.scroll;
        let back = self.icon_button(Rect::new(x, y, 40.0, 40.0), draw_back_arrow);
        let t = self.fit(title, 26, w - 60.0);
        self.bold_text(&t, x + 56.0, y + 29.0, 26, TEXT());
        let mut ny = y + 52.0;
        if !subtitle.is_empty() {
            let s = self.fit(subtitle, 15, w);
            self.text(&s, x, ny + 14.0, 15, MUTED());
            ny += 28.0;
        }
        let close = back || (!self.block_input && self.addr_input.is_none() && is_key_pressed(KeyCode::Escape)) || self.pad_back;
        (Page { x, w, y: ny + 12.0 }, close)
    }

    /// Scroll a page whose content ends at `end` (screen y, already scrolled).
    pub(super) fn page_scroll(&mut self, top: f32, end: f32) {
        let content = end + self.scroll - top + 40.0;
        let max = (content - (sh() - top)).max(0.0);
        if !self.block_input {
            self.scroll -= mouse_wheel().1 * 0.5;
        }
        self.scroll = self.scroll.clamp(0.0, max);
    }

    pub(super) fn section_title(&self, p: &mut Page, title: &str) {
        self.bold_text(title, p.x + 4.0, p.y + 16.0, 15, MUTED());
        p.y += 28.0;
    }

    /// A segmented switch ending at `right`; returns the option clicked.
    fn segmented(&self, right: f32, y: f32, options: &[&str], selected: usize) -> Option<usize> {
        let widths: Vec<f32> = options.iter().map(|o| self.bold_width(o, 15) + 28.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + 8.0;
        let h = 38.0;
        let mut x = right - total;
        rrect(x, y, total, h, 10.0, BG());
        x += 4.0;
        let mouse = mouse();
        let mut clicked = None;
        for (k, (o, w)) in options.iter().zip(&widths).enumerate() {
            let r = Rect::new(x, y + 4.0, *w, h - 8.0);
            let hover = !self.block_input && r.contains(mouse);
            if k == selected {
                rrect(r.x, r.y, r.w, r.h, 7.0, ACCENT());
            } else if hover {
                rrect(r.x, r.y, r.w, r.h, 7.0, mix(BG(), TEXT(), 0.08));
            }
            let fg = if k == selected { Color::new(0.1, 0.06, 0.02, 1.0) } else { mix(TEXT(), MUTED(), 0.25) };
            self.bold_text(o, r.x + (r.w - self.bold_width(o, 15)) / 2.0, r.y + 20.0, 15, fg);
            if self.tap(r) {
                clicked = Some(k);
            }
            x += w;
        }
        clicked
    }

    /// Width a segmented switch needs.
    pub(super) fn segmented_width(&self, options: &[&str]) -> f32 {
        options.iter().map(|o| self.bold_width(o, 15) + 28.0).sum::<f32>() + 8.0
    }

    /// One setting inside a card: label and hint on the left, a switch on the right (or
    /// under the text when the window is narrow). Returns the option clicked.
    pub(super) fn setting(&self, p: &mut Page, first: bool, label: &str, hint: &str, options: &[&str], selected: usize) -> Option<usize> {
        let sw = self.segmented_width(options);
        let inner = p.w - 2.0 * CARD_PAD;
        let stacked = sw + 220.0 > inner;
        let h = if stacked { ROW_H + 46.0 } else { ROW_H };
        if !first {
            draw_line(p.x + CARD_PAD, p.y, p.x + p.w - CARD_PAD, p.y, 1.0, mix(CARD(), TEXT(), 0.06));
        }
        self.text(label, p.x + CARD_PAD, p.y + 31.0, 17, TEXT());
        let text_w = if stacked { inner } else { inner - sw - 16.0 };
        let hint = self.fit(hint, 14, text_w);
        self.text(&hint, p.x + CARD_PAD, p.y + 53.0, 14, MUTED());
        let (right, sy) = if stacked { (p.x + CARD_PAD + sw, p.y + ROW_H - 6.0) } else { (p.x + p.w - CARD_PAD, p.y + (ROW_H - 38.0) / 2.0) };
        let r = self.segmented(right, sy, options, selected);
        p.y += h;
        r
    }

    /// A rounded card from `y0` to the page's current position.
    pub(super) fn card_behind(&self, x: f32, y0: f32, w: f32, y1: f32) {
        rrect(x, y0, w, y1 - y0, 12.0, CARD());
    }

    /// Height of a card holding settings rows with these options.
    fn card_h(&self, p: &Page, rows: &[&[&str]]) -> f32 {
        let inner = p.w - 2.0 * CARD_PAD;
        rows.iter().map(|o| ROW_H + if self.segmented_width(o) + 220.0 > inner { 46.0 } else { 0.0 }).sum()
    }

    /// A card of settings rows under a section title; returns the row clicked and the
    /// option chosen. Rows are (label, hint, options, selected).
    pub(super) fn settings_card(&self, p: &mut Page, title: &str, rows: &[(&str, &str, &[&str], usize)]) -> Option<(usize, usize)> {
        self.section_title(p, title);
        let opts: Vec<&[&str]> = rows.iter().map(|r| r.2).collect();
        let h = self.card_h(p, &opts);
        self.card_behind(p.x, p.y, p.w, p.y + h);
        let mut hit = None;
        for (k, (label, hint, options, selected)) in rows.iter().enumerate() {
            if let Some(o) = self.setting(p, k == 0, label, hint, options, *selected) {
                hit = Some((k, o));
            }
        }
        p.y += SECTION_GAP;
        hit
    }

    /// The display, sound and controller settings a game runs with (its own page, and the
    /// defaults on the Settings page). Returns whether something changed.
    fn game_setting_cards(&self, p: &mut Page, s: &mut Settings) -> bool {
        let mut changed = false;
        let filters: [&str; 5] = ["Sharp", "Smooth", "Enhanced", "HD", "Pixel art"];
        let fs = match s.fullscreen {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        let sc = Scaling::ALL.iter().position(|v| *v == s.scaling).unwrap_or(0);
        let fi = Filter::ALL.iter().position(|v| *v == s.filter).unwrap_or(1);
        let display: [(&str, &str, &[&str], usize); 5] = [
            ("Fullscreen", "Start in fullscreen or in a window (F11 in game)", &["Game default", "On", "Off"], fs),
            ("Window size", "Size of the window when not fullscreen", &["Auto", "1x", "2x", "3x"], s.window_scale.min(3) as usize),
            ("Scaling", "Fit keeps the shape, Integer keeps pixels crisp (F10)", &["Fit", "Integer", "Stretch"], sc),
            ("Upscaling", "HD for drawn art, Pixel art for pixel games (F9)", &filters, fi),
            ("Overlay at start", "Frame rate, timings and game state (F1 in game)", &["Off", "On"], s.hud as usize),
        ];
        if let Some((row, k)) = self.settings_card(p, "DISPLAY", &display) {
            match row {
                0 => s.fullscreen = [None, Some(true), Some(false)][k],
                1 => s.window_scale = k as u8,
                2 => s.scaling = Scaling::ALL[k],
                3 => s.filter = Filter::ALL[k],
                _ => s.hud = k == 1,
            }
            changed = true;
        }

        let vol = ((s.volume as usize + 12) / 25).min(4);
        let sound: [(&str, &str, &[&str], usize); 1] =
            [("Volume", "How loud the game plays (Ctrl+M mutes in game)", &["Off", "25%", "50%", "75%", "100%"], vol)];
        if let Some((_, k)) = self.settings_card(p, "SOUND", &sound) {
            s.volume = (k * 25) as u8;
            changed = true;
        }

        let move_k = match s.pad_wasd {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        };
        let controller: [(&str, &str, &[&str], usize); 4] = [
            ("Controller", "Play with a gamepad: buttons press keys, a stick moves the mouse", &["Off", "On"], s.pad as usize),
            ("Stick moves with", "Which keys the left stick and D-pad press (Automatic: the game's own)", &["Automatic", "Arrow keys", "W A S D"], move_k),
            ("Cursor speed", "Right stick mouse cursor (RT click, LT right-click)", &["Slow", "Normal", "Fast"], (s.pad_speed.clamp(1, 3) - 1) as usize),
            ("Button guide", "Show what the buttons do when the controls change", &["Off", "On"], s.pad_legend as usize),
        ];
        if let Some((row, k)) = self.settings_card(p, "CONTROLLER", &controller) {
            match row {
                0 => s.pad = k == 1,
                1 => s.pad_wasd = [None, Some(false), Some(true)][k.min(2)],
                2 => s.pad_speed = k as u8 + 1,
                _ => s.pad_legend = k == 1,
            }
            changed = true;
        }
        changed
    }

    pub(super) fn draw_settings_panel(&mut self, top: f32, _bottom: f32) {
        let Some((i, mut s)) = self.settings_panel.clone() else { return };
        let title = self.entries.get(i).map(|e| e.title.clone()).unwrap_or_default();
        let (mut p, close) = self.page_start(top, &title, "Settings are saved right away and apply the next time the game starts.");
        let mut changed = self.game_setting_cards(&mut p, &mut s);

        let mods: [(&str, &str, &[&str], usize); 2] = [
            ("Mods and AI upscale packs", "Replacement images and sounds for this game, if it has any", &["Off", "On"], s.mods as usize),
            ("AI upscale pack", "Its sharper images, or the game's originals (Ctrl+U in game)", &["Off", "On"], s.ai_pack as usize),
        ];
        if let Some((row, k)) = self.settings_card(&mut p, "MODS", &mods) {
            if row == 0 {
                s.mods = k == 1;
            } else {
                s.ai_pack = k == 1;
            }
            changed = true;
        }
        if !cfg!(target_os = "android") && self.launch_card(&mut p, &mut s) {
            changed = true;
        }
        if changed {
            s.save();
        }

        self.section_title(&mut p, "SAVES");
        self.saves_card(i, &mut p);
        p.y += 12.0;
        self.snapshots_card(i, &mut p);
        self.page_scroll(top, p.y);
        if close {
            self.settings_panel = None;
            self.scroll = 0.0;
        } else {
            self.settings_panel = Some((i, s));
        }
    }

    /// KlikCore's own settings, and the defaults games start from.
    pub(super) fn draw_app_settings(&mut self, top: f32) {
        let Some(mut defaults) = self.app_settings.clone() else { return };
        let (mut p, close) = self.page_start(top, "Settings", "Saved right away.");
        let android = cfg!(target_os = "android");

        let mut rows: Vec<(&str, &str, &[&str], usize)> = vec![
            ("Check for updates", "Look for a newer KlikCore when the launcher opens", &["Off", "On"], self.config.update_check as usize),
            ("Interface size", "Size of the launcher's text and buttons", &["100%", "125%", "150%", "175%"], UI_SIZES.iter().position(|v| *v == self.config.ui_size).unwrap_or(0)),
        ];
        let discord_row = crate::discord::available().then(|| rows.len());
        if discord_row.is_some() {
            rows.push(("Discord status", "Show the game you're playing in Discord", &["Off", "On"], self.config.discord as usize));
        }
        let accents: Vec<&str> = ACCENTS.iter().map(|a| a.0).collect();
        let theme_row = rows.len();
        rows.push(("Theme", "Dark or light launcher", &["Dark", "Light"], self.config.light as usize));
        let accent_row = rows.len();
        rows.push(("Accent colour", "Buttons, highlights and badges", &accents, self.config.accent as usize));
        let tv_row = rows.len();
        rows.push(("TV mode", "Big interface, fullscreen (Auto: in Steam's Gaming Mode)", &["Off", "On", "Auto"], self.config.tv as usize));
        let steam_row = (!android).then(|| rows.len());
        if steam_row.is_some() {
            rows.push(("Offer to add KlikCore to Steam", "Ask when Steam has no shortcut for KlikCore", &["Never", "Ask"], (!self.config.steam_self_never) as usize));
        }
        if let Some((row, k)) = self.settings_card(&mut p, "KLIKCORE", &rows) {
            let on = k == 1;
            match row {
                0 => self.config.update_check = on,
                1 => self.config.ui_size = UI_SIZES[k],
                r if r == theme_row => self.config.light = k == 1,
                r if r == accent_row => self.config.accent = k as u8,
                r if r == tv_row => {
                    self.config.tv = k as u8;
                    set_fullscreen(self.config.tv == 1 || (self.config.tv == 2 && super::steam_shortcuts::in_gaming_mode()));
                }
                r if Some(r) == discord_row => self.config.discord = on,
                r if Some(r) == steam_row => self.config.steam_self_never = !on,
                _ => {}
            }
            self.config.save();
        }

        if !android {
            self.watched_card(&mut p);
        }
        self.bold_text("GAME DEFAULTS", p.x + 4.0, p.y + 16.0, 15, MUTED());
        let note = self.fit("For games whose settings you haven't changed. A game's own settings page overrides these.", 14, p.w - 8.0);
        self.text(&note, p.x + 4.0, p.y + 40.0, 14, MUTED());
        p.y += 56.0;
        if self.game_setting_cards(&mut p, &mut defaults) {
            defaults.save();
        }
        self.page_scroll(top, p.y);
        if close {
            self.app_settings = None;
            self.scroll = 0.0;
        } else {
            self.app_settings = Some(defaults);
        }
    }

    /// Save manager card: move saves between KlikCore and the original game.
    fn saves_card(&mut self, i: usize, p: &mut Page) {
        use crate::runtime::save_sync::{self, Direction};
        let Some(entry) = self.entries.get(i).cloned() else { return };
        let fs = crate::runtime::files::SaveFs::new(&entry.path, crate::runtime::files::save_root_for(&entry.path, &entry.title))
            .with_prefix(super::steam_shortcuts::proton_prefix(&entry.path));
        if self.saves_status.as_ref().is_none_or(|(k, _)| *k != i) {
            self.saves_status = Some((i, save_sync::status(&fs).line()));
            self.saves_confirm = None;
        }
        let status = self.saves_status.as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
        let confirm = match self.saves_confirm {
            Some(Direction::Import) => Some("Click again to replace KlikCore's saves with the original game's. A backup is kept."),
            Some(Direction::Export) => Some("Click again to replace the original game's saves with KlikCore's. A backup is kept."),
            None => None,
        };
        let labels = ["Use original game's saves", "Copy to original game", "Open folder"];
        let widths: Vec<f32> = labels.iter().map(|l| self.bold_width(l, 16) + 32.0).collect();
        let inner = p.w - 2.0 * CARD_PAD;
        let one_line = widths.iter().sum::<f32>() + 20.0 <= inner;
        let buttons_h = if one_line { 40.0 } else { 3.0 * 48.0 - 8.0 };
        let h = CARD_PAD + 22.0 + 28.0 + buttons_h + CARD_PAD;
        rrect(p.x, p.y, p.w, h, 12.0, CARD());
        self.text("Move your progress between KlikCore and the original game.", p.x + CARD_PAD, p.y + CARD_PAD + 14.0, 15, TEXT());
        let line = self.fit(confirm.unwrap_or(&status), 14, inner);
        self.text(&line, p.x + CARD_PAD, p.y + CARD_PAD + 38.0, 14, if confirm.is_some() { ACCENT() } else { MUTED() });
        let mut x = p.x + CARD_PAD;
        let mut y = p.y + CARD_PAD + 50.0;
        let mut clicked = None;
        let selected = match self.saves_confirm {
            Some(Direction::Import) => 0,
            Some(Direction::Export) => 1,
            None => usize::MAX,
        };
        for (k, (l, w)) in labels.iter().zip(&widths).enumerate() {
            let r = if one_line { Rect::new(x, y, *w, 40.0) } else { Rect::new(p.x + CARD_PAD, y, inner, 40.0) };
            if self.button(r, l, k == selected, true) {
                clicked = Some(k);
            }
            if one_line {
                x += w + 10.0;
            } else {
                y += 48.0;
            }
        }
        p.y += h;
        let dir = match clicked {
            Some(0) => Direction::Import,
            Some(1) => Direction::Export,
            Some(_) => {
                let _ = std::fs::create_dir_all(&fs.save_root);
                let opener = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                let _ = std::process::Command::new(opener).arg(&fs.save_root).spawn();
                return;
            }
            None => return,
        };
        if self.saves_confirm != Some(dir) {
            self.saves_confirm = Some(dir);
            return;
        }
        self.saves_confirm = None;
        if self.is_running(&entry.path) {
            self.say(format!("Close {} first, then copy its saves.", entry.title));
            return;
        }
        let r = save_sync::copy(&fs, dir);
        self.saves_status = None;
        let what = if dir == Direction::Import { "KlikCore now uses the original game's saves" } else { "The original game now has KlikCore's saves" };
        let backup = r.backup.map(|b| format!("; backup in {}", b.display())).unwrap_or_default();
        if r.errors.is_empty() {
            self.say(format!("{what}: {} copied, {} removed{backup}.", r.copied, r.removed));
        } else {
            self.say(format!("Copied {}, but {} failed: {}{backup}", r.copied, r.errors.len(), r.errors[0]));
        }
    }

    pub(super) fn draw_steam_panel(&mut self, top: f32) {
        let (mut p, close) = self.page_start(top, "Steam games", "Finds Clickteam Fusion games installed through Steam and adds them to your library.");
        let idle = self.busy.is_none();
        let copy_opts = ["Link in place", "Copy to storage"];
        let stacked = self.segmented_width(&copy_opts) + 220.0 > p.w - 2.0 * CARD_PAD;
        let card_h = 2.0 * ROW_H + if stacked { 46.0 } else { 0.0 };
        self.card_behind(p.x, p.y, p.w, p.y + card_h);
        if let Some(k) = self.setting(&mut p, true, "Add games by", "Linking plays Steam's copy; copying keeps a copy of your own", &copy_opts, self.config.steam_copy as usize) {
            self.config.steam_copy = k == 1;
            self.config.save();
        }
        // Storage folder row.
        draw_line(p.x + CARD_PAD, p.y, p.x + p.w - CARD_PAD, p.y, 1.0, mix(CARD(), TEXT(), 0.06));
        self.text("Storage folder", p.x + CARD_PAD, p.y + 31.0, 17, if self.config.steam_copy { TEXT() } else { MUTED() });
        let choose = Rect::new(p.x + p.w - CARD_PAD - 120.0, p.y + (ROW_H - 38.0) / 2.0, 120.0, 38.0);
        let shown = self.fit_left(&self.config.storage.display().to_string(), 14, choose.x - p.x - CARD_PAD - 16.0);
        self.text(&shown, p.x + CARD_PAD, p.y + 53.0, 14, MUTED());
        if self.button(choose, "Choose...", false, idle) {
            if let Some(dir) = dialogs::pick_folder() {
                self.config.storage = dir;
                self.config.save();
            }
        }
        p.y += ROW_H + 20.0;
        if self.button(Rect::new(p.x, p.y, 300.0, 44.0), "Find Clickteam games in Steam", true, idle) {
            self.scan_steam();
        }
        p.y += 64.0;
        if !self.steam_lines.is_empty() {
            let h = self.steam_lines.len() as f32 * 26.0 + 2.0 * 14.0;
            rrect(p.x, p.y, p.w, h, 12.0, CARD());
            for (k, line) in self.steam_lines.iter().enumerate() {
                let l = self.fit(line, 15, p.w - 2.0 * CARD_PAD);
                self.text(&l, p.x + CARD_PAD, p.y + 14.0 + 19.0 + k as f32 * 26.0, 15, TEXT());
            }
            p.y += h;
        }
        self.page_scroll(top, p.y);
        if close {
            self.steam_panel = false;
            self.scroll = 0.0;
        }
    }

    /// Steam is open: confirm restarting it to add a shortcut.
    pub(super) fn draw_steam_confirm(&mut self, top: f32, i: usize) {
        let (sw, sh) = (sw(), sh());
        let title = self.entries.get(i).map(|e| e.title.clone()).unwrap_or_default();
        let w = (sw - 48.0).min(560.0);
        let lines = [
            "Steam is running. It only reads its shortcuts when it starts,",
            "and can overwrite changes made while it's open. KlikCore will",
            "close Steam, add the shortcut (with the game's artwork) and",
            "start Steam again.",
        ];
        let h = 64.0 + lines.len() as f32 * 24.0 + 80.0;
        let (x, y) = ((sw - w) / 2.0, top + ((sh - top) - h) / 2.0 - 20.0);
        rrect(x, y, w, h, 14.0, CARD());
        let t = self.fit(&format!("Add {title} to Steam?"), 22, w - 48.0);
        self.bold_text(&t, x + 24.0, y + 40.0, 22, TEXT());
        for (k, l) in lines.iter().enumerate() {
            self.text(l, x + 24.0, y + 74.0 + k as f32 * 24.0, 16, MUTED());
        }
        let by = y + h - 24.0 - 42.0;
        let cancel = Rect::new(x + w - 24.0 - 110.0, by, 110.0, 42.0);
        let go = Rect::new(cancel.x - 12.0 - 230.0, by, 230.0, 42.0);
        if self.button(go, "Restart Steam and add", true, true) || self.pad_a || is_key_pressed(KeyCode::Enter) {
            self.steam_confirm = None;
            self.add_to_steam(i);
        } else if self.button(cancel, "Cancel", false, true) || is_key_pressed(KeyCode::Escape) || self.pad_back {
            self.steam_confirm = None;
        }
    }

    /// The play log: newest sessions first.
    pub(super) fn draw_log(&mut self, top: f32, _bottom: f32) {
        let total: u64 = self.sessions.iter().map(|s| s.seconds).sum();
        let subtitle = format!(
            "{} session{}, {} played in total",
            self.sessions.len(),
            if self.sessions.len() == 1 { "" } else { "s" },
            playlog::duration(total)
        );
        let (mut p, close) = self.page_start(top, "Play log", &subtitle);
        self.stats_card(&mut p);
        if self.sessions.is_empty() {
            rrect(p.x, p.y, p.w, 80.0, 12.0, CARD());
            self.text("Nothing yet: play a game and its session shows up here.", p.x + CARD_PAD, p.y + 46.0, 16, MUTED());
            p.y += 80.0;
        } else {
            let row = 44.0;
            let h = self.sessions.len() as f32 * row + 12.0;
            // Only the visible part of a long log is drawn.
            let (y0, sh) = (p.y, sh());
            rrect(p.x, y0, p.w, h, 12.0, CARD());
            for (k, s) in self.sessions.iter().rev().enumerate() {
                let y = y0 + 6.0 + k as f32 * row;
                if y + row < top || y > sh {
                    continue;
                }
                if k > 0 {
                    draw_line(p.x + CARD_PAD, y, p.x + p.w - CARD_PAD, y, 1.0, mix(CARD(), TEXT(), 0.05));
                }
                let d = playlog::duration(s.seconds);
                let dw = self.text_width(&d, 16);
                self.text(&s.started, p.x + CARD_PAD, y + 28.0, 14, MUTED());
                let t = self.fit(&s.title, 16, p.w - 2.0 * CARD_PAD - 170.0 - dw - 16.0);
                self.text(&t, p.x + CARD_PAD + 170.0, y + 28.0, 16, TEXT());
                self.text(&d, p.x + p.w - CARD_PAD - dw, y + 28.0, 16, TEXT());
            }
            p.y += h;
        }
        self.page_scroll(top, p.y);
        if close {
            self.show_log = false;
            self.scroll = 0.0;
        }
    }

    /// Toast asking to add KlikCore itself to Steam.
    pub(super) fn draw_steam_offer(&mut self, sh: f32) {
        let msg = "Add KlikCore to Steam? Steam will restart.";
        let labels = [("Add", true), ("Not now", false), ("Don't ask again", false)];
        let widths: Vec<f32> = labels.iter().map(|(l, _)| self.bold_width(l, 16) + 28.0).collect();
        let w = self.text_width(msg, 15) + 32.0 + widths.iter().sum::<f32>() + 8.0 * 3.0;
        let r = Rect::new(20.0, sh - 20.0 - 52.0, w, 52.0);
        rrect(r.x, r.y, r.w, r.h, 14.0, MENU());
        self.text(msg, r.x + 16.0, r.y + 31.0, 15, TEXT());
        let mut x = r.x + 16.0 + self.text_width(msg, 15) + 16.0;
        let mut clicked = None;
        for (k, ((label, accent), bw)) in labels.iter().zip(&widths).enumerate() {
            if self.button(Rect::new(x, r.y + 8.0, *bw, 36.0), label, *accent, true) {
                clicked = Some(k);
            }
            x += bw + 8.0;
        }
        match clicked {
            Some(0) => {
                self.steam_offer = false;
                self.busy = Some("Adding KlikCore to Steam...".into());
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let progress = |s: &str| {
                        let _ = tx.send(Msg::Status(s.to_string()));
                    };
                    let msg = steam_shortcuts::add_launcher(true, progress).unwrap_or_else(|e| format!("Could not add KlikCore to Steam: {e}"));
                    let _ = tx.send(Msg::Done(msg));
                });
            }
            Some(1) => self.steam_offer = false,
            Some(_) => {
                self.steam_offer = false;
                self.config.steam_self_never = true;
                self.config.save();
            }
            None => {}
        }
    }
}
