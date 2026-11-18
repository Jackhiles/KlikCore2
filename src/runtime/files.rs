//! Sandboxed file access for games (save files, INI files, ...).
//!
//! Games build real Windows paths ("<exe dir>\save1.txt", "%APPDATA%\MMFApplications\game").
//! Writes are redirected into a per-game save folder so the runtime never touches the
//! original game's files; reads look in that folder first and fall back to the real
//! path, so existing saves from the original game are picked up (read-only).

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SaveFs {
    /// The game executable / data file.
    pub app_file: PathBuf,
    /// Directory containing it.
    pub app_dir: PathBuf,
    /// Where writes go.
    pub save_root: PathBuf,
    /// The real roaming AppData folder (Windows), used to map AppData paths.
    pub appdata: Option<PathBuf>,
    /// Linux / Steam Deck: the `drive_c` folder of the Proton prefix the original game runs
    /// in (where its AppData and C:\ saves are), for the save manager.
    pub prefix: Option<PathBuf>,
    /// Windows' VirtualStore: where old games' writes to protected places (C:\, Program
    /// Files) really went, so their saves are read from there.
    pub virtual_store: Option<PathBuf>,
    /// Tests: never read the player's real AppData files (saves made outside the sandbox),
    /// only the sandbox and the game's own folder.
    pub isolated: bool,
    /// Original files already noted in ORIGINALS_LIST this session.
    noted: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

/// In a save folder: the game paths of original files the game has read (one per line),
/// so the save manager knows which original files belong to the game.
pub const ORIGINALS_LIST: &str = "original-files.txt";

fn norm(p: &str) -> String {
    p.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Default save folder for a game title: %APPDATA%\KlikCore\<title> (or ~/.local/share/KlikCore/<title>).
/// A text file's contents as a string: UTF-16 (little or big endian) or UTF-8 when it starts
/// with that byte-order mark, otherwise UTF-8 (invalid bytes replaced).
/// Open a web address in the default browser. On Windows through the URL handler
/// (`explorer` fails on long addresses with a query, like a new-issue link with its text).
pub fn open_web(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    r.map(|_| ())
}

pub fn decode_text(b: &[u8]) -> String {
    let utf16 = |b: &[u8], be: bool| {
        let units: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match b {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(b).into_owned(),
    }
}

pub fn default_save_root(title: &str) -> PathBuf {
    let safe: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.'".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = if safe.trim().is_empty() {
        "game".to_string()
    } else {
        safe.trim().to_string()
    };
    let dir = data_root().join(safe);
    // Files that had the old names inside a game's folder.
    for (old, new) in [
        ("rekindle.ini", "klikcore.ini"),
        ("rekindle.log", "klikcore.log"),
        ("fusion-rt.ini", "klikcore.ini"),
        ("fusion-rt.log", "klikcore.log"),
    ] {
        let (o, n) = (dir.join(old), dir.join(new));
        if o.is_file() && !n.exists() {
            let _ = std::fs::rename(o, n);
        }
    }
    dir
}

/// The save folder of a game file whose header names it `name` (the file name if empty).
pub fn save_root_for(game: &Path, name: &str) -> PathBuf {
    if name.is_empty() {
        default_save_root(&game.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
    } else {
        default_save_root(name)
    }
}

/// Android: the app's internal and external (USB-visible) files folders, from the activity.
#[cfg(target_os = "android")]
static ANDROID_DIRS: std::sync::OnceLock<(Option<PathBuf>, Option<PathBuf>)> = std::sync::OnceLock::new();

/// Android: remember the app's files folders (`Context.getFilesDir()` / `getExternalFilesDir()`).
#[cfg(target_os = "android")]
pub fn set_android_dirs(internal: Option<PathBuf>, external: Option<PathBuf>) {
    let _ = ANDROID_DIRS.set((internal, external));
}

/// Android: the app's external files folder (`Android/data/<package>/files`), which a PC
/// can reach over USB.
#[cfg(target_os = "android")]
pub fn android_external_dir() -> Option<PathBuf> {
    ANDROID_DIRS.get().and_then(|d| d.1.clone())
}

/// Android: `<app files dir>/KlikCore`.
#[cfg(target_os = "android")]
pub fn data_root() -> PathBuf {
    let base = ANDROID_DIRS
        .get()
        .and_then(|d| d.0.clone())
        .unwrap_or_else(|| PathBuf::from("/data/data/com.takobyte.klikcore/files"));
    base.join("KlikCore")
}

/// `%APPDATA%\KlikCore` (or the platform's data folder): saves, settings and the launcher library.
#[cfg(not(target_os = "android"))]
pub fn data_root() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."));
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| migrate_old_saves(&base, base.join("KlikCore"))).clone()
}

/// Data from before the project's renames (`Rekindle`, earlier `fusion-rt`) moves to the
/// new folder once, leaving a link at the old name. If the old folder can't be moved (something has it open), it stays in
/// use, so nothing is lost.
#[cfg(not(target_os = "android"))]
fn migrate_old_saves(base: &Path, root: PathBuf) -> PathBuf {
    if root.exists() {
        return root;
    }
    for old_name in ["Rekindle", "fusion-rt"] {
        let old = base.join(old_name);
        if old.is_dir() {
            if std::fs::rename(&old, &root).is_err() {
                return old;
            }
            // Paths saved elsewhere still point into the old folder (Steam shortcuts to
            // synced games and their icons, library entries): leave a link to the new one.
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(&root, &old);
            #[cfg(windows)]
            let _ = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(&old).arg(&root).output();
            return root;
        }
    }
    root
}

/// The AppData folder games see when not on Windows: Proton's, so paths match it.
pub const PROTON_APPDATA: &str = r"C:\users\steamuser\AppData\Roaming";

/// Before 2.0.5, games outside Windows had no AppData folder, so "no folder" INI files
/// were filed under `other\MMFApplications`. Move them to where they belong (`appdata\`),
/// keeping anything already there.
fn migrate_rootless_appdata(save_root: &Path) {
    let other = save_root.join("other");
    let Some(old) = std::fs::read_dir(&other)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("mmfapplications")))
    else {
        return;
    };
    // Same spelling as `sandboxed` gives (lowercase).
    let new = save_root.join("appdata").join("mmfapplications");
    let _ = std::fs::create_dir_all(&new);
    for e in std::fs::read_dir(&old).into_iter().flatten().flatten() {
        let to = new.join(e.file_name());
        if !to.exists() {
            let _ = std::fs::rename(e.path(), &to);
        }
    }
    let _ = std::fs::remove_dir(&old);
    let _ = std::fs::remove_dir(&other);
}

