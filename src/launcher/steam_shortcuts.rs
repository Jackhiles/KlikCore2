//! Add games to Steam as non-Steam shortcuts that start them with KlikCore.
//!
//! Steam keeps them in `userdata/<account>/config/shortcuts.vdf`, a binary key/value file,
//! and their artwork in `userdata/<account>/config/grid/`, named after the shortcut's app
//! id. When the game is (or was) a Steam game, its store artwork is copied over from
//! Steam's own cache so the shortcut looks like the original.
//!
//! Steam reads the file when it starts and may write it back from memory, so it is only
//! edited while Steam is closed (see [`steam_running`], [`shutdown_steam`]).

use super::steam;
use std::path::{Path, PathBuf};

/// A binary VDF value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Map(Vec<(String, Value)>),
    Str(String),
    Int(u32),
    U64(u64),
}

fn read_cstr(b: &[u8], pos: &mut usize) -> Option<String> {
    let end = b[*pos..].iter().position(|&c| c == 0)? + *pos;
    let s = String::from_utf8_lossy(&b[*pos..end]).into_owned();
    *pos = end + 1;
    Some(s)
}

fn read_map(b: &[u8], pos: &mut usize) -> Option<Vec<(String, Value)>> {
    let mut out = Vec::new();
    loop {
        let t = *b.get(*pos)?;
        *pos += 1;
        if t == 8 {
            return Some(out);
        }
        let key = read_cstr(b, pos)?;
        let v = match t {
            0 => Value::Map(read_map(b, pos)?),
            1 => Value::Str(read_cstr(b, pos)?),
            2 => {
                let v = u32::from_le_bytes(b.get(*pos..*pos + 4)?.try_into().ok()?);
                *pos += 4;
                Value::Int(v)
            }
            7 => {
                let v = u64::from_le_bytes(b.get(*pos..*pos + 8)?.try_into().ok()?);
                *pos += 8;
                Value::U64(v)
            }
            _ => return None,
        };
        out.push((key, v));
    }
}

/// Parse a binary VDF file (the root map's entries).
pub fn parse(b: &[u8]) -> Option<Vec<(String, Value)>> {
    let mut pos = 0;
    read_map(b, &mut pos)
}

fn write_map(m: &[(String, Value)], out: &mut Vec<u8>) {
    for (k, v) in m {
        let (t, key) = match v {
            Value::Map(_) => (0u8, k),
            Value::Str(_) => (1, k),
            Value::Int(_) => (2, k),
            Value::U64(_) => (7, k),
        };
        out.push(t);
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        match v {
            Value::Map(m) => write_map(m, out),
            Value::Str(s) => {
                out.extend_from_slice(s.as_bytes());
                out.push(0);
            }
            Value::Int(i) => out.extend_from_slice(&i.to_le_bytes()),
            Value::U64(i) => out.extend_from_slice(&i.to_le_bytes()),
        }
    }
    out.push(8);
}

pub fn serialize(root: &[(String, Value)]) -> Vec<u8> {
    let mut out = Vec::new();
    write_map(root, &mut out);
    out
}

/// The id Steam gives a non-Steam shortcut (also the name of its artwork files).
pub fn shortcut_app_id(exe: &str, name: &str) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(exe.as_bytes());
    crc.update(name.as_bytes());
    crc.sum() | 0x8000_0000
}

/// What to add.
pub struct Shortcut {
    pub name: String,
    /// KlikCore itself.
    pub exe: PathBuf,
    /// The game file, passed to KlikCore (None: the shortcut opens the launcher).
    pub game: Option<PathBuf>,
    pub icon: Option<PathBuf>,
}

