//! AI upscaling packs: the game's images run once through Real-ESRGAN, saved as an ordinary
//! replacement pack (`mods/<game>/AI upscale/images/`). The game then draws them at its
//! original sizes, at window resolution (see `render`), and collisions keep using the
//! original images.
//!
//! Real-ESRGAN (BSD-3-Clause, github.com/xinntao/Real-ESRGAN) is not part of KlikCore: its
//! portable ncnn/Vulkan build (any GPU with Vulkan, the Steam Deck included) is downloaded
//! into the data folder the first time a pack is made.
//!
//! Styles: drawn art uses the small, fast `realesr-animevideov3` model at the wanted scale;
//! 3D renders and photos use `realesrgan-x4plus` at 4x, scaled down to the wanted size.
//! Transparency: the AI sees colour only (transparent areas filled with their neighbours'
//! colours so edges don't darken); the original alpha is enlarged separately and put back.

use crate::ctf_loader::{GameData, RgbaImage};
use std::path::{Path, PathBuf};

const TOOL_VERSION: &str = "v0.2.5.0";
const TOOL_BUILD: &str = "20220424";
/// Largest side of an upscaled image (GPU texture limit with room to spare).
const MAX_SIDE: u32 = 8192;
/// Images smaller than this (pixels) aren't worth sending through the AI.
const MIN_PIXELS: u32 = 64;
pub const PACK_NAME: &str = "AI upscale";

/// Progress reports: what is happening, and how far through the whole job (0-1) when known.
pub type Progress<'a> = &'a dyn Fn(String, Option<f32>);

/// Share of the job each step takes, for the progress bar: preparing / checking images,
/// the AI run, saving, then retries.
const PREPARE: (f32, f32) = (0.0, 0.08);
const AI_RUN: (f32, f32) = (0.08, 0.85);
const SAVE: (f32, f32) = (0.85, 0.93);
const RETRY: (f32, f32) = (0.93, 1.0);

/// `part` of the way through a step.
fn at(step: (f32, f32), part: f32) -> Option<f32> {
    Some(step.0 + (step.1 - step.0) * part.clamp(0.0, 1.0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Drawn or painted art, cartoons, pixel-ish art.
    Art,
    /// 3D renders and photos.
    Render,
    /// Worked out from the game's images (`guess_style`).
    Auto,
}

impl Style {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "art" | "drawn" => Some(Self::Art),
            "render" | "renders" | "3d" | "photo" => Some(Self::Render),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Art => "drawn art",
            Self::Render => "3D renders",
            Self::Auto => "automatic",
        }
    }
}

/// Drawn art or 3D renders? Renders and photos are full of gradients and grain: neighbouring
/// pixels that differ a little. Drawn and pixel art is mostly flat colour with hard edges.
/// Measured over up to 60 of the game's images (renders score about 0.4-0.6, drawn art
/// 0.02-0.32).
pub fn guess_style(game: &GameData) -> Style {
    let (mut sum, mut n) = (0f32, 0u32);
    let images = &game.images.images;
    let stride = (images.len() / 240).max(1);
    for e in images.iter().step_by(stride) {
        if n >= 60 {
            break;
        }
        if e.width < 48 || e.height < 48 {
            continue;
        }
        let Ok(img) = e.decode_rgba() else { continue };
        if let Some(f) = soft_share(&img) {
            sum += f;
            n += 1;
        }
    }
    if n > 0 && sum / n as f32 > 0.36 {
        Style::Render
    } else {
        Style::Art
    }
}

/// The style `auto` would choose for a game, as text ("drawn art" / "3D renders").
pub fn guess_style_for(game_path: &Path) -> Result<&'static str, String> {
    let game = GameData::load(game_path).map_err(|e| format!("could not load the game: {e}"))?;
    Ok(guess_style(&game).label())
}

