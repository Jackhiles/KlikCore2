//! The library screen: header with search, a grid of game covers, the selected game's
//! details, pop-up menus and toasts.

use super::*;

/// What a pop-up menu entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuAction {
    AddGame,
    AddFolder,
    SteamGames,
    PlayLog,
    DataFolder,
    Devices,
    GetGames,
    ToSteam(usize),
    Shortcut(usize),
    Mods(usize),
    GameFolder(usize),
    RefreshArt(usize),
    Upscale(usize, super::upscale::Style),
    AppSettings,
    Favourite(usize),
    Collections(usize),
    CheckCompat(usize),
    Screenshots(usize),
    Report(usize),
    Tests,
    /// Your own cover ("cover") or banner ("hero") picture for a game, or remove them.
    CustomArt(usize, &'static str),
    RemoveCustomArt(usize),
    RemoveUpscale(usize),
    RepairUpscale(usize),
    Remove(usize),
}

#[derive(Debug, Clone)]
pub(super) struct Menu {
    pub x: f32,
    pub y: f32,
    pub items: Vec<(&'static str, MenuAction)>,
    /// Opened this frame (the click that opened it must not close it).
    pub fresh: bool,
}

/// Game actions shared by buttons, menus and the controller.
pub(super) const ACT_PLAY: u8 = 0;
pub(super) const ACT_SETTINGS: u8 = 4;

const GAP: f32 = 18.0;
/// Narrower on phones, so more covers fit across.
const MIN_CARD_W: f32 = if cfg!(target_os = "android") { 128.0 } else { 150.0 };
const MAX_CARD_W: f32 = 220.0;
const TITLE_SPACE: f32 = 46.0;
/// The sort / filter chips above the cover grid.
const TOOLBAR_H: f32 = 46.0;
const WARN: Color = Color::new(0.96, 0.73, 0.30, 1.0);

/// A five-pointed star (favourites).
fn draw_star(cx: f32, cy: f32, r: f32, c: Color) {
    let pt = |k: usize, rad: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
        vec2(cx + rad * a.cos(), cy + rad * a.sin())
    };
    let centre = vec2(cx, cy);
    for k in 0..10 {
        let (r1, r2) = if k % 2 == 0 { (r, r * 0.42) } else { (r * 0.42, r) };
        draw_triangle(centre, pt(k, r1), pt(k + 1, r2), c);
    }
}

/// A filled rectangle with rounded corners (opaque colours only: the parts overlap).
pub(super) fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
    let r = r.min(w / 2.0).min(h / 2.0);
    draw_rectangle(x + r, y, w - 2.0 * r, h, c);
    draw_rectangle(x, y + r, r, h - 2.0 * r, c);
    draw_rectangle(x + w - r, y + r, r, h - 2.0 * r, c);
    for (cx, cy) in [(x + r, y + r), (x + w - r, y + r), (x + r, y + h - r), (x + w - r, y + h - r)] {
        draw_circle(cx, cy, r, c);
    }
}

/// `a` blended towards `b` by t (opaque result).
pub(super) fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::new(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1.0)
}

fn draw_dots(cx: f32, cy: f32, c: Color) {
    for dx in [-7.0, 0.0, 7.0] {
        draw_circle(cx + dx, cy, 2.2, c);
    }
}

fn draw_plus(cx: f32, cy: f32, s: f32, c: Color) {
    draw_rectangle(cx - s, cy - 1.0, 2.0 * s, 2.0, c);
    draw_rectangle(cx - 1.0, cy - s, 2.0, 2.0 * s, c);
}

impl Launcher {
    /// Indices of the games matching the search, in library order.
    pub(super) fn visible(&self) -> Vec<usize> {
        let q = self.search.trim().to_lowercase();
        let f = self.config.filter.as_str();
        let mut v: Vec<usize> = (0..self.entries.len())
            .filter(|&i| q.is_empty() || self.entries[i].title.to_lowercase().contains(&q))
            .filter(|&i| {
                let p = &self.entries[i].path;
                match f {
                    "" => true,
                    "fav" => self.meta.is_favourite(p),
                    "steam" => p.to_string_lossy().to_lowercase().contains("steamapps"),
                    c => c.strip_prefix("c:").is_some_and(|c| self.meta.in_collection(p, c)),
                }
            })
            .collect();
        // Versions of one game: only the chosen one (or the first in the list) gets a card.
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for &i in &v {
            groups.entry(super::meta::version_group(&self.entries[i].title)).or_default().push(i);
        }
        v.retain(|&i| {
            let g = super::meta::version_group(&self.entries[i].title);
            let members = &groups[&g];
            if members.len() < 2 {
                return true;
            }
            let pick = self.meta.preferred(&g).and_then(|p| members.iter().copied().find(|&k| self.entries[k].path.to_string_lossy().to_lowercase() == p));
            pick.unwrap_or(members[0]) == i
        });
        // Favourites first, then the chosen order.
        let played = |i: usize| self.totals.get(&self.entries[i].path.to_string_lossy().to_lowercase()).cloned();
        v.sort_by(|&a, &b| {
            let fav = self.meta.is_favourite(&self.entries[b].path).cmp(&self.meta.is_favourite(&self.entries[a].path));
            let name = || self.entries[a].title.to_lowercase().cmp(&self.entries[b].title.to_lowercase());
            fav.then_with(|| match self.config.sort {
                0 => played(b).map(|t| t.1).cmp(&played(a).map(|t| t.1)).then_with(name),
                2 => played(b).map_or(0, |t| t.0).cmp(&played(a).map_or(0, |t| t.0)).then_with(name),
                _ => name(),
            })
        });
        v
    }