fn quoted(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

/// Add (or update) the shortcut in a `shortcuts.vdf` image. Returns the new file and the app id.
pub fn add_to_vdf(existing: Option<&[u8]>, s: &Shortcut) -> Result<(Vec<u8>, u32), String> {
    let mut root = match existing {
        Some(b) if !b.is_empty() => parse(b).ok_or("Steam's shortcuts file couldn't be read")?,
        _ => Vec::new(),
    };
    if root.is_empty() {
        root.push(("shortcuts".into(), Value::Map(Vec::new())));
    }
    let Some((_, Value::Map(list))) = root.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case("shortcuts")) else {
        return Err("Steam's shortcuts file has an unexpected layout".into());
    };
    let exe = quoted(&s.exe);
    let launch = s.game.as_deref().map(quoted).unwrap_or_default();
    let id = shortcut_app_id(&exe, &s.name);
    let dir = s.game.as_deref().unwrap_or(&s.exe).parent().map(quoted).unwrap_or_default();
    let entry = vec![
        ("appid".to_string(), Value::Int(id)),
        ("appname".into(), Value::Str(s.name.clone())),
        ("Exe".into(), Value::Str(exe.clone())),
        ("StartDir".into(), Value::Str(dir)),
        ("icon".into(), Value::Str(s.icon.as_ref().map(|p| p.display().to_string()).unwrap_or_default())),
        ("ShortcutPath".into(), Value::Str(String::new())),
        ("LaunchOptions".into(), Value::Str(launch.clone())),
        ("IsHidden".into(), Value::Int(0)),
        ("AllowDesktopConfig".into(), Value::Int(1)),
        ("AllowOverlay".into(), Value::Int(1)),
        ("OpenVR".into(), Value::Int(0)),
        ("Devkit".into(), Value::Int(0)),
        ("DevkitGameID".into(), Value::Str(String::new())),
        ("DevkitOverrideAppID".into(), Value::Int(0)),
        ("LastPlayTime".into(), Value::Int(0)),
        ("FlatpakAppID".into(), Value::Str(String::new())),
        // Shown as a "KlikCore" collection in Steam's library.
        ("tags".into(), Value::Map(if s.game.is_some() { vec![("0".into(), Value::Str("KlikCore".into()))] } else { vec![] })),
    ];
    // Same game through KlikCore already there (any name): replace it in place.
    let field = |m: &[(String, Value)], key: &str| {
        m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).and_then(|(_, v)| match v {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
    };
    let same = list.iter_mut().find(|(_, v)| match v {
        Value::Map(m) => field(m, "Exe").as_deref() == Some(&exe) && field(m, "LaunchOptions").as_deref() == Some(&launch),
        _ => false,
    });
    match same {
        Some((_, v)) => *v = Value::Map(entry),
        None => {
            let next = list.iter().filter_map(|(k, _)| k.parse::<usize>().ok()).max().map_or(0, |n| n + 1);
            list.push((next.to_string(), Value::Map(entry)));
        }
    }
    Ok((serialize(&root), id))
}

/// The `userdata/<account>/config` folder of the account that last signed in to Steam.
pub fn user_config_dir() -> Option<PathBuf> {
    for root in steam::steam_roots() {
        let userdata = root.join("userdata");
        // loginusers.vdf: "<SteamID64>" { ... "MostRecent" "1" ... }
        let mut recent = None;
        if let Ok(text) = std::fs::read_to_string(root.join("config").join("loginusers.vdf")) {
            let mut current: Option<u64> = None;
            for line in text.lines() {
                let parts: Vec<&str> = line.split('"').collect();
                if parts.len() == 3 && parts[1].len() == 17 {
                    current = parts[1].parse().ok();
                } else if parts.len() >= 5 && parts[1].eq_ignore_ascii_case("mostrecent") && parts[3] == "1" {
                    recent = current;
                }
            }
        }
        if let Some(id64) = recent {
            let dir = userdata.join((id64.wrapping_sub(76_561_197_960_265_728) as u32).to_string()).join("config");
            if dir.is_dir() {
                return Some(dir);
            }
        }
        // Otherwise the most recently used account folder.
        let newest = std::fs::read_dir(&userdata)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) && e.file_name() != "0")
            .filter_map(|e| Some((e.path().join("config").metadata().ok()?.modified().ok()?, e.path().join("config"))))
            .max_by_key(|(t, _)| *t);
        if let Some((_, dir)) = newest {
            return Some(dir);
        }
    }
    None
}

