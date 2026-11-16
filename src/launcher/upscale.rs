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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Drawn or painted art, cartoons, pixel-ish art.
    Art,
    /// 3D renders and photos.
    Render,
}

impl Style {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "art" | "drawn" => Some(Self::Art),
            "render" | "renders" | "3d" | "photo" => Some(Self::Render),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Art => "drawn art",
            Self::Render => "3D renders",
        }
    }
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
fn ensure_tool(progress: &dyn Fn(String)) -> Result<PathBuf, String> {
    let exe = tool_exe();
    if exe.is_file() {
        return Ok(exe);
    }
    let os = tool_zip().ok_or("AI upscaling isn't available on this system")?;
    let dir = tool_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join("download.zip");
    let url = format!("https://github.com/xinntao/Real-ESRGAN/releases/download/{TOOL_VERSION}/realesrgan-ncnn-vulkan-{TOOL_BUILD}-{os}.zip");
    progress("Downloading Real-ESRGAN (about 45 MB, once)...".into());
    super::update::download(&url, &zip)?;
    progress("Unpacking Real-ESRGAN...".into());
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

/// Make (or remake) a game's AI upscale pack. `scale` 0 = automatic. Returns a summary.
pub fn make_pack(game_path: &Path, style: Style, scale: u32, progress: &dyn Fn(String)) -> Result<String, String> {
    let exe = ensure_tool(progress)?;
    progress("Reading the game...".into());
    let game = GameData::load(game_path).map_err(|e| format!("could not load the game: {e}"))?;
    let scale = if scale == 0 { auto_scale(game.header.window_height as u32) } else { scale.clamp(2, 4) };
    let work = crate::runtime::files::data_root().join("tmp").join("upscale");
    let (inp, outp) = (work.join("in"), work.join("out"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&inp).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&outp).map_err(|e| e.to_string())?;
    let result = run(&game, &exe, style, scale, &inp, &outp, game_path, progress);
    // KLIKCORE_UPSCALE_KEEP=1 keeps the working files (debugging).
    if std::env::var_os("KLIKCORE_UPSCALE_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run(game: &GameData, exe: &Path, style: Style, scale: u32, inp: &Path, outp: &Path, game_path: &Path, progress: &dyn Fn(String)) -> Result<String, String> {
    // 1. The game's images as colour-only PNGs.
    let total = game.images.images.len();
    let mut sent = Vec::new();
    // Single-colour shapes (text, font glyphs, masks): nothing for the AI to add, and on a
    // flat colour the models produce garbage. These are enlarged directly.
    let mut flat = Vec::new();
    for (k, e) in game.images.images.iter().enumerate() {
        if k % 25 == 0 {
            progress(format!("Preparing images {}/{total}...", k + 1));
        }
        let Ok(img) = e.decode_rgba() else { continue };
        if img.width * img.height < MIN_PIXELS {
            continue;
        }
        if let Some(c) = flat_colour(&img) {
            flat.push((e.handle, c));
            continue;
        }
        let rgb: Vec<u8> = bleed(&img).into_iter().flatten().collect();
        write_png(&inp.join(format!("{}.png", e.handle)), img.width, img.height, png::ColorType::Rgb, &rgb)?;
        sent.push(e.handle);
    }
    if sent.is_empty() && flat.is_empty() {
        return Err("the game has no images to upscale".into());
    }

    // 2. Real-ESRGAN over the folder.
    let (model, model_scale) = match style {
        Style::Art => ("realesr-animevideov3", scale),
        Style::Render => ("realesrgan-x4plus", 4),
    };
    let label = format!("{}, {scale}x", style.label());
    if !sent.is_empty() {
        ai_pass(exe, model, model_scale, inp, outp, &[], sent.len(), &label, progress)?;
    }

    // 3. Back to the wanted size, with the original transparency. Images the AI got wrong
    // (missing, or black: the GPU driver sometimes drops a long run, "device lost", and
    // everything after comes out black) are run again in small batches with gentler
    // settings; any that still fail keep their original image.
    let pack = pack_dir(game_path);
    let images = pack.join("images");
    let _ = std::fs::remove_dir_all(&images);
    std::fs::create_dir_all(&images).map_err(|e| e.to_string())?;
    let mut written = 0;
    for &(h, colour) in &flat {
        if let Some(orig) = game.images.get(h).and_then(|e| e.decode_rgba().ok()) {
            let (nw, nh) = target_size(orig.width, orig.height, scale);
            let rgb: Vec<u8> = std::iter::repeat_n(colour, (nw * nh) as usize).flatten().collect();
            write_png(&images.join(format!("{h}.png")), nw, nh, png::ColorType::Rgba, &with_alpha(&orig, &rgb, nw, nh))?;
            written += 1;
        }
    }
    let mut todo = sent.clone();
    for attempt in 0..3 {
        let mut failed = Vec::new();
        for (k, &h) in todo.iter().enumerate() {
            if k % 10 == 0 {
                progress(format!("Saving the pack {}/{}...", k + 1, todo.len()));
            }
            match save_one(game, h, scale, outp, &images)? {
                true => written += 1,
                false => failed.push(h),
            }
        }
        if failed.is_empty() || attempt == 2 {
            todo = failed;
            break;
        }
        // Again, only the failed ones, 25 at a time (a dropped run then loses little).
        for (b, batch) in failed.chunks(25).enumerate() {
            let retry_in = inp.with_file_name("retry");
            let _ = std::fs::remove_dir_all(&retry_in);
            std::fs::create_dir_all(&retry_in).map_err(|e| e.to_string())?;
            for h in batch {
                let name = format!("{h}.png");
                let _ = std::fs::remove_file(outp.join(&name));
                std::fs::copy(inp.join(&name), retry_in.join(&name)).map_err(|e| e.to_string())?;
            }
            let what = format!("{label}, again: {}/{}", (b * 25 + batch.len()).min(failed.len()), failed.len());
            // Errors here just leave those images failed.
            let _ = ai_pass(exe, model, model_scale, &retry_in, outp, &["-t", "128", "-j", "1:1:1"], 0, &what, progress);
        }
        todo = failed;
    }
    let skipped = todo.len();
    let readme = format!(
        "Made by KlikCore with Real-ESRGAN ({model}, {} style, {scale}x).\r\nDelete this folder (or use \"Remove AI upscale pack\" in the launcher) to go back to the original images.\r\n",
        style.label()
    );
    let _ = std::fs::write(pack.join("README.txt"), readme);
    let note = if skipped > 0 { format!("; {skipped} kept their original image (the AI failed on them)") } else { String::new() };
    Ok(format!("AI upscale pack made: {written} images at {scale}x ({}){note}.", style.label()))
}

/// One Real-ESRGAN run over a folder. `count` > 0 shows "done/count" progress.
#[allow(clippy::too_many_arguments)]
fn ai_pass(exe: &Path, model: &str, model_scale: u32, inp: &Path, outp: &Path, extra: &[&str], count: usize, label: &str, progress: &dyn Fn(String)) -> Result<(), String> {
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
            progress(format!("Upscaling with AI {done}/{count} ({label})..."));
        } else {
            progress(format!("Upscaling with AI ({label})..."));
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

/// Turn one AI output into the pack image. Ok(false) when the output is missing or came
/// out black although the original isn't.
fn save_one(game: &GameData, h: u32, scale: u32, outp: &Path, images: &Path) -> Result<bool, String> {
    let Ok((aw, ah, rgb)) = read_rgb(&outp.join(format!("{h}.png"))) else { return Ok(false) };
    let Some(orig) = game.images.get(h).and_then(|e| e.decode_rgba().ok()) else { return Ok(true) };
    // The visible part's average colour must survive (a wrong output is often black or
    // grey all over).
    let (ow, oh) = (orig.width, orig.height);
    let small = resize(&rgb, aw as usize, ah as usize, 3, ow as usize, oh as usize);
    let (mut a, mut b, mut n) = ([0f64; 3], [0f64; 3], 0f64);
    for (o, s) in orig.pixels.chunks_exact(4).zip(small.chunks_exact(3)) {
        if o[3] > 127 {
            for c in 0..3 {
                a[c] += o[c] as f64;
                b[c] += s[c] as f64;
            }
            n += 1.0;
        }
    }
    if n > 0.0 && (0..3).map(|c| (a[c] - b[c]).abs() / n).fold(0.0, f64::max) > 48.0 {
        return Ok(false);
    }
    let (nw, nh) = target_size(ow, oh, scale);
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