    /// Every library entry that is a version of game `i` (including it), in library order.
    pub(super) fn versions_of(&self, i: usize) -> Vec<usize> {
        let g = super::meta::version_group(&self.entries[i].title);
        (0..self.entries.len()).filter(|&k| super::meta::version_group(&self.entries[k].title) == g).collect()
    }

    /// The sort and filter chips; returns true if one was clicked.
    fn draw_toolbar(&mut self, x: f32, y: f32, right: f32) {
        let sort = ["Recently played", "Name", "Most played"][self.config.sort.min(2) as usize];
        let mut chips: Vec<(String, String)> = vec![(format!("Sort: {sort}"), "#sort".into()), ("All".into(), String::new()), ("Favourites".into(), "fav".into())];
        if self.entries.iter().any(|e| e.path.to_string_lossy().to_lowercase().contains("steamapps")) {
            chips.push(("Steam".into(), "steam".into()));
        }
        for c in self.meta.collections() {
            chips.push((c.clone(), format!("c:{c}")));
        }
        draw_rectangle(0.0, y - 10.0, right + 24.0, TOOLBAR_H, BG);
        let mut cx = x;
        let mut clicked = None;
        for (label, key) in &chips {
            let w = self.bold_width(label, 14) + 26.0;
            if cx + w > right {
                break;
            }
            let r = Rect::new(cx, y, w, 30.0);
            let on = key != "#sort" && *key == self.config.filter;
            let hover = !self.block_input && r.contains(mouse());
            let bg = if on { ACCENT } else if hover { mix(BUTTON, TEXT, 0.08) } else { BUTTON };
            rrect(r.x, r.y, r.w, r.h, 15.0, bg);
            let fg = if on { Color::new(0.08, 0.05, 0.02, 1.0) } else { TEXT };
            self.bold_text(label, r.x + 13.0, r.y + 20.0, 14, fg);
            if self.tap(r) {
                clicked = Some(key.clone());
            }
            cx += w + 8.0;
        }
        if let Some(k) = clicked {
            if k == "#sort" {
                self.config.sort = (self.config.sort + 1) % 3;
            } else {
                self.config.filter = k;
            }
            self.config.save();
            self.scroll = 0.0;
        }
    }

    /// Move the selection by `delta` places among the visible games.
    pub(super) fn move_selection(&mut self, delta: isize) {
        let vis = self.visible();
        if vis.is_empty() {
            return;
        }
        let cur = self.selected.and_then(|s| vis.iter().position(|&i| i == s));
        let next = match cur {
            None => 0,
            Some(k) => (k as isize + delta).clamp(0, vis.len() as isize - 1) as usize,
        };
        self.selected = Some(vis[next]);
        self.reveal = true;
    }

    /// A small square button with a drawn icon; returns true when clicked.
    pub(super) fn icon_button(&self, r: Rect, draw_icon: impl Fn(f32, f32, Color)) -> bool {
        let hover = !self.block_input && r.contains(mouse());
        rrect(r.x, r.y, r.w, r.h, 8.0, if hover { mix(BUTTON, TEXT, 0.08) } else { BUTTON });
        draw_icon(r.x + r.w / 2.0, r.y + r.h / 2.0, TEXT);
        self.tap(r)
    }