/// The Steam app a game file belongs to: installed under a Steam library, or copied out of
/// one into a folder named after the game.
pub fn steam_app_for(game: &Path) -> Option<String> {
    let game = std::fs::canonicalize(game).unwrap_or_else(|_| game.to_path_buf());
    let lower = |p: &Path| {
        let s = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().into_owned();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    let g = lower(&game);
    let folder_names: Vec<String> = game.ancestors().skip(1).take(3).filter_map(|a| a.file_name()).map(|n| n.to_string_lossy().to_lowercase()).collect();
    let mut by_name = None;
    for root in steam::steam_roots() {
        for lib in steam::libraries(&root) {
            for (id, name, folder) in steam::installed(&lib) {
                let f = lower(&folder);
                if g.starts_with(&f) && g[f.len()..].starts_with(std::path::MAIN_SEPARATOR) {
                    return Some(id);
                }
                if by_name.is_none() && folder_names.iter().any(|n| *n == name.to_lowercase()) {
                    by_name = Some(id);
                }
            }
        }
    }
    by_name
}

/// Linux / Steam Deck: the `drive_c` of the Proton prefix a game's original .exe runs in:
/// the Steam game's, or the non-Steam shortcut's that starts this .exe (cached).
pub fn proton_prefix(game: &Path) -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, Option<PathBuf>>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(game).cloned()) {
        return hit;
    }
    let mut ids: Vec<String> = steam_app_for(game).into_iter().collect();
    ids.extend(shortcut_ids_for(game).into_iter().map(|id| id.to_string()));
    let found = steam::steam_roots().iter().flat_map(|r| steam::libraries(r)).find_map(|lib| {
        ids.iter().map(|id| lib.join("steamapps").join("compatdata").join(id).join("pfx").join("drive_c")).find(|p| p.is_dir())
    });
    if let Ok(mut c) = cache.lock() {
        c.insert(game.to_path_buf(), found.clone());
    }
    found
}

/// App ids of non-Steam shortcuts that start this .exe directly (with Proton).
fn shortcut_ids_for(game: &Path) -> Vec<u32> {
    let Some(file) = user_config_dir().map(|d| d.join("shortcuts.vdf")) else { return Vec::new() };
    let Some(root) = std::fs::read(file).ok().and_then(|b| parse(&b)) else { return Vec::new() };
    let Some((_, Value::Map(list))) = root.iter().find(|(k, _)| k.eq_ignore_ascii_case("shortcuts")) else { return Vec::new() };
    let want = std::fs::canonicalize(game).unwrap_or_else(|_| game.to_path_buf());
    list.iter()
        .filter_map(|(_, v)| {
            let Value::Map(m) = v else { return None };
            let exe = m.iter().find(|(k, _)| k.eq_ignore_ascii_case("Exe")).and_then(|(_, v)| match v {
                Value::Str(s) => Some(s.trim().trim_matches('"').to_string()),
                _ => None,
            })?;
            let exe = std::fs::canonicalize(&exe).unwrap_or_else(|_| PathBuf::from(&exe));
            if exe != want {
                return None;
            }
            m.iter().find(|(k, _)| k.eq_ignore_ascii_case("appid")).and_then(|(_, v)| match v {
                Value::Int(i) => Some(*i),
                _ => None,
            })
        })
        .collect()
}

/// Steam's cached store artwork for an app: (grid file suffix, source file).
pub fn steam_art(app_id: &str) -> Vec<(&'static str, PathBuf)> {
    // grid name suffix, file name in the cache folder, old flat cache name
    const KINDS: [(&str, &str); 4] = [
        ("p", "library_600x900"),
        ("", "header"),
        ("_hero", "library_hero"),
        ("_logo", "logo"),
    ];
    let mut out = Vec::new();
    for root in steam::steam_roots() {
        let cache = root.join("appcache").join("librarycache");
        let dir = cache.join(app_id);
        for (suffix, stem) in KINDS {
            if out.iter().any(|(s, _)| *s == suffix) {
                continue;
            }
            let mut found = None;
            for ext in ["jpg", "png"] {
                // Current layout: librarycache/<id>/<name> (sometimes one folder deeper).
                let direct = dir.join(format!("{stem}.{ext}"));
                if direct.is_file() {
                    found = Some(direct);
                    break;
                }
                let nested = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path().join(format!("{stem}.{ext}"))).find(|p| p.is_file());
                if nested.is_some() {
                    found = nested;
                    break;
                }
                // Older layout: librarycache/<id>_<name>
                let flat = cache.join(format!("{app_id}_{stem}.{ext}"));
                if flat.is_file() {
                    found = Some(flat);
                    break;
                }
            }
            if let Some(p) = found {
                out.push((suffix, p));
            }
        }
    }
    out
}