/// Share of horizontally neighbouring visible pixels that differ by 1-24 levels (sampled
/// on a grid of at most about 256 x 256). None for images too small or nearly one colour.
fn soft_share(img: &RgbaImage) -> Option<f32> {
    let (w, h) = (img.width as usize, img.height as usize);
    let step = (w.max(h) / 256).max(1);
    let px = |x: usize, y: usize| &img.pixels[(y * w + x) * 4..(y * w + x) * 4 + 4];
    let (mut soft, mut total, mut visible) = (0u32, 0u32, 0u32);
    let mut colours = std::collections::HashSet::new();
    for y in (0..h).step_by(step) {
        let mut x = 0;
        while x + step < w {
            let (a, b) = (px(x, y), px(x + step, y));
            if a[3] > 127 {
                visible += 1;
                if colours.len() < 8 {
                    colours.insert([a[0], a[1], a[2]]);
                }
                if b[3] > 127 {
                    let d = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
                    total += 1;
                    if (1..=24).contains(&d) {
                        soft += 1;
                    }
                }
            }
            x += step;
        }
    }
    (visible >= 500 && colours.len() >= 4 && total > 0).then(|| soft as f32 / total as f32)
}

/// Is AI upscaling available on this system (a Real-ESRGAN build exists for it)?
pub fn supported() -> bool {
    tool_zip().is_some()
}

fn tool_zip() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("ubuntu")
    } else {
        None
    }
}

fn tool_dir() -> PathBuf {
    crate::runtime::files::data_root().join("tools").join("realesrgan")
}

fn tool_exe() -> PathBuf {
    tool_dir().join(if cfg!(windows) { "realesrgan-ncnn-vulkan.exe" } else { "realesrgan-ncnn-vulkan" })
}

/// The pack folder for a game.
pub fn pack_dir(game: &Path) -> PathBuf {
    crate::mods::Mods::dir_for(game).join(PACK_NAME)
}

pub fn has_pack(game: &Path) -> bool {
    pack_dir(game).join("images").is_dir()
}

pub fn remove_pack(game: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(pack_dir(game))
}

/// The scale that brings a game of this height to about 1080 lines (2x-4x).
pub fn auto_scale(height: u32) -> u32 {
    (1080f32 / height.max(1) as f32).ceil().clamp(2.0, 4.0) as u32
}

/// Download and unpack Real-ESRGAN if it isn't there yet.
fn ensure_tool(progress: Progress) -> Result<PathBuf, String> {
    let exe = tool_exe();
    if exe.is_file() {
        return Ok(exe);
    }
    let os = tool_zip().ok_or("AI upscaling isn't available on this system")?;
    let dir = tool_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join("download.zip");
    let url = format!("https://github.com/xinntao/Real-ESRGAN/releases/download/{TOOL_VERSION}/realesrgan-ncnn-vulkan-{TOOL_BUILD}-{os}.zip");
    progress("Downloading Real-ESRGAN (about 45 MB, once)...".into(), None);
    super::update::download(&url, &zip)?;
    progress("Unpacking Real-ESRGAN...".into(), None);
    let bytes = std::fs::read(&zip).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&zip);
    // Only the program, its libraries and the models (not the demo pictures and video).
    let wanted = |name: &str| {
        let n = name.to_ascii_lowercase();
        !n.ends_with('/') && (n.starts_with("models/") || n.ends_with(".exe") || n.ends_with(".dll") || n == "realesrgan-ncnn-vulkan" || n.ends_with(".so"))
    };
    for (name, data) in unzip(&bytes, &wanted)? {
        let out = dir.join(&name);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&out, data).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }
    if exe.is_file() {
        Ok(exe)
    } else {
        Err("the Real-ESRGAN download didn't contain the program".into())
    }
}

/// Files from a zip archive (stored or deflated), chosen by name.
fn unzip(b: &[u8], wanted: &dyn Fn(&str) -> bool) -> Result<Vec<(String, Vec<u8>)>, String> {
    use std::io::Read;
    let u16_at = |p: usize| b.get(p..p + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize);
    let u32_at = |p: usize| b.get(p..p + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
    // End of central directory: the last "PK\x05\x06".
    let eocd = (0..b.len().saturating_sub(21)).rev().find(|&p| b[p..].starts_with(b"PK\x05\x06")).ok_or("not a zip file")?;
    let count = u16_at(eocd + 10).ok_or("bad zip")?;
    let mut p = u32_at(eocd + 16).ok_or("bad zip")?;
    let mut out = Vec::new();
    for _ in 0..count {
        if !b.get(p..).is_some_and(|s| s.starts_with(b"PK\x01\x02")) {
            return Err("bad zip directory".into());
        }
        let method = u16_at(p + 10).ok_or("bad zip")?;
        let csize = u32_at(p + 20).ok_or("bad zip")?;
        let usize_ = u32_at(p + 24).ok_or("bad zip")?;
        let (nlen, xlen, clen) = (u16_at(p + 28).ok_or("bad zip")?, u16_at(p + 30).ok_or("bad zip")?, u16_at(p + 32).ok_or("bad zip")?);
        let local = u32_at(p + 42).ok_or("bad zip")?;
        let name = String::from_utf8_lossy(b.get(p + 46..p + 46 + nlen).ok_or("bad zip")?).replace('\\', "/");
        p += 46 + nlen + xlen + clen;
        // No absolute paths or ".." in names (the files go into our own folder).
        if !wanted(&name) || name.starts_with('/') || name.split('/').any(|c| c == "..") {
            continue;
        }
        let start = local + 30 + u16_at(local + 26).ok_or("bad zip")? + u16_at(local + 28).ok_or("bad zip")?;
        let data = b.get(start..start + csize).ok_or("bad zip entry")?;
        let bytes = match method {
            0 => data.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(data).read_to_end(&mut v).map_err(|e| e.to_string())?;
                v
            }
            m => return Err(format!("unsupported zip compression {m}")),
        };
        out.push((name, bytes));
    }
    Ok(out)
}