/// `base` joined with `parts`, following the real spelling of folders and files that exist
/// (save-folder paths are lowercase; Linux file names are case-sensitive).
fn join_existing_case(base: PathBuf, parts: &[String]) -> PathBuf {
    let mut out = base;
    for part in parts {
        let real = std::fs::read_dir(&out)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name())
            .find(|n| n.to_string_lossy().eq_ignore_ascii_case(part));
        out.push(real.unwrap_or_else(|| part.into()));
    }
    out
}

impl SaveFs {
    pub fn new(app_file: &Path, save_root: PathBuf) -> Self {
        let app_file = std::path::absolute(app_file).unwrap_or_else(|_| app_file.to_path_buf());
        let app_dir = app_file.parent().map(Path::to_path_buf).unwrap_or_default();
        // Elsewhere than Windows, games see the AppData folder Proton gives them, so their
        // saves are filed exactly as on Windows (and sync between devices lines up).
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| (!cfg!(windows)).then(|| PathBuf::from(PROTON_APPDATA)));
        if !cfg!(windows) {
            migrate_rootless_appdata(&save_root);
        }
        let virtual_store = if cfg!(windows) { std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("VirtualStore")) } else { None };
        Self {
            app_file,
            app_dir,
            save_root,
            appdata,
            prefix: None,
            virtual_store,
            isolated: false,
            noted: Default::default(),
        }
    }

    /// With the original game's Proton prefix (`.../pfx/drive_c`), for the save manager.
    pub fn with_prefix(mut self, prefix: Option<PathBuf>) -> Self {
        self.prefix = prefix;
        self
    }

    /// "C:" style drive of the application (empty on systems without drives).
    pub fn app_drive(&self) -> String {
        match self.app_dir.components().next() {
            Some(Component::Prefix(p)) => p.as_os_str().to_string_lossy().into_owned(),
            _ => String::new(),
        }
    }

    /// Application directory without the drive, with a trailing separator.
    pub fn app_directory(&self) -> String {
        let full = self.app_path();
        full[self.app_drive().len()..].to_string()
    }

    /// Application directory with drive and trailing separator.
    pub fn app_path(&self) -> String {
        let mut s = self.app_dir.to_string_lossy().into_owned();
        if !s.ends_with(['\\', '/']) {
            s.push(std::path::MAIN_SEPARATOR);
        }
        s
    }

    /// Path inside the save folder that stands in for a game path.
    pub fn sandboxed(&self, path: &str) -> PathBuf {
        let n = norm(path);
        let app = norm(&self.app_dir.to_string_lossy());
        let rel_under = |base: &str| {
            n.strip_prefix(base)
                .map(|r| r.trim_start_matches('\\').to_string())
        };
        let (area, rel) = if let Some(r) = rel_under(&app) {
            ("app", r)
        } else if let Some(r) = self
            .appdata
            .as_ref()
            .and_then(|a| rel_under(&norm(&a.to_string_lossy())))
        {
            ("appdata", r)
        } else if !n.contains(':') && !n.starts_with('\\') {
            ("app", n.clone())
        } else {
            ("other", n.replace(':', ""))
        };
        let mut out = self.save_root.join(area);
        for part in rel
            .split('\\')
            .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        {
            out.push(part);
        }
        out
    }

    /// The real location a game path refers to (for reading original saves).
    fn real(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path.replace('\\', std::path::MAIN_SEPARATOR_STR));
        let p = if p.is_absolute() { p } else { self.app_dir.join(p) };
        // A save the original game wrote to a protected place lives in the VirtualStore.
        match self.virtualized(&p) {
            Some(v) if !p.exists() && v.is_file() => v,
            _ => p,
        }
    }

    /// The VirtualStore copy of a path on the system drive (Windows), if there is a store.
    fn virtualized(&self, p: &Path) -> Option<PathBuf> {
        let store = self.virtual_store.as_ref()?;
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let s = p.to_string_lossy();
        let rest = s.get(..drive.len()).filter(|d| d.eq_ignore_ascii_case(&drive)).map(|_| &s[drive.len()..])?;
        let rest = rest.trim_start_matches(['\\', '/']);
        (!rest.is_empty()).then(|| store.join(rest))
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        if let Ok(b) = std::fs::read(self.sandboxed(path)) {
            return Some(b);
        }
        let b = self.real_allowed(path).then(|| std::fs::read(self.real(path)).ok()).flatten()?;
        if !self.isolated {
            self.note_original(path);
        }
        Some(b)
    }

    /// Read a game data file (a sound, an image) the same way, without counting it as a save.
    pub fn read_asset(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.sandboxed(path))
            .ok()
            .or_else(|| self.real_allowed(path).then(|| std::fs::read(self.real(path)).ok()).flatten())
    }

    /// Remember that the game read an original file (see ORIGINALS_LIST).
    fn note_original(&self, path: &str) {
        let key = norm(&self.real(path).to_string_lossy());
        let Ok(mut set) = self.noted.lock() else { return };
        if !set.insert(key) {
            return;
        }
        let list = self.save_root.join(ORIGINALS_LIST);
        let known = std::fs::read_to_string(&list).unwrap_or_default();
        let real = self.real(path).to_string_lossy().into_owned();
        if known.lines().any(|l| norm(l) == norm(&real)) {
            return;
        }
        let _ = std::fs::create_dir_all(&self.save_root);
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&list) {
            let _ = writeln!(f, "{real}");
        }
    }

    /// The original location of a file inside the save folder (`area/rest` as written by
    /// `sandboxed`), or None for locations that cannot be mapped back.
    pub fn original_of(&self, sandboxed: &Path) -> Option<PathBuf> {
        let rel = sandboxed.strip_prefix(&self.save_root).ok()?;
        let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
        let area = parts.next()?;
        let rest: Vec<String> = parts.collect();
        if rest.is_empty() {
            return None;
        }
        // Linux / Steam Deck: AppData and C:\ are inside the original's Proton prefix.
        if !cfg!(windows) && area != "app" {
            let drive_c = self.prefix.clone()?;
            let base = match area.as_str() {
                "appdata" => join_existing_case(drive_c, &["users", "steamuser", "AppData", "Roaming"].map(String::from)),
                "other" if rest.first().is_some_and(|d| d.eq_ignore_ascii_case("c")) => drive_c,
                _ => return None,
            };
            let skip = usize::from(area == "other");
            return Some(join_existing_case(base, &rest[skip..]));
        }
        let mut out = match area.as_str() {
            "app" => self.app_dir.clone(),
            "appdata" => self.appdata.clone()?,
            // `other/c/dir/file` came from `C:/dir/file`.
            "other" if cfg!(windows) => {
                let drive = rest.first()?;
                if drive.len() != 1 {
                    return None;
                }
                let mut p = PathBuf::from(format!("{}:\\", drive.to_uppercase()));
                for r in &rest[1..] {
                    p.push(r);
                }
                // Files in a drive's root can't be written without admin rights: old games'
                // copies are in the VirtualStore, and that's where an export goes too.
                if let Some(v) = self.virtualized(&p) {
                    if v.is_file() || (!p.exists() && rest.len() == 2) {
                        return Some(v);
                    }
                }
                return Some(p);
            }
            _ => return None,
        };
        for r in &rest {
            out.push(r);
        }
        Some(out)
    }

    /// May the real location of `path` be read? (Not AppData when isolated.)
    fn real_allowed(&self, path: &str) -> bool {
        let real = self.real(path);
        !self.isolated
            || (self.appdata.as_ref().is_none_or(|a| !real.starts_with(a)) && self.virtual_store.as_ref().is_none_or(|v| !real.starts_with(v)))
    }

    pub fn write(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        let target = self.sandboxed(path);
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(target, data)
    }

    pub fn exists(&self, path: &str) -> bool {
        self.sandboxed(path).is_file() || (self.real_allowed(path) && self.real(path).is_file())
    }

    /// A folder exists (in the save folder, or at the real location).
    pub fn dir_exists(&self, path: &str) -> bool {
        self.sandboxed(path).is_dir() || (self.real_allowed(path) && self.real(path).is_dir())
    }

    /// Create a folder (inside the save folder).
    pub fn create_dir(&self, path: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(self.sandboxed(path))
    }

    /// Deletes only the sandboxed copy; original game files are never touched.
    pub fn delete(&self, path: &str) {
        let _ = std::fs::remove_file(self.sandboxed(path));
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn text_files_with_byte_order_marks() {
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("#T03\r\nHey".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode_text(&le), "#T03\r\nHey");
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("ab".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode_text(&be), "ab");
        assert_eq!(decode_text(b"\xEF\xBB\xBFplain"), "plain");
        assert_eq!(decode_text(b"plain"), "plain");
    }
    use super::*;

    /// Linux / Steam Deck: AppData saves are filed as on Windows and map into the original
    /// game's Proton prefix; saves filed the old way move over.
    #[cfg(not(windows))]
    #[test]
    fn proton_saves_map_into_the_prefix() {
        let root = std::env::temp_dir().join(format!("klikcore-proton-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let save = root.join("saves");
        std::fs::create_dir_all(save.join("other").join("mmfapplications")).unwrap();
        std::fs::write(save.join("other").join("mmfapplications").join("old"), b"1").unwrap();
        let drive_c = root.join("pfx").join("drive_c");
        let roaming = drive_c.join("users").join("steamuser").join("AppData").join("Roaming");
        std::fs::create_dir_all(roaming.join("MMFApplications")).unwrap();
        let fs = SaveFs::new(&root.join("game").join("game.exe"), save.clone()).with_prefix(Some(drive_c.clone()));
        // The old location moved.
        assert!(save.join("appdata").join("mmfapplications").join("old").is_file());
        assert!(!save.join("other").exists());
        // An INI in AppData lands in appdata\ and maps to the prefix's Roaming folder.
        let ini = format!(r"{PROTON_APPDATA}\MMFApplications\game1");
        let boxed = fs.sandboxed(&ini);
        assert_eq!(boxed, save.join("appdata").join("mmfapplications").join("game1"));
        // The prefix's own spelling (MMFApplications) is followed.
        assert_eq!(fs.original_of(&boxed), Some(roaming.join("MMFApplications").join("game1")));
        // C:\ files map to drive_c; no prefix, no original location.
        assert_eq!(fs.original_of(&save.join("other").join("c").join("x.sav")), Some(drive_c.join("x.sav")));
        let plain = SaveFs::new(&root.join("game").join("game.exe"), save);
        assert_eq!(plain.original_of(&boxed), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn app_relative_paths_map_into_the_sandbox() {
        // An absolute game path on this platform; games then build their paths from it.
        let (exe, save) = if cfg!(windows) {
            ("C:\\Games\\X\\game.exe", "C:\\Games\\X\\save1.txt")
        } else {
            ("/games/x/game.exe", "/games/x/save1.txt")
        };
        let fs = SaveFs::new(Path::new(exe), PathBuf::from("S"));
        let p = fs.sandboxed(save);
        assert_eq!(p, PathBuf::from("S").join("app").join("save1.txt"));
        assert_eq!(
            fs.sandboxed("saves/a.ini"),
            PathBuf::from("S").join("app").join("saves").join("a.ini")
        );
        assert!(!fs.sandboxed("..\\..\\evil").starts_with(".."));
    }
}