/// A Steam game's icon on Steam's servers (for Discord). Steam's cache names the icon after
/// its hash (`librarycache/<id>/<40 hex digits>.jpg`), which is also its public address.
pub fn steam_icon_url(app_id: &str) -> Option<String> {
    for root in steam::steam_roots() {
        let dir = root.join("appcache").join("librarycache").join(app_id);
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(hash) = name.strip_suffix(".jpg") {
                if hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(format!("https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{app_id}/{hash}.jpg"));
                }
            }
        }
    }
    None
}

/// Copy artwork into the grid folder under the shortcut's id. Returns how many were copied.
pub fn install_art(config_dir: &Path, shortcut_id: u32, art: &[(&str, PathBuf)]) -> usize {
    let grid = config_dir.join("grid");
    let _ = std::fs::create_dir_all(&grid);
    let mut n = 0;
    for (suffix, src) in art {
        let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "jpg".into());
        if std::fs::copy(src, grid.join(format!("{shortcut_id}{suffix}.{ext}"))).is_ok() {
            n += 1;
        }
    }
    n
}

/// Does Steam already have a shortcut that opens this KlikCore (the launcher, no game)?
pub fn has_launcher_shortcut() -> bool {
    if super::update::program_path().is_err() {
        return true;
    }
    launcher_shortcut().is_some()
}

/// Steam's shortcut to KlikCore itself, if there is one: its app id and name.
fn launcher_shortcut() -> Option<(u32, String)> {
    let exe = quoted(&super::update::program_path().ok()?);
    let file = user_config_dir()?.join("shortcuts.vdf");
    let root = std::fs::read(file).ok().and_then(|b| parse(&b))?;
    let Some((_, Value::Map(list))) = root.iter().find(|(k, _)| k.eq_ignore_ascii_case("shortcuts")) else { return None };
    list.iter().find_map(|(_, v)| {
        let Value::Map(m) = v else { return None };
        let get = |key: &str| {
            m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).and_then(|(_, v)| match v {
                Value::Str(s) => Some(s.trim().to_string()),
                _ => None,
            })
        };
        // Steam itself may store the path with or without quotes.
        let e = get("Exe").unwrap_or_default();
        if !((e == exe || format!("\"{e}\"") == exe) && get("LaunchOptions").unwrap_or_default().is_empty()) {
            return None;
        }
        let name = get("appname").unwrap_or_default();
        let id = m.iter().find(|(k, _)| k.eq_ignore_ascii_case("appid")).and_then(|(_, v)| match v {
            Value::Int(i) => Some(*i),
            _ => None,
        });
        Some((id.unwrap_or_else(|| shortcut_app_id(&e, &name)), name))
    })
}

/// Bump when KlikCore's own Steam artwork changes, so existing shortcuts get the new one.
const BRAND_ART_VERSION: &str = "2";

/// Keep KlikCore's own shortcut up to date: an old one (made under the Rekindle name) is
/// renamed when Steam is closed, and its artwork is replaced when ours is newer. Artwork
/// files don't need Steam closed. Returns a message when something changed.
pub fn refresh_launcher(steam_open: bool) -> Option<String> {
    let (id, name) = launcher_shortcut()?;
    if name != "KlikCore 2" && !steam_open {
        return add_launcher(false, |_| {}).ok().map(|_| "Updated KlikCore's Steam shortcut.".to_string());
    }
    let dir = super::library::data_dir().join("steam").join("klikcore");
    let stamp = dir.join("art-version.txt");
    if std::fs::read_to_string(&stamp).ok().as_deref() == Some(BRAND_ART_VERSION) {
        return None;
    }
    let art = super::art::generate_brand(&dir);
    let n = install_art(&user_config_dir()?, id, &art);
    if n > 0 {
        let _ = std::fs::write(&stamp, BRAND_ART_VERSION);
    }
    None
}