/// Fill fully transparent pixels with the colours of their opaque neighbours (a few rings
/// deep, then the image's average), so the AI doesn't draw dark fringes along edges.
fn bleed(img: &RgbaImage) -> Vec<[u8; 3]> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut rgb: Vec<[u8; 3]> = img.pixels.chunks_exact(4).map(|p| [p[0], p[1], p[2]]).collect();
    let mut known: Vec<bool> = img.pixels.chunks_exact(4).map(|p| p[3] > 0).collect();
    for _ in 0..16 {
        let mut next = known.clone();
        let mut changed = false;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if known[i] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if known[j] {
                        for c in 0..3 {
                            sum[c] += rgb[j][c] as u32;
                        }
                        n += 1;
                    }
                }
                if n > 0 {
                    rgb[i] = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8];
                    next[i] = true;
                    changed = true;
                }
            }
        }
        known = next;
        if !changed {
            break;
        }
    }
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for (c, k) in rgb.iter().zip(&known) {
        if *k {
            for i in 0..3 {
                sum[i] += c[i] as u64;
            }
            n += 1;
        }
    }
    let avg = if n > 0 { [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8] } else { [0; 3] };
    for (c, k) in rgb.iter_mut().zip(&known) {
        if !*k {
            *c = avg;
        }
    }
    rgb
}

/// Resize one channel-interleaved image (`ch` channels) with bilinear filtering, averaging
/// whole source blocks first when shrinking by a whole factor (sharper than bilinear).
fn resize(px: &[u8], w: usize, h: usize, ch: usize, nw: usize, nh: usize) -> Vec<u8> {
    if (w, h) == (nw, nh) {
        return px.to_vec();
    }
    if nw < w && w % nw == 0 && h % nh == 0 && w / nw == h / nh {
        let f = w / nw;
        let mut out = vec![0u8; nw * nh * ch];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..ch {
                    let mut s = 0u32;
                    for yy in 0..f {
                        for xx in 0..f {
                            s += px[((y * f + yy) * w + x * f + xx) * ch + c] as u32;
                        }
                    }
                    out[(y * nw + x) * ch + c] = (s / (f * f) as u32) as u8;
                }
            }
        }
        return out;
    }
    let mut out = vec![0u8; nw * nh * ch];
    let (sx, sy) = (w as f32 / nw as f32, h as f32 / nh as f32);
    for y in 0..nh {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, ty) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..nw {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, tx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            for c in 0..ch {
                let p = |xx: usize, yy: usize| px[(yy * w + xx) * ch + c] as f32;
                let top = p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx;
                let bot = p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx;
                out[(y * nw + x) * ch + c] = (top * (1.0 - ty) + bot * ty).round() as u8;
            }
        }
    }
    out
}

fn write_png(path: &Path, w: u32, h: u32, color: png::ColorType, px: &[u8]) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(color);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.write_header().and_then(|mut wr| wr.write_image_data(px)).map_err(|e| e.to_string())
}

fn read_rgb(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let ch = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        _ => return Err("unexpected PNG colour type".into()),
    };
    let n = (info.width * info.height) as usize;
    let rgb: Vec<u8> = (0..n)
        .flat_map(|i| {
            let p = &buf[i * ch..i * ch + ch];
            if ch >= 3 {
                [p[0], p[1], p[2]]
            } else {
                [p[0], p[0], p[0]]
            }
        })
        .collect();
    Ok((info.width, info.height, rgb))
}

