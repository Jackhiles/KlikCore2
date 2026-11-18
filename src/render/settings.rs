//! Per-game player settings, in `<data dir>\KlikCore\settings\<game file name>.ini`
//! (edited by the launcher, and by F9 / F10 / F11 in game).

use std::path::{Path, PathBuf};

/// How the finished frame is enlarged to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// Hard pixel edges.
    Sharp,
    /// Bilinear.
    Smooth,
    /// Bicubic (Catmull-Rom) upscaling: smooth but crisper than bilinear.
    Enhanced,
    /// Bicubic plus edge sharpening kept within the neighbouring colours (no halos):
    /// for drawn / rendered art.
    Hd,
    /// Scale2x-style rounding of stair-stepped diagonals, crisp elsewhere: for pixel art.
    PixelArt,
}

/// How the game view is fitted into the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scaling {
    /// Largest size that keeps the shape (black bars fill the rest).
    Fit,
    /// Largest whole multiple of the game size (crisp pixels), centred.
    Integer,
    /// Fill the whole window, ignoring the shape.
    Stretch,
}

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::Sharp, Filter::Smooth, Filter::Enhanced, Filter::Hd, Filter::PixelArt];
    pub fn name(self) -> &'static str {
        match self {
            Filter::Sharp => "sharp",
            Filter::Smooth => "smooth",
            Filter::Enhanced => "enhanced",
            Filter::Hd => "hd",
            Filter::PixelArt => "pixel-art",
        }
    }

    /// The next filter in F9 order.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == s)
    }
}

impl Scaling {
    pub const ALL: [Scaling; 3] = [Scaling::Fit, Scaling::Integer, Scaling::Stretch];
    pub fn name(self) -> &'static str {
        match self {
            Scaling::Fit => "fit",
            Scaling::Integer => "integer",
            Scaling::Stretch => "stretch",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == s)
    }
}

/// On-screen touch controls (see `render::touch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchMode {
    /// Shown when the game (and the current frame) uses keys.
    Auto,
    /// Always shown.
    On,
    /// Hidden.
    Off,
}

impl TouchMode {
    pub fn name(self) -> &'static str {
        match self {
            TouchMode::Auto => "auto",
            TouchMode::On => "on",
            TouchMode::Off => "off",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        [TouchMode::Auto, TouchMode::On, TouchMode::Off].into_iter().find(|m| m.name() == s)
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    /// None = follow the game's own "fullscreen at start" setting.
    pub fullscreen: Option<bool>,
    pub filter: Filter,
    pub scaling: Scaling,
    /// Windowed size as a multiple of the game size; 0 = automatic (fits the desktop).
    pub window_scale: u8,
    /// Show the debug overlay (frame, tick, unsupported features) when the game starts.
    pub hud: bool,
    /// Map a gamepad to keyboard + mouse.
    pub pad: bool,
    /// Keys the left stick / D-pad press: None = the game's own (worked out from its
    /// events), Some(false) = arrow keys, Some(true) = W A S D.
    pub pad_wasd: Option<bool>,
    /// Right-stick cursor speed: 1 slow, 2 normal, 3 fast.
    pub pad_speed: u8,
    /// Button remaps, e.g. ("south", "13") or ("rt", "click"); see `render::gamepad`.
    pub pad_map: Vec<(String, String)>,
    /// On-screen touch controls (Android, or desktop with KLIKCORE_TOUCH_CONTROLS=1).
    pub touch_controls: TouchMode,
    /// Sound volume, 0-100.
    pub volume: u8,
    /// Use the game's mods and AI upscale packs.
    pub mods: bool,
    /// Use the AI upscale pack (when mods are on; Ctrl+U in game).
    pub ai_pack: bool,
    /// Show the controller's button legend.
    pub pad_legend: bool,
    /// Launch options: the frame to start on (0 = the game's first), extra command-line
    /// options the launcher adds, and debug panels open at the start (1 variables, 2
    /// events, 3 both).
    pub start_frame: u32,
    pub launch_args: String,
    pub panels: u8,
    /// Event log: 0 off, 1 last minute, 2 last 5 minutes, 3 whole session.
    pub event_log: u8,
    /// The event log also records objects' own movements and animations.
    pub event_log_detail: bool,
    path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fullscreen: None,
            filter: Filter::Smooth,
            scaling: Scaling::Fit,
            window_scale: 0,
            hud: false,
            pad: true,
            pad_wasd: None,
            pad_speed: 2,
            pad_map: Vec::new(),
            touch_controls: TouchMode::Auto,
            volume: 100,
            mods: true,
            ai_pack: true,
            pad_legend: true,
            start_frame: 0,
            launch_args: String::new(),
            panels: 0,
            event_log: 0,
            event_log_detail: false,
            path: None,
        }
    }
}