/// Add KlikCore itself (the launcher) to Steam, with artwork made from its icon.
pub fn add_launcher(may_restart: bool, progress: impl Fn(&str)) -> Result<String, String> {
    let exe = super::update::program_path().map_err(|e| e.to_string())?;
    let dir = super::library::data_dir().join("steam").join("klikcore");
    let _ = std::fs::create_dir_all(&dir);
    let icon = dir.join("icon.png");
    let icon = std::fs::write(&icon, super::ICON_PNG).ok().map(|_| icon);
    let art = super::art::generate_brand(&dir);
    let shortcut = Shortcut { name: "KlikCore 2".into(), exe, game: None, icon };
    write_with_restart(&shortcut, &art, may_restart, progress)?;
    let _ = std::fs::write(dir.join("art-version.txt"), BRAND_ART_VERSION);
    Ok("Added KlikCore to Steam.".into())
}

/// Write a shortcut, closing and restarting Steam around it when allowed and needed.
fn write_with_restart(s: &Shortcut, art: &[(&str, PathBuf)], may_restart: bool, progress: impl Fn(&str)) -> Result<(usize, bool), String> {
    let running = steam_running();
    let restart = running && may_restart && !in_gaming_mode();
    if restart {
        progress("Closing Steam...");
        if !shutdown_steam() {
            return Err("Steam didn't close; close it yourself and try again".into());
        }
    }
    progress("Adding the shortcut...");
    let n = add(s, art);
    if restart {
        progress("Starting Steam...");
        start_steam();
    }
    Ok((n?, running && !restart))
}

/// KlikCore's own Steam artwork (for its launcher shortcut), into `dir`.
pub fn preview_brand_art(dir: &Path) -> Vec<PathBuf> {
    super::art::generate_brand(dir).into_iter().map(|(_, p)| p).collect()
}

/// Make the artwork KlikCore would give a game without Steam artwork, into `dir`.
pub fn preview_art(game: &Path, title: &str, dir: &Path) -> Vec<PathBuf> {
    load_game(game).map(|g| super::art::generate(&g, title, dir).into_iter().map(|(_, p)| p).collect()).unwrap_or_default()
}

/// Write the shortcut (Steam must not be running). Returns how many artwork files were added.
pub fn add(s: &Shortcut, art: &[(&str, PathBuf)]) -> Result<usize, String> {
    let config = user_config_dir().ok_or("no Steam account found on this computer")?;
    let file = config.join("shortcuts.vdf");
    let existing = std::fs::read(&file).ok();
    let (bytes, id) = add_to_vdf(existing.as_deref(), s)?;
    if let Some(old) = &existing {
        let backup = config.join("shortcuts.vdf.klikcore-backup");
        if !backup.exists() {
            let _ = std::fs::write(&backup, old);
        }
    }
    let tmp = config.join("shortcuts.vdf.klikcore-tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
    Ok(install_art(&config, id, art))
}

/// Is the Steam client running?
pub fn steam_running() -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("steam.exe"))
    }
    #[cfg(not(windows))]
    {
        std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|e| {
            std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| matches!(c.trim(), "steam" | "steamwebhelper"))
        })
    }
}

/// Running inside Steam's Gaming Mode (where closing Steam would close KlikCore too).
pub fn in_gaming_mode() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.eq_ignore_ascii_case("gamescope")) || std::env::var_os("SteamGamepadUI").is_some()
}

fn steam_command() -> std::process::Command {
    #[cfg(windows)]
    {
        let exe = steam::steam_roots().into_iter().map(|r| r.join("steam.exe")).find(|p| p.is_file()).unwrap_or_else(|| PathBuf::from("steam.exe"));
        std::process::Command::new(exe)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("steam")
    }
}

/// Ask Steam to close and wait (up to ~40 s) until it has.
pub fn shutdown_steam() -> bool {
    let _ = steam_command().arg("-shutdown").spawn();
    for _ in 0..80 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if !steam_running() {
            // Give it a moment to finish writing its files.
            std::thread::sleep(std::time::Duration::from_secs(1));
            return true;
        }
    }
    false
}

pub fn start_steam() {
    let mut c = steam_command();
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let _ = c.spawn();
}

/// Load a game for its icon / pictures (a ".dat" game's icon lives in the .exe next to it).
pub(super) fn load_game(game: &Path) -> Option<crate::ctf_loader::GameData> {
    let src = if game.extension().is_some_and(|e| e.eq_ignore_ascii_case("dat")) && game.with_extension("exe").is_file() {
        game.with_extension("exe")
    } else {
        game.to_path_buf()
    };
    crate::ctf_loader::GameData::load(&src).ok()
}