/// Images per Real-ESRGAN run. When the GPU driver drops a run ("device lost"), everything
/// after that point in the run comes out black, so short runs lose little.
const BATCH: usize = 150;

/// How a pack was made (`pack.ini` in the pack folder), for repairs.
fn write_pack_info(pack: &Path, style: Style, scale: u32) {
    let style = match style {
        Style::Render => "render",
        _ => "art",
    };
    let _ = std::fs::write(pack.join("pack.ini"), format!("[ai-upscale]\r\nstyle={style}\r\nscale={scale}\r\n"));
}

/// The style and scale an existing pack was made with: from `pack.ini`, the README older
/// versions wrote, or (a pack that was stopped early) the size of its images.
fn read_pack_info(game: &GameData, pack: &Path) -> (Style, u32) {
    let text = std::fs::read_to_string(pack.join("pack.ini")).or_else(|_| std::fs::read_to_string(pack.join("README.txt"))).unwrap_or_default();
    let mut style = if text.contains("style=render") || text.contains("3D renders style") { Style::Render } else { Style::Art };
    if text.contains("style=art") {
        style = Style::Art;
    }
    let scale = text
        .lines()
        .find_map(|l| l.strip_prefix("scale=").and_then(|v| v.trim().parse::<u32>().ok()))
        .or_else(|| {
            // "(realesrgan-x4plus, 3D renders style, 3x)."
            let i = text.find("x).")?;
            text[..i].rsplit(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
        })
        .or_else(|| {
            let images = pack.join("images");
            game.images.images.iter().find_map(|e| {
                let (w, _) = crate::mods::png_size(&images.join(format!("{}.png", e.handle)))?;
                (e.width >= 16).then(|| (w as f32 / e.width as f32).round() as u32)
            })
        })
        .unwrap_or(2)
        .clamp(2, 4);
    (style, scale)
}

/// Make (or remake) a game's AI upscale pack. `scale` 0 = automatic. Returns a summary.
pub fn make_pack(game_path: &Path, style: Style, scale: u32, progress: Progress) -> Result<String, String> {
    let exe = ensure_tool(progress)?;
    progress("Reading the game...".into(), Some(0.0));
    let game = GameData::load(game_path).map_err(|e| format!("could not load the game: {e}"))?;
    let scale = if scale == 0 { auto_scale(game.header.window_height as u32) } else { scale.clamp(2, 4) };
    let style = if style == Style::Auto { guess_style(&game) } else { style };
    with_work_dir(|inp, outp| run(&game, &exe, style, scale, inp, outp, game_path, false, progress))
}

/// Check an existing pack against the game's images and redo only the ones that are
/// missing or came out wrong (black, wrong colours, detail wiped out). Returns a summary.
pub fn repair_pack(game_path: &Path, progress: Progress) -> Result<String, String> {
    let exe = ensure_tool(progress)?;
    progress("Reading the game...".into(), Some(0.0));
    let game = GameData::load(game_path).map_err(|e| format!("could not load the game: {e}"))?;
    let pack = pack_dir(game_path);
    if !pack.join("images").is_dir() {
        return Err("the game has no AI upscale pack".into());
    }
    let (style, scale) = read_pack_info(&game, &pack);
    with_work_dir(|inp, outp| run(&game, &exe, style, scale, inp, outp, game_path, true, progress))
}

fn with_work_dir(f: impl FnOnce(&Path, &Path) -> Result<String, String>) -> Result<String, String> {
    let work = crate::runtime::files::data_root().join("tmp").join("upscale");
    let (inp, outp) = (work.join("in"), work.join("out"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&inp).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&outp).map_err(|e| e.to_string())?;
    let result = f(&inp, &outp);
    // KLIKCORE_UPSCALE_KEEP=1 keeps the working files (debugging).
    if std::env::var_os("KLIKCORE_UPSCALE_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result
}

/// Does an AI result, shrunk back to the original's size (`small`, RGB), still look like
/// the original? Catches black or grey outputs (also of dark images), wrong colours, and
/// detail the model wiped out (stars, text, thin lines).
fn looks_right(orig: &RgbaImage, small: &[u8]) -> bool {
    let (w, h) = (orig.width as usize, orig.height as usize);
    let vis = |i: usize| orig.pixels[i * 4 + 3] > 127;
    let luma = |p: &[u8]| p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114;
    let (mut diff, mut lo, mut ls, mut n) = (0f32, 0f32, 0f32, 0f32);
    let (mut eo, mut es, mut ne) = (0f32, 0f32, 0f32);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !vis(i) {
                continue;
            }
            let (o, s) = (&orig.pixels[i * 4..i * 4 + 3], &small[i * 3..i * 3 + 3]);
            diff += (o[0].abs_diff(s[0]) as f32 * 0.299) + (o[1].abs_diff(s[1]) as f32 * 0.587) + (o[2].abs_diff(s[2]) as f32 * 0.114);
            lo += luma(o);
            ls += luma(s);
            n += 1.0;
            // Local contrast against the right and lower neighbours.
            for j in [(x + 1 < w).then_some(i + 1), (y + 1 < h).then_some(i + w)].into_iter().flatten() {
                if vis(j) {
                    eo += (luma(o) - luma(&orig.pixels[j * 4..j * 4 + 3])).abs();
                    es += (luma(s) - luma(&small[j * 3..j * 3 + 3])).abs();
                    ne += 1.0;
                }
            }
        }
    }
    if n == 0.0 {
        return true;
    }
    let (lo, ls) = (lo / n, ls / n);
    // Much darker than the original (a dropped GPU run comes out black).
    if lo > 6.0 && ls < lo * 0.6 {
        return false;
    }
    if diff / n > 24.0 {
        return false;
    }
    // Detail: a faithful enlargement keeps most of the original's local contrast when
    // shrunk back.
    if ne > 0.0 && eo / ne > 6.0 && es < eo * 0.4 {
        return false;
    }
    true
}