impl Settings {
    /// Settings file for a game file.
    pub fn path_for(game: &Path) -> PathBuf {
        let stem = game.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "game".into());
        let safe: String = stem.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
        crate::runtime::files::data_root().join("settings").join(format!("{}.ini", safe.trim()))
    }

    /// The settings games start from until they have their own (set in the launcher).
    pub fn defaults_path() -> PathBuf {
        crate::runtime::files::data_root().join("game-defaults.ini")
    }

    /// The defaults for games, to edit (saved to `defaults_path`).
    pub fn defaults() -> Self {
        let mut s = Self::default();
        if let Ok(text) = std::fs::read_to_string(Self::defaults_path()) {
            s.apply(&text);
        }
        s.path = Some(Self::defaults_path());
        s
    }

    /// Load a game's settings. `legacy_dir` is the game's save folder, where older versions
    /// kept `klikcore.ini`; it is read if the new file doesn't exist yet. A game without
    /// its own settings starts from the defaults.
    pub fn for_game(game: &Path, legacy_dir: Option<&Path>) -> Self {
        let path = Self::path_for(game);
        let mut s = Self { path: Some(path.clone()), ..Self::defaults() };
        let text = std::fs::read_to_string(&path)
            .ok()
            .or_else(|| legacy_dir.and_then(|d| std::fs::read_to_string(d.join("klikcore.ini")).ok()));
        if let Some(text) = text {
            s.apply(&text);
        }
        s.with_env_overrides()
    }

    /// Testing: KLIKCORE_SETTINGS="filter=hd;window_scale=3" overrides settings for this
    /// run (written back only if the player changes a setting in game).
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(o) = std::env::var("KLIKCORE_SETTINGS") {
            self.apply(&o.replace(';', "\n"));
        }
        self
    }

    fn apply(&mut self, text: &str) {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().to_ascii_lowercase();
            let on = matches!(v.as_str(), "1" | "true" | "yes" | "on");
            match k.trim() {
                "fullscreen" => self.fullscreen = if v == "default" { None } else { Some(on) },
                // Older files: smooth=0/1.
                "smooth" => self.filter = if on { Filter::Smooth } else { Filter::Sharp },
                "filter" => self.filter = Filter::parse(&v).unwrap_or(self.filter),
                "scaling" => self.scaling = Scaling::parse(&v).unwrap_or(self.scaling),
                "window_scale" => self.window_scale = v.parse::<u8>().unwrap_or(0).min(4),
                "hud" => self.hud = on,
                "controller" => self.pad = on,
                // Older files always wrote controller_move (arrows by default), so only an
                // explicit W A S D carries over.
                "controller_move" if v == "wasd" => self.pad_wasd = Some(true),
                "controller_keys" => self.pad_wasd = match v.as_str() { "wasd" => Some(true), "arrows" => Some(false), _ => None },
                "controller_cursor_speed" => self.pad_speed = v.parse::<u8>().unwrap_or(2).clamp(1, 3),
                "touch_controls" => self.touch_controls = TouchMode::parse(&v).unwrap_or(self.touch_controls),
                "volume" => self.volume = v.parse::<u8>().unwrap_or(100).min(100),
                "mods" => self.mods = on,
                "ai_pack" => self.ai_pack = on,
                "controller_legend" => self.pad_legend = on,
                "start_frame" => self.start_frame = v.parse::<u32>().unwrap_or(0),
                "panels" => self.panels = v.parse::<u8>().unwrap_or(0).min(3),
                "event_log" => self.event_log = v.parse::<u8>().unwrap_or(0).min(3),
                "event_log_detail" => self.event_log_detail = on,
                // Kept as typed (the value above is lower-cased).
                "launch_args" => self.launch_args = line.split_once('=').map_or(String::new(), |(_, v)| v.trim().to_string()),
                key if key.starts_with("pad_") => {
                    let button = key[4..].to_string();
                    self.pad_map.retain(|(b, _)| *b != button);
                    self.pad_map.push((button, v.clone()));
                }
                _ => {}
            }
        }
    }

    pub fn to_text(&self) -> String {
        let mut text = String::from("[klikcore]\n");
        match self.fullscreen {
            Some(f) => text += &format!("fullscreen={}\n", f as u8),
            None => text += "fullscreen=default\n",
        }
        text += &format!("filter={}\n", self.filter.name());
        text += &format!("scaling={}\n", self.scaling.name());
        text += &format!("window_scale={}\n", self.window_scale);
        text += &format!("hud={}\n", self.hud as u8);
        text += &format!("controller={}\n", self.pad as u8);
        text += &format!("controller_keys={}\n", match self.pad_wasd { None => "auto", Some(false) => "arrows", Some(true) => "wasd" });
        text += &format!("controller_cursor_speed={}\n", self.pad_speed);
        text += &format!("touch_controls={}\n", self.touch_controls.name());
        text += &format!("volume={}\n", self.volume);
        text += &format!("mods={}\n", self.mods as u8);
        text += &format!("ai_pack={}\n", self.ai_pack as u8);
        text += &format!("controller_legend={}\n", self.pad_legend as u8);
        text += &format!("start_frame={}\n", self.start_frame);
        text += &format!("panels={}\n", self.panels);
        text += &format!("event_log={}\n", self.event_log);
        text += &format!("event_log_detail={}\n", self.event_log_detail as u8);
        text += &format!("launch_args={}\n", self.launch_args.replace(['\n', '\r'], " "));
        for (b, v) in &self.pad_map {
            text += &format!("pad_{b}={v}\n");
        }
        text
    }

    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, self.to_text()) {
            eprintln!("could not save {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_legacy_keys() {
        let mut s = Settings::default();
        s.fullscreen = Some(true);
        s.filter = Filter::Enhanced;
        s.scaling = Scaling::Integer;
        s.window_scale = 2;
        s.hud = true;
        s.touch_controls = TouchMode::Off;
        s.volume = 40;
        s.mods = false;
        s.pad_legend = false;
        let mut t = Settings::default();
        t.apply(&s.to_text());
        assert_eq!((t.volume, t.mods, t.pad_legend), (40, false, false));
        assert_eq!((t.fullscreen, t.filter, t.scaling, t.window_scale, t.hud), (Some(true), Filter::Enhanced, Scaling::Integer, 2, true));
        assert_eq!(t.touch_controls, TouchMode::Off);
        assert_eq!(Settings::default().touch_controls, TouchMode::Auto);
        let mut old = Settings::default();
        old.apply("[klikcore]\nfullscreen=0\nsmooth=0\n");
        assert_eq!((old.fullscreen, old.filter), (Some(false), Filter::Sharp));
    }
}