/// Where generated files for a game go: `<data dir>/steam/<id>/`.
fn work_dir(game: &Path) -> Option<PathBuf> {
    let small = super::library::icon_path(game);
    Some(super::library::data_dir().join("steam").join(small.file_stem()?))
}

/// The game's own icon as a 256 x 256 PNG for Steam.
fn icon_for(g: &crate::ctf_loader::GameData, dir: &Path) -> Option<PathBuf> {
    let icon = g.icon.as_ref()?;
    std::fs::create_dir_all(dir).ok()?;
    let out = dir.join("icon.png");
    super::library::write_png(&out, 256, &icon.resized(256)).ok()?;
    Some(out)
}

/// Add a game to Steam: closes Steam first if it's running (and `may_restart`), writes the
/// shortcut with the game's Steam artwork when it has any, then starts Steam again.
/// `progress` gets status lines. Returns a summary for the user.
pub fn add_game(game: &Path, title: &str, may_restart: bool, progress: impl Fn(&str)) -> Result<String, String> {
    let exe = super::update::program_path().map_err(|e| e.to_string())?;
    let game = std::path::absolute(game).unwrap_or_else(|_| game.to_path_buf());
    progress("Preparing artwork...");
    let data = load_game(&game);
    let dir = work_dir(&game);
    let icon = data.as_ref().zip(dir.as_deref()).and_then(|(g, d)| icon_for(g, d));
    // "(KlikCore)" tells it apart from a Steam copy of the same game.
    let shortcut = Shortcut { name: format!("{title} (KlikCore)"), exe, game: Some(game.clone()), icon };
    let app = steam_app_for(&game);
    let mut art = app.as_deref().map(steam_art).unwrap_or_default();
    // Not a Steam game (or Steam has no pictures of it): make artwork from the game itself.
    let generated = art.is_empty();
    if generated {
        if let (Some(g), Some(d)) = (&data, &dir) {
            art = super::art::generate(g, title, d);
        }
    }
    drop(data);
    let (n, later) = write_with_restart(&shortcut, &art, may_restart, progress)?;
    let art_note = match (generated, n) {
        (false, _) => " with its Steam artwork",
        (true, n) if n > 0 => " with artwork made from the game",
        _ => "",
    };
    Ok(if later {
        format!("Added {title} to Steam{art_note}. It appears after Steam restarts.")
    } else {
        format!("Added {title} to Steam{art_note}.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_replaces_shortcuts() {
        // One existing shortcut written the way Steam writes them.
        let existing = serialize(&[(
            "shortcuts".into(),
            Value::Map(vec![(
                "0".into(),
                Value::Map(vec![
                    ("appid".into(), Value::Int(0xe4e8_c1bb)),
                    ("appname".into(), Value::Str("Other".into())),
                    ("Exe".into(), Value::Str("\"/x/other\"".into())),
                    ("LastPlayTime".into(), Value::Int(5)),
                    ("tags".into(), Value::Map(vec![])),
                ]),
            )]),
        )]);
        let s = Shortcut { name: "My Game".into(), exe: "/apps/KlikCore".into(), game: Some("/games/My Game/game.exe".into()), icon: None };
        let (once, id) = add_to_vdf(Some(&existing), &s).unwrap();
        assert_eq!(id, shortcut_app_id("\"/apps/KlikCore\"", "My Game"));
        assert!(id & 0x8000_0000 != 0);
        let root = parse(&once).unwrap();
        let Value::Map(list) = &root[0].1 else { panic!() };
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].0, "1");
        // Adding the same game again (renamed) replaces it instead of duplicating it.
        let renamed = Shortcut { name: "Renamed".into(), ..s };
        let (twice, _) = add_to_vdf(Some(&once), &renamed).unwrap();
        let root = parse(&twice).unwrap();
        let Value::Map(list) = &root[0].1 else { panic!() };
        assert_eq!(list.len(), 2);
        assert_eq!(serialize(&parse(&existing).unwrap()), existing, "round trip is exact");
    }
}