/// A pack image (or AI output) shrunk to the original's size, as RGB.
fn shrunk(path: &Path, orig: &RgbaImage) -> Option<Vec<u8>> {
    let (aw, ah, rgb) = read_rgb(path).ok()?;
    Some(resize(&rgb, aw as usize, ah as usize, 3, orig.width as usize, orig.height as usize))
}

#[allow(clippy::too_many_arguments)]
fn run(game: &GameData, exe: &Path, style: Style, scale: u32, inp: &Path, outp: &Path, game_path: &Path, repair: bool, progress: Progress) -> Result<String, String> {
    let pack = pack_dir(game_path);
    let images = pack.join("images");
    if !repair {
        let _ = std::fs::remove_dir_all(&images);
    }
    std::fs::create_dir_all(&images).map_err(|e| e.to_string())?;

    // 1. The game's images as colour-only PNGs (in a repair, only those whose pack image is
    // missing or wrong). Single-colour shapes (text, font glyphs, masks) have nothing for
    // the AI to add, and on a flat colour the models produce garbage: enlarged directly.
    let total = game.images.images.len();
    let mut sent = Vec::new();
    let (mut written, mut kept) = (0, 0);
    for (k, e) in game.images.images.iter().enumerate() {
        if k % 25 == 0 {
            progress(format!("{} images {}/{total}...", if repair { "Checking" } else { "Preparing" }, k + 1), at(PREPARE, k as f32 / total.max(1) as f32));
        }
        let Ok(img) = e.decode_rgba() else { continue };
        let target = images.join(format!("{}.png", e.handle));
        if img.width * img.height < MIN_PIXELS {
            continue;
        }
        if let Some(c) = flat_colour(&img) {
            let (nw, nh) = target_size(img.width, img.height, scale);
            let rgb: Vec<u8> = std::iter::repeat_n(c, (nw * nh) as usize).flatten().collect();
            write_png(&target, nw, nh, png::ColorType::Rgba, &with_alpha(&img, &rgb, nw, nh))?;
            written += 1;
            continue;
        }
        if repair {
            if shrunk(&target, &img).is_some_and(|s| looks_right(&img, &s)) {
                kept += 1;
                continue;
            }
            // Wrong or missing: the original is used until a good replacement exists.
            let _ = std::fs::remove_file(&target);
        }
        let rgb: Vec<u8> = bleed(&img).into_iter().flatten().collect();
        write_png(&inp.join(format!("{}.png", e.handle)), img.width, img.height, png::ColorType::Rgb, &rgb)?;
        sent.push(e.handle);
    }
    if sent.is_empty() && written == 0 && kept == 0 {
        return Err("the game has no images to upscale".into());
    }

    // 2. Real-ESRGAN, a batch at a time.
    let (model, model_scale) = match style {
        Style::Render => ("realesrgan-x4plus", 4),
        _ => ("realesr-animevideov3", scale),
    };
    let label = format!("{}, {scale}x", style.label());
    let staged = inp.with_file_name("batch");
    for batch in sent.chunks(BATCH) {
        stage(inp, &staged, batch)?;
        // A failed run leaves its images missing; they are retried below.
        if let Err(e) = ai_pass(exe, model, model_scale, &staged, outp, &[], sent.len(), &label, AI_RUN, progress) {
            if !outp.read_dir().is_ok_and(|mut d| d.next().is_some()) {
                return Err(e);
            }
        }
    }

    // 3. Back to the wanted size, with the original transparency. Results that don't look
    // like the original are run again in small batches with gentler GPU settings, then
    // (3D renders) with the drawn-art model; any still wrong keep their original image.
    let mut todo = sent.clone();
    let mut fixed_later = 0;
    for attempt in 0..3 {
        let mut failed = Vec::new();
        for (k, &h) in todo.iter().enumerate() {
            if k % 10 == 0 {
                let f = k as f32 / todo.len().max(1) as f32;
                let frac = if attempt == 0 { at(SAVE, f) } else { at(RETRY, (attempt as f32 - 1.0 + f) / 2.0) };
                progress(format!("Checking and saving {}/{}...", k + 1, todo.len()), frac);
            }
            match save_one(game, h, scale, outp, &images)? {
                true => {
                    written += 1;
                    if attempt > 0 {
                        fixed_later += 1;
                    }
                }
                false => failed.push(h),
            }
        }
        todo = failed;
        if todo.is_empty() || attempt == 2 {
            break;
        }
        let (m, ms, what) = if attempt == 1 && style == Style::Render { ("realesr-animevideov3", scale, "other model") } else { (model, model_scale, "again") };
        for (b, batch) in todo.chunks(25).enumerate() {
            for h in batch {
                let _ = std::fs::remove_file(outp.join(format!("{h}.png")));
            }
            stage(inp, &staged, batch)?;
            let note = format!("{label}, {what}: {}/{}", (b * 25 + batch.len()).min(todo.len()), todo.len());
            // Errors here just leave those images failed.
            let part = (attempt as f32 + b as f32 / todo.len().div_ceil(25).max(1) as f32) / 2.0;
            let step = (at(RETRY, part).unwrap_or(RETRY.0), RETRY.1);
            let _ = ai_pass(exe, m, ms, &staged, outp, &["-t", "128", "-j", "1:1:1"], 0, &note, step, progress);
        }
    }
    let skipped = todo.len();
    let readme = format!(
        "Made by KlikCore with Real-ESRGAN ({model}, {} style, {scale}x).\r\nDelete this folder (or use \"Remove AI upscale pack\" in the launcher) to go back to the original images.\r\n",
        style.label()
    );
    let _ = std::fs::write(pack.join("README.txt"), readme);
    write_pack_info(&pack, style, scale);
    let note = if skipped > 0 { format!("; {skipped} kept their original image (the AI got them wrong)") } else { String::new() };
    if repair {
        Ok(format!("AI upscale pack repaired: {kept} images were fine, {written} redone ({fixed_later} on a second try){note}."))
    } else {
        Ok(format!("AI upscale pack made: {written} images at {scale}x ({}){note}.", style.label()))
    }
}