    pub(super) fn draw_header(&mut self, sw: f32) {
        draw_rectangle(0.0, 0.0, sw, HEADER_H, BG);
        if let Some(t) = self.logo() {
            draw_texture_ex(&t, 24.0, 16.0, WHITE, DrawTextureParams { dest_size: Some(vec2(32.0, 32.0)), ..Default::default() });
        }
        self.bold_text("KlikCore 2", 66.0, 41.0, 24, TEXT);
        let idle = self.busy.is_none();

        let menu_btn = Rect::new(sw - 24.0 - 40.0, 14.0, 40.0, 36.0);
        let add_btn = Rect::new(menu_btn.x - 10.0 - 92.0, 14.0, 92.0, 36.0);
        if self.icon_button(menu_btn, draw_dots) {
            let mut items = if cfg!(target_os = "android") {
                vec![("Play log and stats", MenuAction::PlayLog)]
            } else {
                vec![("Steam games", MenuAction::SteamGames), ("Play log and stats", MenuAction::PlayLog), ("Tests", MenuAction::Tests), ("Open data folder", MenuAction::DataFolder)]
            };
            if self.sync.is_some() {
                items.insert(0, ("Devices and sync", MenuAction::Devices));
            }
            items.push(("Settings", MenuAction::AppSettings));
            self.open_menu(menu_btn.x + menu_btn.w, menu_btn.y + menu_btn.h + 6.0, items, true);
        }
        if self.button(add_btn, "     Add", true, idle) {
            let mut items = if cfg!(target_os = "android") {
                vec![("Get games from a paired device", MenuAction::GetGames)]
            } else {
                vec![("Add a game...", MenuAction::AddGame), ("Add a folder of games...", MenuAction::AddFolder)]
            };
            if !cfg!(target_os = "android") && self.sync.is_some() {
                items.push(("Get games from a paired device", MenuAction::GetGames));
            }
            if self.has_steam {
                items.push(("Find games in Steam", MenuAction::SteamGames));
            }
            self.open_menu(add_btn.x + add_btn.w, add_btn.y + add_btn.h + 6.0, items, true);
        }
        draw_plus(add_btn.x + 24.0, add_btn.y + add_btn.h / 2.0, 6.0, Color::new(0.1, 0.06, 0.02, 1.0));

        // A newer release: a pill left of the buttons.
        let mut right = add_btn.x - 12.0;
        if let Some(r) = self.update.clone() {
            let label = if self.updating { format!("Updating to v{}...", r.version) } else { format!("Update to v{}", r.version) };
            let w = self.bold_width(&label, 16) + 28.0;
            let rect = Rect::new(right - w, 14.0, w, 36.0);
            if self.button(rect, &label, false, !self.updating) {
                self.updating = true;
                self.busy = Some(format!("Downloading KlikCore v{}...", r.version));
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    #[cfg(target_os = "android")]
                    let result = {
                        let status = tx.clone();
                        update::install_android(&r, move |s| {
                            let _ = status.send(Msg::Status(s));
                        })
                        .map(|_| std::path::PathBuf::new())
                    };
                    #[cfg(not(target_os = "android"))]
                    let result = update::install(&r);
                    let _ = tx.send(Msg::UpdateInstalled(result));
                });
            }
            right = rect.x - 12.0;
        }

        // Search: typing anywhere in the library filters it.
        let x0 = 66.0 + self.bold_width("KlikCore 2", 24) + 36.0;
        let w = (right - 12.0 - x0).min(380.0);
        if w > 150.0 && self.library_view() {
            let r = Rect::new(x0, 14.0, w, 36.0);
            // Phones: tapping the search box brings up the on-screen keyboard.
            #[cfg(target_os = "android")]
            if self.tap(r) {
                macroquad::miniquad::window::show_keyboard(true);
            }
            rrect(r.x, r.y, r.w, r.h, 18.0, CARD);
            draw_circle_lines(r.x + 20.0, r.y + 17.0, 6.0, 1.6, MUTED);
            draw_line(r.x + 24.5, r.y + 21.5, r.x + 29.0, r.y + 26.0, 1.8, MUTED);
            if self.search.is_empty() {
                self.text("Type to search", r.x + 38.0, r.y + 24.0, 16, MUTED);
            } else {
                let shown = self.fit(&self.search, 16, r.w - 60.0);
                self.text(&shown, r.x + 38.0, r.y + 24.0, 16, TEXT);
                if (get_time() * 2.0) as i64 % 2 == 0 {
                    let cx = r.x + 38.0 + self.text_width(&shown, 16) + 2.0;
                    draw_rectangle(cx, r.y + 10.0, 1.5, 17.0, TEXT);
                }
            }
        }
        draw_line(0.0, HEADER_H - 0.5, sw, HEADER_H - 0.5, 1.0, mix(BG, TEXT, 0.06));
    }

    /// The library screen is showing (no panel open).
    pub(super) fn library_view(&self) -> bool {
        self.steam_confirm.is_none() && !self.steam_panel && self.settings_panel.is_none() && !self.show_log && !self.devices_page && self.app_settings.is_none() && self.extra.is_none()
    }

    fn open_menu(&mut self, right: f32, y: f32, items: Vec<(&'static str, MenuAction)>, align_right: bool) {
        let w = self.menu_width(&items);
        let x = if align_right { right - w } else { right };
        self.menu = Some(Menu { x, y, items, fresh: true });
    }

    fn menu_width(&self, items: &[(&str, MenuAction)]) -> f32 {
        items.iter().map(|(l, _)| self.text_width(l, 16)).fold(0.0, f32::max).max(160.0) + 40.0
    }

    /// Keyboard in the library: type to search, arrows / Enter to pick and play.
    fn library_keys(&mut self) -> Option<(usize, u8)> {
        if self.block_input {
            while get_char_pressed().is_some() {}
            return None;
        }
        while let Some(c) = get_char_pressed() {
            if !c.is_control() {
                self.search.push(c);
                self.scroll = 0.0;
            }
        }
        if is_key_pressed(KeyCode::Backspace) {
            self.search.pop();
        }
        if is_key_pressed(KeyCode::Escape) {
            self.search.clear();
            #[cfg(target_os = "android")]
            macroquad::miniquad::window::show_keyboard(false);
        }
        let cols = self.grid_cols.max(1) as isize;
        for (k, d) in [(KeyCode::Left, -1), (KeyCode::Right, 1), (KeyCode::Up, -cols), (KeyCode::Down, cols)] {
            if is_key_pressed(k) {
                self.move_selection(d);
            }
        }
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
            return self.selected.map(|i| (i, ACT_PLAY));
        }
        None
    }

    pub(super) fn draw_library(&mut self, sw: f32, sh: f32, pad_action: Option<(usize, u8)>) {
        let mut action = pad_action.or(self.library_keys());
        if self.entries.is_empty() {
            self.draw_empty(sw, sh);
            return;
        }
        let vis = self.visible();
        if !vis.is_empty() && !self.selected.is_some_and(|s| vis.contains(&s)) {
            self.selected = Some(vis[0]);
        }
        let panel_w = if sw < 900.0 { 300.0 } else { 360.0 };
        let top = HEADER_H + TOOLBAR_H;
        let grid_x = 24.0;
        let grid_right = sw - 24.0 - panel_w - 24.0;
        let grid_w = (grid_right - grid_x).max(MIN_CARD_W);
        let cols = (((grid_w + GAP) / (MIN_CARD_W + GAP)).floor() as usize).max(1);
        let card_w = ((grid_w - GAP * (cols - 1) as f32) / cols as f32).min(MAX_CARD_W);
        let card_h = (card_w * 1.5).round();
        let cell_h = card_h + TITLE_SPACE + GAP;
        self.grid_cols = cols;

        let rows = vis.len().div_ceil(cols);
        let view_h = sh - top;
        let max_scroll = (rows as f32 * cell_h + 24.0 - view_h).max(0.0);
        let mouse = mouse();
        if mouse.x < grid_right && !self.block_input {
            self.scroll -= mouse_wheel().1 * 0.5;
        }
        if self.reveal {
            self.reveal = false;
            if let Some(k) = self.selected.and_then(|s| vis.iter().position(|&i| i == s)) {
                let y = (k / cols) as f32 * cell_h;
                if y < self.scroll {
                    self.scroll = y;
                } else if y + cell_h + 24.0 > self.scroll + view_h {
                    self.scroll = y + cell_h + 24.0 - view_h;
                }
            }
        }
        self.scroll = self.scroll.clamp(0.0, max_scroll);

        if vis.is_empty() {
            let msg = if self.search.trim().is_empty() { "No games here yet: choose another filter, or add some.".to_string() } else { format!("No games match \"{}\"", self.search.trim()) };
            let w = self.text_width(&msg, 18);
            self.text(&msg, grid_x + (grid_w - w) / 2.0, top + 120.0, 18, MUTED);
        }
        for (k, &i) in vis.iter().enumerate() {
            let x = grid_x + (k % cols) as f32 * (card_w + GAP);
            let y = top + 20.0 + (k / cols) as f32 * cell_h - self.scroll;
            if y + cell_h < top || y > sh {
                continue;
            }
            let card = Rect::new(x, y, card_w, card_h);
            let hover = !self.block_input && card.contains(mouse) && mouse.y > top;
            let selected = self.selected == Some(i);
            if selected {
                rrect(x - 4.0, y - 4.0, card_w + 8.0, card_h + 8.0, 8.0, ACCENT);
            }
            let path = self.entries[i].path.clone();
            let cover = self.art_for(i).0;
            match cover {
                Some(t) => draw_texture_ex(&t, x, y, WHITE, DrawTextureParams { dest_size: Some(vec2(card_w, card_h)), ..Default::default() }),
                None => {
                    rrect(x, y, card_w, card_h, 6.0, CARD);
                    if let Some(t) = self.icon(&path) {
                        let s = (card_w * 0.4).min(72.0);
                        draw_texture_ex(&t, x + (card_w - s) / 2.0, y + (card_h - s) / 2.0, WHITE, DrawTextureParams { dest_size: Some(vec2(s, s)), ..Default::default() });
                    }
                }
            }
            if hover && !selected {
                draw_rectangle(x, y, card_w, card_h, Color::new(1.0, 1.0, 1.0, 0.07));
            }
            let missing = !path.is_file();
            let running = self.running.iter().any(|(p, _)| *p == path);
            if missing || running {
                let label = if missing { "File missing" } else { "Running" };
                if missing {
                    draw_rectangle(x, y, card_w, card_h, Color::new(0.0, 0.0, 0.0, 0.55));
                }
                let w = self.bold_width(label, 13) + 16.0;
                rrect(x + 8.0, y + 8.0, w, 22.0, 11.0, if missing { DANGER } else { ACCENT });
                self.bold_text(label, x + 16.0, y + 24.0, 13, Color::new(0.08, 0.05, 0.02, 1.0));
            }
            // Favourite star (top right): always for favourites, on hover to add one.
            let fav = self.meta.is_favourite(&path);
            let star = Rect::new(x + card_w - 34.0, y + 6.0, 28.0, 28.0);
            if fav || hover {
                draw_circle(star.x + 14.0, star.y + 14.0, 14.0, Color::new(0.0, 0.0, 0.0, 0.55));
                draw_star(star.x + 14.0, star.y + 14.5, 9.0, if fav { WARN } else { Color::new(1.0, 1.0, 1.0, 0.6) });
            }
            let mut star_hit = false;
            if (fav || hover) && self.tap(star) {
                self.meta.toggle_favourite(&path);
                star_hit = true;
            }
            // Several versions of the game.
            let versions = self.versions_of(i).len();
            if versions > 1 {
                let label = format!("{versions} versions");
                let w = self.bold_width(&label, 12) + 14.0;
                rrect(x + 6.0, y + 8.0, w, 20.0, 10.0, Color::new(0.0, 0.0, 0.0, 0.6));
                self.bold_text(&label, x + 13.0, y + 23.0, 12, TEXT);
            }
            // Compatibility (bottom left), once checked.
            if let Some(c) = self.meta.get(&path).compat {
                let (label, color) = if c.unsupported == 0 { ("Runs".to_string(), ACCENT) } else { (format!("Partial: {}", c.unsupported), WARN) };
                let w = self.bold_width(&label, 12) + 14.0;
                rrect(x + 6.0, y + card_h - 26.0, w, 20.0, 10.0, color);
                self.bold_text(&label, x + 13.0, y + card_h - 11.0, 12, Color::new(0.08, 0.05, 0.02, 1.0));
            }
            let title = self.fit(&self.entries[i].title, 16, card_w);
            self.text(&title, x, y + card_h + 22.0, 16, if selected { TEXT } else { mix(TEXT, MUTED, 0.3) });
            if let Some((secs, _)) = self.totals.get(&path.to_string_lossy().to_lowercase()) {
                self.text(&format!("{} played", playlog::duration(*secs)), x, y + card_h + 40.0, 13, MUTED);
            }
            if !star_hit && mouse.y > top && self.tap(card) {
                self.selected = Some(i);
                let now = get_time();
                if self.last_click.is_some_and(|(j, t)| j == i && now - t < 0.4) {
                    action = Some((i, ACT_PLAY));
                    self.last_click = None;
                } else {
                    self.last_click = Some((i, now));
                }
            }
        }

        self.draw_toolbar(grid_x, HEADER_H + 14.0, grid_right);
        if let Some(i) = self.selected.filter(|s| vis.contains(s)) {
            if let Some(a) = self.draw_details(i, sw - 24.0 - panel_w, HEADER_H + 20.0, panel_w, sh - HEADER_H - 44.0) {
                action = Some((i, a));
            }
        }
        if let Some((i, a)) = action {
            self.game_action(i, a);
        }
    }

    /// No games yet: what to do.
    fn draw_empty(&mut self, sw: f32, sh: f32) {
        let cy = (sh + HEADER_H) / 2.0 - 60.0;
        if let Some(t) = self.logo() {
            draw_texture_ex(&t, sw / 2.0 - 36.0, cy - 110.0, WHITE, DrawTextureParams { dest_size: Some(vec2(72.0, 72.0)), ..Default::default() });
        }
        let title = "Your library is empty";
        self.bold_text(title, (sw - self.bold_width(title, 26)) / 2.0, cy, 26, TEXT);
        let hint = if cfg!(target_os = "android") {
            "Copy games from KlikCore on your PC: pair it under Devices, then open its Games."
        } else {
            "Add Clickteam Fusion games (.exe, .ccn or .dat), a folder of them, or the ones you own on Steam."
        };
        let hint = self.fit(hint, 16, sw - 48.0);
        self.text(&hint, (sw - self.text_width(&hint, 16)) / 2.0, cy + 32.0, 16, MUTED);
        let mut buttons: Vec<(&str, MenuAction, bool)> = if cfg!(target_os = "android") {
            vec![("Get games from a PC", MenuAction::GetGames, true), ("Pair a device", MenuAction::Devices, false)]
        } else {
            vec![("Add a game", MenuAction::AddGame, true), ("Add a folder", MenuAction::AddFolder, false)]
        };
        if self.has_steam {
            buttons.push(("Find Steam games", MenuAction::SteamGames, false));
        }
        let widths: Vec<f32> = buttons.iter().map(|(l, _, _)| self.bold_width(l, 16) + 40.0).collect();
        let total = widths.iter().sum::<f32>() + 12.0 * (buttons.len() - 1) as f32;
        let mut x = (sw - total) / 2.0;
        let idle = self.busy.is_none();
        let mut clicked = None;
        for ((label, act, accent), w) in buttons.into_iter().zip(widths) {
            if self.button(Rect::new(x, cy + 60.0, w, 42.0), label, accent, idle) {
                clicked = Some(act);
            }
            x += w + 12.0;
        }
        if let Some(a) = clicked {
            self.run_menu(a);
        }
    }

    /// The selected game's panel. Returns ACT_PLAY / ACT_SETTINGS when clicked.
    fn draw_details(&mut self, i: usize, x: f32, y: f32, w: f32, h: f32) -> Option<u8> {
        draw_rectangle(x, y, w, h, CARD);
        let path = self.entries[i].path.clone();
        let hero_h = (w * 0.45).round();
        match self.art_for(i).1 {
            Some(t) => {
                // Crop the banner to the panel's shape, centred.
                let (tw, th) = (t.width(), t.height());
                let want = w / hero_h;
                let src = if tw / th > want {
                    let sw = th * want;
                    Rect::new((tw - sw) / 2.0, 0.0, sw, th)
                } else {
                    let shh = tw / want;
                    Rect::new(0.0, (th - shh) / 2.0, tw, shh)
                };
                draw_texture_ex(&t, x, y, WHITE, DrawTextureParams { dest_size: Some(vec2(w, hero_h)), source: Some(src), ..Default::default() });
                // Fade into the panel.
                let steps = 24;
                let fade = hero_h * 0.55;
                for s in 0..steps {
                    let a = (s as f32 + 1.0) / steps as f32;
                    let sy = y + hero_h - fade + fade * s as f32 / steps as f32;
                    draw_rectangle(x, sy, w, fade / steps as f32 + 0.5, Color::new(CARD.r, CARD.g, CARD.b, a * a));
                }
            }
            None => {
                if let Some(t) = self.icon(&path) {
                    draw_texture_ex(&t, x + 24.0, y + 24.0, WHITE, DrawTextureParams { dest_size: Some(vec2(64.0, 64.0)), ..Default::default() });
                }
            }
        }
        let pad = 24.0;
        let inner = w - 2.0 * pad;
        let mut ty = y + hero_h - 6.0;
        // Title, up to two lines.
        let title = self.entries[i].title.clone();
        let mut lines: Vec<String> = Vec::new();
        let mut line = String::new();
        for word in title.split_whitespace() {
            let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && self.bold_width(&cand, 26) > inner {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = cand;
            }
        }
        lines.push(line);
        if lines.len() > 2 {
            let rest = lines[1..].join(" ");
            lines.truncate(1);
            lines.push(rest);
        }
        for l in &lines {
            let l = self.fit(l, 26, inner);
            self.bold_text(&l, x + pad, ty, 26, TEXT);
            ty += 32.0;
        }
        let stats = match self.totals.get(&path.to_string_lossy().to_lowercase()) {
            Some((secs, last)) => format!("Played {}  -  last {last}", playlog::duration(*secs)),
            None => "Not played yet".to_string(),
        };
        let stats = self.fit(&stats, 15, inner);
        self.text(&stats, x + pad, ty - 6.0, 15, MUTED);
        ty += 18.0;
        // Versions of the game: which one shows and plays.
        let versions = self.versions_of(i);
        if versions.len() > 1 {
            let mut cx = x + pad;
            let mut chosen = None;
            for &k in &versions {
                let label = super::meta::version_label(&self.entries[k].title);
                let label = self.fit(&label, 13, inner * 0.6);
                let cw = self.bold_width(&label, 13) + 20.0;
                if cx + cw > x + w - pad {
                    break;
                }
                let r = Rect::new(cx, ty - 14.0, cw, 24.0);
                let on = k == i;
                rrect(r.x, r.y, r.w, r.h, 12.0, if on { ACCENT } else { BUTTON });
                self.bold_text(&label, r.x + 10.0, r.y + 17.0, 13, if on { Color::new(0.08, 0.05, 0.02, 1.0) } else { TEXT });
                if !on && self.tap(r) {
                    chosen = Some(k);
                }
                cx += cw + 6.0;
            }
            ty += 30.0;
            if let Some(k) = chosen {
                let g = super::meta::version_group(&self.entries[k].title);
                let p = self.entries[k].path.clone();
                self.meta.set_preferred(&g, &p);
                self.selected = Some(k);
            }
        }
        // Compatibility, with a button to (re)check.
        let checking = self.checking.contains(&path);
        let (compat, color) = match (checking, self.meta.get(&path).compat) {
            (true, _) => ("Checking every screen...".to_string(), ACCENT),
            (_, Some(c)) if c.unsupported == 0 => ("Runs: nothing unsupported".to_string(), ACCENT),
            (_, Some(c)) => (format!("{} unsupported feature{}", c.unsupported, if c.unsupported == 1 { "" } else { "s" }), WARN),
            (_, None) => ("Compatibility not checked yet".to_string(), MUTED),
        };
        let check = Rect::new(x + w - pad - 70.0, ty - 12.0, 70.0, 26.0);
        let compat = self.fit(&compat, 14, inner - 80.0);
        self.text(&compat, x + pad, ty + 6.0, 14, color);
        if path.is_file() && self.button(check, "Check", false, !checking) {
            self.check_compat(i);
        }
        ty += 26.0;

        let missing = !path.is_file();
        let running = self.running.iter().any(|(p, _)| *p == path);
        let play = Rect::new(x + pad, ty, inner, 50.0);
        let label = if missing { "File not found" } else if running { "Running" } else { "Play" };
        let mut result = None;
        let shown = if !missing && !running { format!("      {label}") } else { label.to_string() };
        if self.button(play, &shown, true, !missing && !running) {
            result = Some(ACT_PLAY);
        }
        if !missing && !running {
            let lw = self.bold_width(&shown, 16);
            let cx = play.x + (play.w - lw) / 2.0;
            let cy = play.y + play.h / 2.0;
            draw_triangle(vec2(cx, cy - 7.0), vec2(cx, cy + 7.0), vec2(cx + 12.0, cy), Color::new(0.1, 0.06, 0.02, 1.0));
        }
        ty += 62.0;
        let more = Rect::new(x + w - pad - 50.0, ty, 50.0, 40.0);
        let settings = Rect::new(x + pad, ty, inner - 60.0, 40.0);
        if self.button(settings, "Settings", false, true) {
            result = Some(ACT_SETTINGS);
        }
        if self.icon_button(more, draw_dots) {
            let mut items = Vec::new();
            items.push((if self.meta.is_favourite(&path) { "Remove from favourites" } else { "Add to favourites" }, MenuAction::Favourite(i)));
            items.push(("Collections...", MenuAction::Collections(i)));
            if !missing {
                items.push(("Check compatibility", MenuAction::CheckCompat(i)));
            }
            items.push(("Screenshots", MenuAction::Screenshots(i)));
            items.push(("Report a problem", MenuAction::Report(i)));
            if self.has_steam && !missing {
                items.push(("Add to Steam", MenuAction::ToSteam(i)));
            }
            if !missing {
                items.push((if cfg!(target_os = "android") { "Add to home screen" } else { "Desktop shortcut" }, MenuAction::Shortcut(i)));
            }
            if !cfg!(target_os = "android") {
                items.push(("Mods folder", MenuAction::Mods(i)));
                items.push(("Open game folder", MenuAction::GameFolder(i)));
            }
            if !missing && super::upscale::supported() && self.upscaling.is_none() {
                if super::upscale::has_pack(&path) {
                    items.push(("Repair AI upscale pack", MenuAction::RepairUpscale(i)));
                    items.push(("Remove AI upscale pack", MenuAction::RemoveUpscale(i)));
                } else {
                    items.push(("Make AI upscale pack", MenuAction::Upscale(i, super::upscale::Style::Auto)));
                }
            }
            if !cfg!(target_os = "android") {
                items.push(("Choose cover picture...", MenuAction::CustomArt(i, "cover")));
                items.push(("Choose banner picture...", MenuAction::CustomArt(i, "hero")));
                if super::covers::has_custom(&path) {
                    items.push(("Remove my pictures", MenuAction::RemoveCustomArt(i)));
                }
            }
            items.push(("Refresh artwork", MenuAction::RefreshArt(i)));
            items.push(("Remove from library", MenuAction::Remove(i)));
            self.open_menu(more.x + more.w, more.y + more.h + 6.0, items, true);
        }
        ty += 66.0;
        if ty + 40.0 < y + h {
            self.text("Location", x + pad, ty, 13, MUTED);
            let p = self.fit_left(&path.display().to_string(), 14, inner);
            self.text(&p, x + pad, ty + 20.0, 14, mix(TEXT, MUTED, 0.4));
        }
        result
    }

    /// `s` shortened at the start ("...end") to fit `max_w` pixels.
    pub(super) fn fit_left(&self, s: &str, size: u16, max_w: f32) -> String {
        let mut p = s.to_string();
        while self.text_width(&p, size) > max_w && p.chars().count() > 8 {
            p = format!("...{}", p.chars().skip(4).collect::<String>());
        }
        p
    }

    /// Cover and hero textures for a game; asks the art worker the first time.
    fn art_for(&mut self, i: usize) -> (Option<Texture2D>, Option<Texture2D>) {
        let path = self.entries[i].path.clone();
        if let Some(a) = self.art.get(&path) {
            return a.clone();
        }
        if self.art_requested.insert(path.clone()) && path.is_file() {
            let _ = self.art_tx.send((path, self.entries[i].title.clone()));
        }
        (None, None)
    }

    /// Run a game action: play, settings, or a menu entry's.
    pub(super) fn game_action(&mut self, i: usize, a: u8) {
        match a {
            ACT_PLAY => self.play(i),
            ACT_SETTINGS => {
                let path = self.entries[i].path.clone();
                let legacy = crate::runtime::files::default_save_root(&self.entries[i].title);
                self.settings_panel = Some((i, Settings::for_game(&path, Some(&legacy))));
                self.saves_status = None;
            }
            _ => {}
        }
    }

    pub(super) fn run_menu(&mut self, a: MenuAction) {
        let opener = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        match a {
            MenuAction::AddGame => {
                if let Some(f) = dialogs::pick_game() {
                    self.add_paths(vec![f], "game".into());
                }
            }
            MenuAction::AddFolder => {
                if let Some(dir) = dialogs::pick_folder() {
                    let files = library::find_games(&dir);
                    self.add_paths(files, "games from folder".into());
                }
            }
            MenuAction::SteamGames => {
                self.devices_page = false;
                self.steam_panel = true;
                self.settings_panel = None;
                self.app_settings = None;
                self.extra = None;
        self.extra = None;
                self.show_log = false;
            }
            MenuAction::PlayLog => {
                self.devices_page = false;
                self.show_log = true;
                self.steam_panel = false;
                self.settings_panel = None;
                self.app_settings = None;
                self.extra = None;
        self.extra = None;
                self.scroll = 0.0;
            }
            MenuAction::DataFolder => self.open_data_folder(),
            MenuAction::GetGames => self.open_get_games(),
            MenuAction::Devices => {
                self.games_chooser = false;
                self.games_from_menu = false;
                self.devices_page = true;
                self.remote_games_for = None;
                self.steam_panel = false;
                self.settings_panel = None;
                self.app_settings = None;
                self.extra = None;
        self.extra = None;
                self.show_log = false;
                self.scroll = 0.0;
            }
            MenuAction::ToSteam(i) => {
                if steam_shortcuts::steam_running() && !steam_shortcuts::in_gaming_mode() {
                    self.steam_confirm = Some(i);
                } else {
                    self.add_to_steam(i);
                }
            }
            #[cfg(target_os = "android")]
            MenuAction::Shortcut(i) => self.home_screen_shortcut(i),
            #[cfg(not(target_os = "android"))]
            MenuAction::Shortcut(i) => {
                let e = self.entries[i].clone();
                match library::create_shortcut(&e.path, &e.title) {
                    Ok(p) => self.say(format!("Shortcut created: {}", p.display())),
                    Err(err) => self.say(format!("Could not create the shortcut: {err}")),
                }
            }
            MenuAction::Mods(i) => {
                let path = self.entries[i].path.clone();
                match crate::mods::Mods::ensure_dir(&path) {
                    Ok(dir) => {
                        let _ = std::process::Command::new(opener).arg(&dir).spawn();
                        self.say(format!("Mods folder: {} (see README.txt inside)", dir.display()));
                    }
                    Err(err) => self.say(format!("Could not create the mods folder: {err}")),
                }
            }
            MenuAction::GameFolder(i) => {
                if let Some(dir) = self.entries[i].path.parent() {
                    let _ = std::process::Command::new(opener).arg(dir).spawn();
                }
            }
            MenuAction::CustomArt(i, which) => {
                let path = self.entries[i].path.clone();
                if let Some(pic) = dialogs::pick_image() {
                    match covers::set_custom(&path, which, &pic) {
                        Ok(()) => {
                            self.art.remove(&path);
                            self.art_requested.remove(&path);
                            self.say(if which == "cover" { "Cover changed." } else { "Banner changed." });
                        }
                        Err(e) => self.say(format!("Couldn't use that picture: {e}")),
                    }
                }
            }
            MenuAction::RemoveCustomArt(i) => {
                let path = self.entries[i].path.clone();
                covers::remove_custom(&path);
                self.art.remove(&path);
                self.art_requested.remove(&path);
                self.say("Back to the game's own artwork.");
            }
            MenuAction::RefreshArt(i) => {
                let path = self.entries[i].path.clone();
                covers::clear(&path);
                self.art.remove(&path);
                self.art_requested.remove(&path);
                self.say("Making new artwork...");
            }
            MenuAction::Upscale(i, style) => {
                let path = self.entries[i].path.clone();
                self.upscaling = Some(path.clone());
                self.busy = Some("Starting the AI upscale...".into());
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let status = tx.clone();
                    let r = super::upscale::make_pack(&path, style, 0, &move |s, f| {
                        let _ = status.send(Msg::UpscaleProgress(s, f));
                    });
                    let _ = tx.send(Msg::Upscaled(r.unwrap_or_else(|e| format!("AI upscale failed: {e}"))));
                });
            }
            MenuAction::Favourite(i) => {
                if let Some(e) = self.entries.get(i).cloned() {
                    let on = self.meta.toggle_favourite(&e.path);
                    self.say(if on { format!("{} is a favourite.", e.title) } else { format!("{} is no longer a favourite.", e.title) });
                }
            }
            MenuAction::Collections(i) => self.open_extra(super::extras::Extra::Collections(i)),
            MenuAction::CheckCompat(i) => self.check_compat(i),
            MenuAction::Screenshots(i) => self.open_extra(super::extras::Extra::Screenshots(i)),
            MenuAction::Report(i) => self.open_extra(super::extras::Extra::Report(i)),
            MenuAction::Tests => self.open_extra(super::extras::Extra::Tests),
            MenuAction::AppSettings => {
                self.devices_page = false;
                self.steam_panel = false;
                self.settings_panel = None;
                self.show_log = false;
                self.app_settings = Some(Settings::defaults());
                self.scroll = 0.0;
            }
            MenuAction::RepairUpscale(i) => {
                let path = self.entries[i].path.clone();
                self.upscaling = Some(path.clone());
                self.busy = Some("Checking the AI upscale pack...".into());
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let status = tx.clone();
                    let r = super::upscale::repair_pack(&path, &move |s, f| {
                        let _ = status.send(Msg::UpscaleProgress(s, f));
                    });
                    let _ = tx.send(Msg::Upscaled(r.unwrap_or_else(|e| format!("AI upscale repair failed: {e}"))));
                });
            }
            MenuAction::RemoveUpscale(i) => {
                let path = self.entries[i].path.clone();
                match super::upscale::remove_pack(&path) {
                    Ok(()) => self.say("Removed the AI upscale pack (the game uses its original images again)."),
                    Err(e) => self.say(format!("Could not remove the pack: {e}")),
                }
            }
            MenuAction::Remove(i) => {
                let e = self.entries.remove(i);
                library::save(&self.entries);
                self.selected = None;
                self.say(format!("Removed {} from the library (the game files are untouched).", e.title));
            }
        }
    }

    /// Pop-up menu and toasts, drawn over everything.
    pub(super) fn draw_overlays(&mut self, sw: f32, sh: f32) {
        self.draw_toasts(sw, sh);
        let Some(mut m) = self.menu.clone() else { return };
        let w = self.menu_width(&m.items);
        let item_h = 38.0;
        let h = m.items.len() as f32 * item_h + 12.0;
        m.x = m.x.clamp(8.0, (sw - w - 8.0).max(8.0));
        if m.y + h > sh - 8.0 {
            m.y = (sh - 8.0 - h).max(8.0);
        }
        rrect(m.x + 2.0, m.y + 4.0, w, h, 10.0, mix(BG, Color::new(0.0, 0.0, 0.0, 1.0), 0.6));
        rrect(m.x, m.y, w, h, 10.0, MENU);
        let mouse = mouse();
        let mut chosen = None;
        for (k, (label, act)) in m.items.iter().enumerate() {
            let r = Rect::new(m.x + 6.0, m.y + 6.0 + k as f32 * item_h, w - 12.0, item_h);
            let hover = r.contains(mouse);
            if hover {
                rrect(r.x, r.y, r.w, r.h, 6.0, mix(MENU, TEXT, 0.1));
            }
            let c = if matches!(act, MenuAction::Remove(_)) { DANGER } else { TEXT };
            self.text(label, r.x + 14.0, r.y + 25.0, 16, c);
            if hover && is_mouse_button_released(MouseButton::Left) && !m.fresh {
                chosen = Some(*act);
            }
        }
        let inside = Rect::new(m.x, m.y, w, h).contains(mouse);
        let close = chosen.is_some()
            || is_key_pressed(KeyCode::Escape)
            || self.pad_back
            || (!m.fresh && !inside && is_mouse_button_pressed(MouseButton::Left));
        m.fresh = false;
        self.menu = if close { None } else { Some(m) };
        if let Some(a) = chosen {
            self.run_menu(a);
        }
    }

    fn draw_toasts(&mut self, sw: f32, sh: f32) {
        if self.steam_offer && self.busy.is_none() {
            self.draw_steam_offer(sh);
            return;
        }
        let line = match (&self.busy, &self.status) {
            (Some(b), _) => Some(b.clone()),
            (None, Some((s, until))) if get_time() < *until => Some(s.clone()),
            _ => None,
        };
        // The AI upscale job: a card with its step, a progress bar, percent and time left.
        if let (Some(s), Some((f, started))) = (&self.busy, self.upscale_progress) {
            let w = (sw - 40.0).min(460.0);
            let r = Rect::new(20.0, sh - 20.0 - 74.0, w, 74.0);
            rrect(r.x, r.y, r.w, r.h, 14.0, MENU);
            let title = self.fit(&self.upscale_title(), 15, w - 120.0);
            self.bold_text(&title, r.x + 16.0, r.y + 24.0, 15, TEXT);
            let pct = format!("{:.0}%", f * 100.0);
            self.bold_text(&pct, r.x + w - 16.0 - self.bold_width(&pct, 15), r.y + 24.0, 15, ACCENT);
            let (bx, by, bw) = (r.x + 16.0, r.y + 36.0, w - 32.0);
            rrect(bx, by, bw, 8.0, 4.0, BG);
            if f > 0.0 {
                rrect(bx, by, (bw * f).max(8.0), 8.0, 4.0, ACCENT);
            }
            let elapsed = get_time() - started;
            let left = if f > 0.03 && elapsed > 20.0 { (elapsed * (1.0 - f as f64) / f as f64) as u64 } else { u64::MAX };
            let eta = match left {
                u64::MAX => String::new(),
                l if l < 90 => " - under 2 min left".into(),
                l if l < 5400 => format!(" - about {} min left", (l + 30) / 60),
                l => format!(" - about {:.1} h left", l as f64 / 3600.0),
            };
            let line = self.fit(&format!("{s}{eta}"), 13, w - 32.0);
            self.text(&line, r.x + 16.0, r.y + 63.0, 13, MUTED);
            return;
        }
        if let Some(s) = line {
            let s = self.fit(&s, 15, sw - 80.0);
            let w = self.text_width(&s, 15) + 32.0;
            let r = Rect::new(20.0, sh - 20.0 - 36.0, w, 36.0);
            rrect(r.x, r.y, r.w, r.h, 18.0, MENU);
            self.text(&s, r.x + 16.0, r.y + 23.0, 15, TEXT);
        }
    }

    /// "AI upscale: <game>" for the progress card.
    fn upscale_title(&self) -> String {
        let name = self.upscaling.as_ref().and_then(|p| self.entries.iter().find(|e| &e.path == p)).map(|e| e.title.clone()).unwrap_or_default();
        if name.is_empty() { "AI upscale".into() } else { format!("AI upscale: {name}") }
    }
}
