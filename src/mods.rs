//! Replacement packs: files that override a game's images and sounds without touching the
//! game itself.
//!
//! Layout: `<data dir>\KlikCore\mods\<game file name>\` holding `images\` / `sounds\` folders,
//! either directly or inside any number of pack folders (applied in alphabetical order, so
//! later packs win). A file named `123.png` (or `123_anything.png`, as `--extract` names
//! them) replaces image 123; `45.wav` / `.ogg` / `.mp3` replaces sound 45.
//!
//! Replacement images are drawn at the original image's size (so a double-resolution image
//! looks sharper but lines up exactly); collisions keep using the original image.

use crate::ctf_loader::RgbaImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct Mods {
    pub images: HashMap<u32, PathBuf>,
    pub sounds: HashMap<u32, PathBuf>,
    /// Folders that contributed files, in the order they were applied.
    pub packs: Vec<String>,
}

const README: &str = "KlikCore replacement packs for this game.\r\n\r\n\
Put files in an \"images\" or \"sounds\" folder here, or inside a pack folder of your choice\r\n\
(e.g. \"HD textures\\images\\\"). Packs are applied in alphabetical order; later ones win.\r\n\r\n\
  images\\123.png        replaces image 123\r\n\
  sounds\\45.wav         replaces sound 45 (.ogg and .mp3 work too)\r\n\r\n\
To find the numbers, export the game's files with:  klikcore \"<game file>\" --extract <folder>\r\n\
Exported names like 123_name.png can be dropped back in as they are.\r\n\
Replacement images are drawn at the original size, so larger images simply look sharper.\r\n";

impl Mods {
    /// The mods folder for a game file (one per game file name).
    pub fn dir_for(game: &Path) -> PathBuf {
        let stem = game.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "game".into());
        let safe: String = stem.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
        // Testing: KLIKCORE_MODS_DIR replaces the mods folder.
        let base = std::env::var_os("KLIKCORE_MODS_DIR").map(PathBuf::from).unwrap_or_else(|| crate::runtime::files::data_root().join("mods"));
        base.join(safe.trim())
    }

    /// Create the folder (with a short how-to) if it doesn't exist yet.
    pub fn ensure_dir(game: &Path) -> std::io::Result<PathBuf> {
        let dir = Self::dir_for(game);
        std::fs::create_dir_all(dir.join("images"))?;
        std::fs::create_dir_all(dir.join("sounds"))?;
        let readme = dir.join("README.txt");
        if !readme.exists() {
            std::fs::write(readme, README)?;
        }
        Ok(dir)
    }

    pub fn load(game: &Path) -> Self {
        Self::load_dir(&Self::dir_for(game))
    }

    /// Load the packs in a mods folder.
    pub fn load_dir(dir: &Path) -> Self {
        let dir = dir.to_path_buf();
        let mut m = Self::default();
        if !dir.is_dir() {
            return m;
        }
        m.scan(&dir, "(base)");
        let mut packs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !is_asset_dir(p))
            .collect();
        packs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        for p in packs {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            m.scan(&p, &name);
        }
        m
    }

    fn scan(&mut self, dir: &Path, name: &str) {
        let mut found = 0;
        for (sub, exts, map) in [
            ("images", &["png"][..], &mut self.images),
            ("sounds", &["wav", "ogg", "mp3"][..], &mut self.sounds),
        ] {
            for e in std::fs::read_dir(dir.join(sub)).into_iter().flatten().flatten() {
                let p = e.path();
                let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                if !exts.contains(&ext.as_str()) {
                    continue;
                }
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let digits: String = stem.chars().take_while(char::is_ascii_digit).collect();
                if let Ok(h) = digits.parse::<u32>() {
                    map.insert(h, p);
                    found += 1;
                }
            }
        }
        if found > 0 {
            self.packs.push(name.to_string());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.sounds.is_empty()
    }

    /// The replacement for image `handle`, decoded to RGBA.
    pub fn image(&self, handle: u32) -> Option<RgbaImage> {
        decode_png(self.images.get(&handle)?).map_err(|e| eprintln!("mod image {handle}: {e}")).ok()
    }

    /// How much larger the replacement images are than the originals (the largest ratio
    /// found among a sample; 1.0 = same size or no images). Reads only PNG headers.
    /// `original` gives an image's original size.
    pub fn image_scale(&self, original: impl Fn(u32) -> Option<(u32, u32)>) -> f32 {
        let mut best = 1.0f32;
        // A sample is enough: packs are made at one scale.
        for (&h, path) in self.images.iter().take(64) {
            let (Some((w, _)), Some((ow, _))) = (png_size(path), original(h)) else { continue };
            if ow > 0 {
                best = best.max(w as f32 / ow as f32);
            }
        }
        best
    }

    /// The replacement file for sound `handle`.
    pub fn sound(&self, handle: u32) -> Option<Vec<u8>> {
        std::fs::read(self.sounds.get(&handle)?).ok()
    }
}

fn is_asset_dir(p: &Path) -> bool {
    p.file_name().is_some_and(|n| {
        let n = n.to_string_lossy().to_lowercase();
        n == "images" || n == "sounds"
    })
}

/// Width and height from a PNG's header.
pub fn png_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut head = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[1..4] != b"PNG" || &head[12..16] != b"IHDR" {
        return None;
    }
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    Some((be(&head[16..20]), be(&head[20..24])))
}

fn decode_png(path: &Path) -> Result<RgbaImage, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width, info.height);
    let px = (w * h) as usize;
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2].chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => return Err(format!("unsupported PNG colour type {other:?}")),
    };
    Ok(RgbaImage { width: w, height: h, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_packs_win_and_extracted_names_work() {
        let dir = std::env::temp_dir().join(format!("klikcore-mods-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (pack, file) in [("", "images/7.png"), ("a pack", "images/7_title.png"), ("b pack", "sounds/3_click.wav")] {
            let p = if pack.is_empty() { dir.join(file) } else { dir.join(pack).join(file) };
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        let m = Mods::load_dir(&dir);
        assert!(m.images[&7].starts_with(dir.join("a pack")), "a later pack overrides the base folder");
        assert!(m.sounds.contains_key(&3));
        assert_eq!(m.packs, vec!["(base)", "a pack", "b pack"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