/// Copy some of the prepared images into an empty folder for one Real-ESRGAN run.
fn stage(inp: &Path, staged: &Path, handles: &[u32]) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(staged);
    std::fs::create_dir_all(staged).map_err(|e| e.to_string())?;
    for h in handles {
        let name = format!("{h}.png");
        std::fs::copy(inp.join(&name), staged.join(&name)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One Real-ESRGAN run over a folder. `count` > 0 shows "done/count" progress through
/// `step` of the job.
#[allow(clippy::too_many_arguments)]
fn ai_pass(exe: &Path, model: &str, model_scale: u32, inp: &Path, outp: &Path, extra: &[&str], count: usize, label: &str, step: (f32, f32), progress: Progress) -> Result<(), String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("-i").arg(inp).arg("-o").arg(outp).args(["-n", model, "-s", &model_scale.to_string(), "-f", "png", "-m"]).arg(tool_dir().join("models")).args(extra);
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start Real-ESRGAN: {e}"))?;
    // Keep its messages (for errors) without letting the pipe fill up.
    let stderr = child.stderr.take();
    let log = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = stderr {
            use std::io::Read;
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if count > 0 {
            let done = std::fs::read_dir(outp).map(|d| d.count()).unwrap_or(0);
            progress(format!("Upscaling with AI {done}/{count} ({label})..."), at(step, done as f32 / count as f32));
        } else {
            progress(format!("Upscaling with AI ({label})..."), Some(step.0));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    let messages = log.join().unwrap_or_default();
    if !status.success() {
        let hint = if messages.to_lowercase().contains("vulkan") { " (it needs a graphics card with Vulkan)" } else { "" };
        let last: String = messages.lines().filter(|l| !l.trim().ends_with('%')).last().unwrap_or("").to_string();
        return Err(format!("Real-ESRGAN failed{hint}: {last}"));
    }
    Ok(())
}

/// Turn one AI output into the pack image. Ok(false) when the output is missing or doesn't
/// look like the original (see `looks_right`).
fn save_one(game: &GameData, h: u32, scale: u32, outp: &Path, images: &Path) -> Result<bool, String> {
    let Ok((aw, ah, rgb)) = read_rgb(&outp.join(format!("{h}.png"))) else { return Ok(false) };
    let Some(orig) = game.images.get(h).and_then(|e| e.decode_rgba().ok()) else { return Ok(true) };
    let small = resize(&rgb, aw as usize, ah as usize, 3, orig.width as usize, orig.height as usize);
    if !looks_right(&orig, &small) {
        return Ok(false);
    }
    let (nw, nh) = target_size(orig.width, orig.height, scale);
    let rgb = resize(&rgb, aw as usize, ah as usize, 3, nw as usize, nh as usize);
    write_png(&images.join(format!("{h}.png")), nw, nh, png::ColorType::Rgba, &with_alpha(&orig, &rgb, nw, nh))?;
    Ok(true)
}

/// The pack size of an image: `scale` times larger, kept within the texture limit.
fn target_size(w: u32, h: u32, scale: u32) -> (u32, u32) {
    let fit = (MAX_SIDE as f32 / (w.max(h) * scale) as f32).min(1.0);
    (((w * scale) as f32 * fit).round().max(1.0) as u32, ((h * scale) as f32 * fit).round().max(1.0) as u32)
}

/// Colour at the pack size plus the original's transparency, enlarged.
fn with_alpha(orig: &RgbaImage, rgb: &[u8], nw: u32, nh: u32) -> Vec<u8> {
    let alpha: Vec<u8> = orig.pixels.chunks_exact(4).map(|p| p[3]).collect();
    let mut rgba = Vec::with_capacity((nw * nh * 4) as usize);
    if alpha.iter().all(|&a| a == 255) {
        for p in rgb.chunks_exact(3) {
            rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        return rgba;
    }
    let hard = alpha.iter().all(|&a| a == 0 || a == 255);
    let a = resize(&alpha, orig.width as usize, orig.height as usize, 1, nw as usize, nh as usize);
    for (p, &a) in rgb.chunks_exact(3).zip(&a) {
        // Cut-out sprites keep crisp edges (smoothed a little, not blurred).
        let a = if hard { (((a as f32 - 128.0) * 2.0) + 128.0).clamp(0.0, 255.0) as u8 } else { a };
        rgba.extend_from_slice(&[p[0], p[1], p[2], a]);
    }
    rgba
}

/// The colour of an image whose visible pixels are (nearly) all one colour.
fn flat_colour(img: &RgbaImage) -> Option<[u8; 3]> {
    let (mut lo, mut hi, mut sum, mut n) = ([255u8; 3], [0u8; 3], [0u64; 3], 0u64);
    for p in img.pixels.chunks_exact(4).filter(|p| p[3] > 0) {
        for c in 0..3 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
            sum[c] += p[c] as u64;
        }
        n += 1;
    }
    (n > 0 && (0..3).all(|c| hi[c] - lo[c] <= 12)).then(|| [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_colour_images_are_found() {
        let img = RgbaImage { width: 2, height: 1, pixels: vec![250, 250, 250, 255, 0, 0, 0, 0] };
        assert_eq!(flat_colour(&img), Some([250, 250, 250]));
        let img = RgbaImage { width: 2, height: 1, pixels: vec![250, 250, 250, 255, 10, 10, 10, 255] };
        assert_eq!(flat_colour(&img), None);
    }

    #[test]
    fn wrong_ai_results_are_caught() {
        // A dark 8x8 checkerboard (low average brightness, lots of detail).
        let mut px = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let v = if (x + y) % 2 == 0 { 90 } else { 10 };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let orig = RgbaImage { width: 8, height: 8, pixels: px.clone() };
        let same: Vec<u8> = px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        assert!(looks_right(&orig, &same));
        // Black (a dropped GPU run): only about 50 off on average, but much darker.
        assert!(!looks_right(&orig, &vec![0; 8 * 8 * 3]));
        // Detail wiped out: the right average colour, no checkerboard.
        assert!(!looks_right(&orig, &vec![50; 8 * 8 * 3]));
    }

    #[test]
    fn renders_and_drawn_art_are_told_apart() {
        // A smooth gradient (a render) and four flat colour bands (drawn art), 64x64.
        let mut grad = Vec::new();
        let mut flat = Vec::new();
        for _y in 0..64u32 {
            for x in 0..64u32 {
                let g = (x * 3 + 20) as u8;
                grad.extend_from_slice(&[g, g / 2, 255 - g, 255]);
                let band = [[200, 40, 40], [40, 200, 40], [40, 40, 200], [220, 220, 30]][(x / 16) as usize];
                flat.extend_from_slice(&[band[0], band[1], band[2], 255]);
            }
        }
        let render = soft_share(&RgbaImage { width: 64, height: 64, pixels: grad }).unwrap();
        let art = soft_share(&RgbaImage { width: 64, height: 64, pixels: flat }).unwrap();
        assert!(render > 0.36 && art < 0.36, "{render} {art}");
    }

    #[test]
    fn scale_targets_1080() {
        assert_eq!(auto_scale(720), 2);
        assert_eq!(auto_scale(480), 3);
        assert_eq!(auto_scale(240), 4);
        assert_eq!(auto_scale(1080), 2);
    }

    #[test]
    fn bleed_fills_transparent_pixels_from_neighbours() {
        // 3x1: red, transparent, transparent.
        let img = RgbaImage { width: 3, height: 1, pixels: vec![255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0] };
        let rgb = bleed(&img);
        assert_eq!(rgb, vec![[255, 0, 0]; 3]);
    }

    #[test]
    fn resize_box_and_bilinear() {
        // 2x2 -> 1x1 averages.
        assert_eq!(resize(&[0, 100, 200, 100], 2, 2, 1, 1, 1), vec![100]);
        // 1x1 -> 2x2 repeats.
        assert_eq!(resize(&[7], 1, 1, 1, 2, 2), vec![7; 4]);
    }

    #[test]
    fn unzip_reads_stored_and_deflated() {
        use std::io::Write;
        // A hand-built zip with one deflated file "models/a.txt" and one skipped file.
        let mut deflated = Vec::new();
        let mut enc = flate2::write::DeflateEncoder::new(&mut deflated, flate2::Compression::default());
        enc.write_all(b"hello hello hello").unwrap();
        enc.finish().unwrap();
        let mut zip = Vec::new();
        let mut central = Vec::new();
        let entries: [(&str, u16, &[u8], usize); 2] = [("models/a.txt", 8, &deflated, 17), ("demo.mp4", 0, b"xx", 2)];
        for (name, method, data, size) in entries {
            let off = zip.len() as u32;
            zip.extend_from_slice(b"PK\x03\x04");
            zip.extend_from_slice(&[20, 0, 0, 0]);
            zip.extend_from_slice(&method.to_le_bytes());
            zip.extend_from_slice(&[0; 8]);
            zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(size as u32).to_le_bytes());
            zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
            zip.extend_from_slice(&[0, 0]);
            zip.extend_from_slice(name.as_bytes());
            zip.extend_from_slice(data);
            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(size as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&off.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd = zip.len() as u32;
        zip.extend_from_slice(&central);
        zip.extend_from_slice(b"PK\x05\x06\0\0\0\0\x02\0\x02\0");
        zip.extend_from_slice(&(central.len() as u32).to_le_bytes());
        zip.extend_from_slice(&cd.to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        let files = unzip(&zip, &|n| n.starts_with("models/")).unwrap();
        assert_eq!(files, vec![("models/a.txt".to_string(), b"hello hello hello".to_vec())]);
    }
}
