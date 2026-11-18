//! Click & Create / The Games Factory `.cca` games: a small runtime .exe (`CNCSA32`) next
//! to a `.cca` file holding the whole game. The layout is described in `docs/cca-format.md`
//! (worked out from game files). This reads it into the same `GameData` as Fusion games:
//!
//! - objects are stored per frame there; here every frame's objects get their own range of
//!   handles, and instances and event references are renumbered to match;
//! - events use the Multimedia Fusion 1.5 layout, so the existing parser reads them;
//! - images are palette-based (the game palette); bank images use per-row transparent runs
//!   plus one run-length pixel stream, images embedded in objects (backdrops, quick backdrop
//!   tiles) one run-length stream over the whole picture.

use super::events::{self, Events, Param, TokenValue};
use super::frame::{Frame, Layer, ObjectInstance};
use super::header::{AppHeader, PlayerControls};
use super::images::{ImageBank, ImageEntry};
use super::objects::{ExtensionInfo, ObjectInfo, ObjectKind};
use super::properties::{
    Animation, AnimationDirection, Backdrop, CounterSettings, Fill, Movement, MovementKind, ObjectCommon, ObjectProperties, QuickBackdrop, Shape, ShapeKind,
};
use super::reader::Blob;
use super::{GameData, ContainerKind};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn i16_at(b: &[u8], o: usize) -> Option<i16> {
    u16_at(b, o).map(|v| v as i16)
}
fn i32_at(b: &[u8], o: usize) -> Option<i32> {
    u32_at(b, o).map(|v| v as i32)
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn cstr(b: &[u8], o: usize, max: usize) -> String {
    b.get(o..(o + max).min(b.len())).unwrap_or(&[]).iter().take_while(|&&c| c != 0).map(|&c| c as char).collect()
}

/// Is this a `.cca` game file?
pub fn is_cca(b: &[u8]) -> bool {
    b.len() > 0x200 && &b[0..4] == b"GAME" && u16_at(b, 4).is_some_and(|v| (0x0200..0x0300).contains(&v))
}

/// A Windows LOGPALETTE (`00 03 00 01` + 256 × RGBF) at or after `from`.
fn find_palette(b: &[u8], from: usize, to: usize) -> Option<(usize, Vec<[u8; 4]>)> {
    let at = b.get(from..to.min(b.len()))?.windows(4).position(|w| w == [0, 3, 0, 1])? + from;
    let pal = (0..256).map(|i| {
        let p = at + 4 + 4 * i;
        [b[p], b[p + 1], b[p + 2], 255]
    });
    Some((at, pal.collect()))
}

/// An embedded image's pixels: one run-length stream over the whole picture, rows padded
/// to an even number of bytes. `unit` is the bytes per pixel (1 = palette index, 2 = 15-bit colour).
pub fn decode_rle(data: &[u8], w: usize, h: usize, unit: usize) -> Vec<u16> {
    let out = decode_stream(data, unit);
    let stride = (w * unit).div_ceil(2) * 2 / unit;
    let mut px = vec![0u16; w * h];
    for y in 0..h {
        for x in 0..w {
            px[y * w + x] = out.get(y * stride + x).copied().unwrap_or(0);
        }
    }
    px
}

/// Decode a bank image's data (after its 24-byte header) to pixel values plus an opacity
/// mask: per-row (skip, draw) runs, then one run-length stream of the drawn pixels. `unit` is
/// the bytes per pixel (1 = palette index, 2 = 15-bit colour).
pub fn decode_masked(d: &[u8], w: usize, h: usize, unit: usize) -> (Vec<u16>, Vec<bool>) {
    let mut px = vec![0u16; w * h];
    let mut mask = vec![false; w * h];
    if h == 0 || d.len() < 4 + 8 * h {
        return (px, mask);
    }
    let rows: Vec<(usize, usize)> = (0..h).map(|r| (u32_at(d, 4 + 8 * r).unwrap_or(0) as usize, u32_at(d, 8 + 8 * r).unwrap_or(0) as usize)).collect();
    let base = rows[0].1;
    let stream = decode_stream(d.get(base + 4..).unwrap_or(&[]), unit);
    for (y, &(runs, start)) in rows.iter().enumerate() {
        let mut r = runs + 4;
        // Row starts count bytes of the decoded stream.
        let (mut x, mut k) = (0usize, start.saturating_sub(base) / unit);
        while x < w && r + 1 < d.len() {
            let (skip, draw) = (d[r] as usize, d[r + 1] as usize);
            r += 2;
            x += skip;
            for _ in 0..draw {
                if x < w {
                    px[y * w + x] = stream.get(k).copied().unwrap_or(0);
                    mask[y * w + x] = true;
                }
                x += 1;
                k += 1;
            }
            if draw == 0 {
                break;
            }
        }
    }
    (px, mask)
}

/// The run-length pixel stream: a byte below 0x80 repeats the next pixel that many times,
/// 0x80 + n copies the next n pixels, 0 ends it. Pixels are `unit` bytes (little-endian).
fn decode_stream(data: &[u8], unit: usize) -> Vec<u16> {
    let px = |i: usize| -> u16 {
        match unit {
            2 => u16_at(data, i).unwrap_or(0),
            _ => data.get(i).copied().unwrap_or(0) as u16,
        }
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let n = data[i] as usize;
        i += 1;
        if n == 0 {
            break;
        }
        if n < 0x80 {
            let v = px(i);
            out.extend(std::iter::repeat_n(v, n));
            i += unit;
        } else {
            for k in 0..(n & 0x7F) {
                out.push(px(i + k * unit));
            }
            i += (n & 0x7F) * unit;
        }
    }
    out
}

struct Ctx {
    palette: std::sync::Arc<Vec<[u8; 4]>>,
    images: Vec<ImageEntry>,
    next_image: u32,
}

impl Ctx {
    /// Add an image embedded in an object record, at `at`: `u16 0x8002, u32 block size, u32
    /// checksum, u16 0, u32 data size, u16 w, u16 h, u8 3, u8 flags, i16 hotspot x / y, 4
    /// bytes`, then the data. Returns its handle.
    fn embedded_image(&mut self, b: &[u8], at: usize) -> Option<u16> {
        let size = u32_at(b, at + 12)? as usize;
        let (w, h) = (u16_at(b, at + 16)?, u16_at(b, at + 18)?);
        let flags = *b.get(at + 21)?;
        let hot = (i16_at(b, at + 22).unwrap_or(0), i16_at(b, at + 24).unwrap_or(0));
        let data = b.get(at + 30..at + 30 + size)?.to_vec();
        let handle = self.next_image;
        self.next_image += 1;
        self.images.push(ImageEntry::cca(handle, w, h, hot, (0, 0), data, flags & 0x40 != 0, pixel_bytes(b.get(at + 20).copied().unwrap_or(3), flags), self.palette.clone()));
        Some(handle as u16)
    }
}

/// Bytes per pixel from an image's colour mode byte (3 palette indices, 6 15-bit colour) and
/// flags (0x?1 / 0x?2 the same, but not always set).
fn pixel_bytes(mode: u8, flags: u8) -> u8 {
    if mode == 6 || flags & 0x03 == 2 {
        2
    } else {
        1
    }
}

/// The game file a runtime .exe stands for: `<name>.cca` beside it.
pub fn companion(exe: &std::path::Path) -> Option<PathBuf> {
    let p = exe.with_extension("cca");
    p.is_file().then_some(p)
}

pub fn load(file: Blob, path: PathBuf) -> Result<GameData> {
    let b: &[u8] = &file;
    if !is_cca(b) {
        bail!("not a Click & Create game file");
    }
    let mut warnings = Vec::new();
    let name = cstr(b, 0x06, 80);
    let author = cstr(b, 0x56, 158);
    let (ww, wh) = (u16_at(b, 0xF8).unwrap_or(640), u16_at(b, 0xFA).unwrap_or(480));
    let mut controls: [PlayerControls; 4] = Default::default();
    for (p, c) in controls.iter_mut().enumerate() {
        c.control_type = u16_at(b, 0x108 + 2 * p).unwrap_or(4);
        for k in 0..8 {
            c.keys[k] = u16_at(b, 0x110 + 16 * p + 2 * k).unwrap_or(0);
        }
    }
    let nframes = u16_at(b, 0x15C).context("frame count")? as usize;
    let frame_offsets: Vec<usize> = (1..=nframes).filter_map(|i| u32_at(b, 0x162 + 4 * i).map(|v| v as usize)).collect();
    let first = *frame_offsets.first().context("no frames")?;
    // The palette as the first frame has it (the header area can contain the same byte
    // pattern by chance, inside the icon or the preview picture).
    let (_, pal) = find_palette(b, first, first + u32_at(b, first - 4).unwrap_or(0) as usize).context("game palette")?;
    let mut ctx = Ctx { palette: std::sync::Arc::new(pal), images: Vec::new(), next_image: 0 };

    // Image bank: right after the last frame.
    let last = *frame_offsets.last().expect("checked");
    let after = last + u32_at(b, last - 4).unwrap_or(0) as usize;
    let bank = (after.saturating_sub(8)..after + 32)
        .find(|&p| u32_at(b, p).is_some_and(|n| n > 0 && n < 100_000 && u32_at(b, p + 4) == Some(4 + 8 * n)))
        .context("image bank")?;
    let count = u32_at(b, bank).unwrap_or(0) as usize;
    for i in 0..count {
        let (Some(off), Some(size)) = (u32_at(b, bank + 4 + 8 * i), u32_at(b, bank + 8 + 8 * i)) else { break };
        let p = bank + off as usize;
        let (w, h) = (u16_at(b, p + 10).unwrap_or(0), u16_at(b, p + 12).unwrap_or(0));
        let hot = (i16_at(b, p + 16).unwrap_or(0), i16_at(b, p + 18).unwrap_or(0));
        let act = (i16_at(b, p + 20).unwrap_or(0), i16_at(b, p + 22).unwrap_or(0));
        let flags = b.get(p + 15).copied().unwrap_or(0x41);
        let data = b.get(p + 24..p + size as usize).unwrap_or(&[]).to_vec();
        ctx.images.push(ImageEntry::cca(i as u32, w, h, hot, act, data, flags & 0x40 != 0, pixel_bytes(b.get(p + 14).copied().unwrap_or(3), flags), ctx.palette.clone()));
    }
    ctx.next_image = count as u32;
    // After the images: the font, music and sound banks, each a table like the image bank's.
    let images_end = (0..count)
        .filter_map(|i| Some(bank + u32_at(b, bank + 4 + 8 * i)? as usize + u32_at(b, bank + 8 + 8 * i)? as usize))
        .max()
        .unwrap_or(bank);
    let (music, sounds) = read_audio_banks(b, images_end);

    let mut objects: Vec<ObjectInfo> = Vec::new();
    let mut extensions: Vec<ExtensionInfo> = Vec::new();
    let mut frames = Vec::new();
    for (fi, &f) in frame_offsets.iter().enumerate() {
        // Each frame's objects get handles from just past the highest one used so far.
        let base = objects.iter().map(|o| o.handle + 1).max().unwrap_or(0);
        match read_frame(b, f, base, &mut ctx, &mut objects, &mut extensions) {
            Ok(frame) => frames.push(frame),
            Err(e) => warnings.push(format!("frame #{fi}: {e:#}")),
        }
    }
    let object_index = objects.iter().enumerate().map(|(i, o)| (o.handle as u32, i)).collect();
    let header = AppHeader {
        window_width: ww,
        window_height: wh,
        controls,
        frame_count: frames.len() as u32,
        frame_rate: 50,
        initial_lives: 3,
        border_color: [0, 0, 0, 255],
        ..Default::default()
    };
    Ok(GameData {
        path,
        container: ContainerKind::Ccn,
        data_offset: 0,
        pack_files: Vec::new(),
        unicode: false,
        runtime_version: 0x0207,
        runtime_subversion: 0,
        product_version: 0,
        product_build: 0,
        encrypted: false,
        old: true,
        header,
        name,
        author,
        copyright: String::new(),
        editor_filename: String::new(),
        target_filename: String::new(),
        about_text: String::new(),
        extensions,
        objects,
        object_index,
        frame_handles: (0..frames.len() as u16).collect(),
        frames,
        global_values: Vec::new(),
        global_strings: Vec::new(),
        images: ImageBank::from_entries(ctx.images),
        sounds,
        music,
        fonts: Default::default(),
        chunks: Vec::new(),
        warnings,
        icon: None,
    })
}

fn read_frame(b: &[u8], f: usize, base: u16, ctx: &mut Ctx, objects: &mut Vec<ObjectInfo>, extensions: &mut Vec<ExtensionInfo>) -> Result<Frame> {
    let size = u32_at(b, f - 4).context("frame size")? as usize;
    let end = (f + size).min(b.len());
    let (w, h) = (u16_at(b, f + 4).unwrap_or(640) as i32, u16_at(b, f + 6).unwrap_or(480) as i32);
    let name = cstr(b, f + 8, 64);
    let (pal_at, palette) = find_palette(b, f, end).context("frame palette")?;
    // Thumbnail, then the frame's blocks.
    let mut p = pal_at + 4 + 1024;
    p += 6 + u32_at(b, p + 2).unwrap_or(0) as usize;
    let mut instances = Vec::new();
    let mut event_sheet = Events::default();
    while p + 6 <= end {
        let (Some(kind), Some(sz)) = (u16_at(b, p), u32_at(b, p + 2)) else { break };
        let sz = sz as usize;
        if p + 6 + sz > end + 8 {
            break;
        }
        let body = p + 6;
        match kind & 0x7FFF {
            2 => read_objects(b, body, body + sz, base, ctx, objects, extensions),
            3 => {
                let n = u16_at(b, body).unwrap_or(0) as usize;
                let recs = body + 2 + 4 * n;
                for k in 0..n {
                    let r = recs + 32 * k;
                    let (Some(oi), Some(x), Some(y)) = (u16_at(b, r), i16_at(b, r + 2), i16_at(b, r + 4)) else { break };
                    instances.push(ObjectInstance { handle: k as u16, object_info: base + oi, x: x as i32, y: y as i32, parent_type: 0, parent_handle: -1, layer: 0 });
                }
            }
            4 => {
                // "Fra\0", u16, u32 size, then 1.5-layout event groups.
                let ev_size = u32_at(b, body + 6).unwrap_or(0) as usize;
                let all = b.get(body + 10..(body + 10 + ev_size).min(body + sz)).unwrap_or(&[]);
                // Groups run until a zero-sized one (the end marker).
                let mut g = 0;
                while g + 2 <= all.len() {
                    let gs = (i16_at(all, g).unwrap_or(0) as i32).unsigned_abs() as usize;
                    if gs == 0 || g + gs > all.len() {
                        break;
                    }
                    g += gs;
                }
                let groups = &all[..g];
                let mut wrapped = b"ERev".to_vec();
                wrapped.extend_from_slice(&(groups.len() as u32).to_le_bytes());
                wrapped.extend_from_slice(groups);
                match events::parse_events(&wrapped, false, true) {
                    Ok(mut e) => {
                        renumber(&mut e, base);
                        event_sheet = e;
                    }
                    Err(e) => bail!("events: {e:#}"),
                }
            }
            _ => {}
        }
        p = body + sz;
    }
    let bg = b.get(f + 0x4A..f + 0x4D).map(|c| [c[0], c[1], c[2], 255]).unwrap_or([0, 0, 0, 255]);
    Ok(Frame {
        name,
        width: w,
        height: h,
        background_color: bg,
        flags: 0,
        password: None,
        virtual_rect: None,
        layers: vec![Layer { flags: Layer::VISIBLE, x_coefficient: 1.0, y_coefficient: 1.0, background_count: 0, background_index: 0, name: String::new() }],
        instances,
        fade_in: None,
        fade_out: None,
        random_seed: None,
        movement_timer_base: None,
        events: None,
        event_sheet,
        palette: Some(palette),
    })
}

/// Object records: found by their self-pointer (the u32 at name + 64 = file offset of name + 68).
fn read_objects(b: &[u8], start: usize, end: usize, base: u16, ctx: &mut Ctx, objects: &mut Vec<ObjectInfo>, extensions: &mut Vec<ExtensionInfo>) {
    let mut n = start;
    // Instances and events refer to objects by their position in this list.
    let mut index: u16 = 0;
    while n + 74 < end {
        if u32_at(b, n + 64) != Some((n + 68) as u32) {
            n += 1;
            continue;
        }
        let name = cstr(b, n, 40);
        let typ = u16_at(b, n + 42).unwrap_or(0);
        let id = u16_at(b, n + 46).unwrap_or(1);
        let psz = u32_at(b, n + 70).unwrap_or(0) as usize;
        let props_at = n + 74;
        // A record whose fields don't add up (a long name spilling over them, or a match
        // inside other data): skip it, but keep scanning byte by byte.
        let base_type = typ & 0xFF;
        let hidden = typ & 0x800 != 0;
        let known = typ < 0x1000 && typ != 0xFFF && (base_type <= 7 || base_type >= 32);
        let _ = id;
        if !known || props_at + psz > end {
            // An object whose record has a different layout (e.g. a text object holding a
            // long text): keep its place in the numbering.
            objects.push(ObjectInfo { handle: base + index, type_id: -1, kind: ObjectKind::Unknown(-1), name: cstr(b, n, 40), flags: 0, ink_effect: 0, ink_effect_param: 0, properties: None, props: ObjectProperties::None });
            index += 1;
            n += 68;
            continue;
        }
        let kind_id = base_type as i16;
        let handle = base + index;
        index += 1;
        let (kind, props) = match base_type {
            0 => {
                let (bw, bh) = (u16_at(b, n + 34).unwrap_or(32) as u32, u16_at(b, n + 36).unwrap_or(32) as u32);
                let fill = if u16_at(b, n + 68) == Some(0x8002) {
                    ctx.embedded_image(b, n + 68).map(Fill::Motif).unwrap_or(Fill::Solid([0, 0, 0, 255]))
                } else {
                    let c = b.get(n + 28..n + 31).map(|c| [c[0], c[1], c[2], 255]).unwrap_or([0, 0, 0, 255]);
                    Fill::Solid(c)
                };
                (
                    ObjectKind::QuickBackdrop,
                    ObjectProperties::QuickBackdrop(QuickBackdrop { obstacle: 0, collision: 0, width: bw, height: bh, shape: Shape { border_size: 0, border_color: [0; 4], kind: ShapeKind::Rectangle, fill, line_flags: 0 } }),
                )
            }
            1 => {
                let img = if u16_at(b, n + 68) == Some(0x8002) { ctx.embedded_image(b, n + 68) } else { None };
                let (bw, bh) = img.and_then(|h| ctx.images.iter().find(|e| e.handle == h as u32)).map_or((1, 1), |e| (e.width as u32, e.height as u32));
                (ObjectKind::Backdrop, ObjectProperties::Backdrop(Backdrop { obstacle: 0, collision: 0, width: bw, height: bh, image: img.unwrap_or(0) }))
            }
            _ => {
                // Shown at start unless the type's 0x800 flag says hidden (text that a
                // "display text" action shows later).
                let shown = if hidden { 0 } else { super::properties::oc_new_flags::VISIBLE_AT_START };
                let mut c = ObjectCommon { new_flags: shown, ..Default::default() };
                let props = b.get(props_at..(props_at + psz).min(end)).unwrap_or(&[]);
                if base_type == 2 {
                    read_active(props, &mut c);
                }
                if base_type == 3 {
                    let (w, h) = (u16_at(b, n + 34).unwrap_or(0) as u32, u16_at(b, n + 36).unwrap_or(0) as u32);
                    c.text = read_text(props, w, h);
                }
                if base_type == 7 {
                    // `CNTR`, u16 size, then initial, minimum, maximum (i32), then how it shows.
                    if let Some(at) = props.windows(4).position(|w| w == b"CNTR") {
                        let p = at + 6;
                        let (initial, lo, hi) = (i32_at(props, p).unwrap_or(0), i32_at(props, p + 4).unwrap_or(0), i32_at(props, p + 8).unwrap_or(999_999_999));
                        let (minimum, maximum) = if lo <= hi { (lo, hi) } else { (0, 999_999_999) };
                        c.counter = Some(CounterSettings { initial: initial.clamp(minimum, maximum), minimum, maximum });
                        c.counter_display = read_display(props, p + 12, 0);
                    }
                }
                // Score / lives: `SCRE` / `LIVE`, then how they show (digit images, or a life
                // icon repeated per life).
                for (tag, player_kind) in [(b"SCRE", 5u16), (b"LIVE", 6)] {
                    if base_type == player_kind {
                        if let Some(at) = props.windows(4).position(|w| w == tag) {
                            c.counter_display = read_display(props, at + 4, 1);
                        }
                    }
                }
                let kind = match base_type {
                    2 => ObjectKind::Active,
                    3 => ObjectKind::Text,
                    5 => ObjectKind::Score,
                    6 => ObjectKind::Lives,
                    7 => ObjectKind::Counter,
                    t if t >= 32 => {
                        let slot = t as i16;
                        if !extensions.iter().any(|e| e.handle == slot - 32) {
                            extensions.push(ExtensionInfo { handle: slot - 32, magic: 0, version_ls: 0, version_ms: 0, name: ext_name(b, (slot - 32) as usize), sub_type: String::new() });
                        }
                        ObjectKind::Extension(slot)
                    }
                    _ => ObjectKind::Unknown(kind_id),
                };
                (kind, ObjectProperties::Common(Box::new(c)))
            }
        };
        objects.push(ObjectInfo { handle, type_id: kind_id, kind, name, flags: 0, ink_effect: 0, ink_effect_param: 0, properties: None, props });
        n += 74 + psz.max(1);
    }
}

/// The music (MIDI files) and sound banks after the image bank. Each entry: a 32-byte header
/// (`u32 ?, u16 ?, u32 size, name` up to 18 bytes, `u32 ?`), then `size` bytes: a MIDI file,
/// or for a sound a 16-byte WAVEFORMAT and the PCM samples (rebuilt here as a WAV file).
/// The banks are told apart by their contents; the font bank (104-byte LOGFONT entries) is
/// skipped.
fn read_audio_banks(b: &[u8], from: usize) -> (super::SoundBank, super::SoundBank) {
    use super::SoundBank;
    let (mut music, mut sounds) = (Vec::new(), Vec::new());
    let mut p = from;
    while p + 12 < b.len() {
        let n = u32_at(b, p).unwrap_or(0) as usize;
        if !(n > 0 && n < 10_000 && u32_at(b, p + 4) == Some((4 + 8 * n) as u32)) {
            p += 1;
            continue;
        }
        let mut files = Vec::with_capacity(n);
        let mut end = p + 4 + 8 * n;
        let mut kind = None;
        for i in 0..n {
            let (Some(off), Some(len)) = (u32_at(b, p + 4 + 8 * i), u32_at(b, p + 8 + 8 * i)) else { break };
            let e = p + off as usize;
            end = end.max(e + len as usize);
            if len == 0 {
                files.push((String::new(), Vec::new()));
                continue;
            }
            let size = u32_at(b, e + 6).unwrap_or(0) as usize;
            let name = cstr(b, e + 10, 18);
            let Some(data) = b.get(e + 32..e + 32 + size) else { break };
            if data.starts_with(b"MThd") {
                kind = Some(true);
                files.push((name, data.to_vec()));
            } else if size > 16 && u16_at(data, 0) == Some(1) {
                kind = Some(false);
                let (fmt, pcm) = data.split_at(16);
                let mut wav = Vec::with_capacity(44 + pcm.len());
                wav.extend_from_slice(b"RIFF");
                wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
                wav.extend_from_slice(b"WAVEfmt ");
                wav.extend_from_slice(&16u32.to_le_bytes());
                wav.extend_from_slice(fmt);
                wav.extend_from_slice(b"data");
                wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
                wav.extend_from_slice(pcm);
                files.push((name, wav));
            } else {
                files.push((name, Vec::new()));
            }
        }
        match kind {
            Some(true) if music.is_empty() => music = files,
            Some(false) if sounds.is_empty() => sounds = files,
            _ => {}
        }
        p = end.max(p + 1);
    }
    (SoundBank::from_files(music), SoundBank::from_files(sounds))
}

/// How a counter, score or lives object shows: `u16 size, u16 type, u16 ?`, then for digit
/// images (1) and icons (4) `u16 count, u16 images[count]`, for bars (2 vertical, 3
/// horizontal) `u16 width, u16 ?, r, g, b, ?, u16 height`; 0 = hidden.
fn read_display(p: &[u8], at: usize, player: u16) -> Option<super::properties::CounterDisplay> {
    use super::properties::{CounterDisplay, CounterDisplayKind};
    let typ = u16_at(p, at + 2)?;
    let images = || -> Vec<u16> {
        let n = u16_at(p, at + 6).unwrap_or(0) as usize;
        (0..n.min(16)).filter_map(|k| u16_at(p, at + 8 + 2 * k)).collect()
    };
    let (kind, w, h) = match typ {
        0 => (CounterDisplayKind::Hidden, 0, 0),
        1 => (CounterDisplayKind::Numbers(images()), 0, 0),
        4 => (CounterDisplayKind::Animation(images()), 0, 0),
        2 | 3 => {
            let w = u16_at(p, at + 6)? as u32;
            let c = p.get(at + 10..at + 13)?;
            let h = u16_at(p, at + 14)? as u32;
            let shape = Shape { border_size: 0, border_color: [0; 4], kind: ShapeKind::Rectangle, fill: Fill::Solid([c[0], c[1], c[2], 255]), line_flags: 0 };
            (if typ == 2 { CounterDisplayKind::VerticalBar(shape) } else { CounterDisplayKind::HorizontalBar(shape) }, w, h)
        }
        t => (CounterDisplayKind::Unknown(t), 0, 0),
    };
    Some(CounterDisplay { width: w, height: h, player, flags: 0, font: 0, kind })
}

/// A text object's `TEXT` block: `u16 size, u16 ?, u16 count, u16 offsets[count]` (from the
/// size field), then per paragraph `u16 size, u16 font (0xFFFF = as before), u8 r, g, b, u8 ?,
/// u16 flags (alignment, as in Fusion)` and the zero-ended text. Later paragraphs often keep
/// the first one's colour (their colour field reads 00 00 80 00).
pub(crate) fn read_text(p: &[u8], width: u32, height: u32) -> Option<super::properties::TextBlock> {
    use super::properties::{Paragraph, TextBlock};
    let at = p.windows(4).position(|w| w == b"TEXT")? + 4;
    let count = u16_at(p, at + 4)? as usize;
    let mut paragraphs = Vec::with_capacity(count);
    let mut colour = [0, 0, 0, 255];
    for k in 0..count.min(256) {
        let off = at + u16_at(p, at + 6 + 2 * k)? as usize;
        let font = u16_at(p, off + 2)?;
        let c = p.get(off + 4..off + 8)?;
        if k == 0 || c != [0, 0, 0x80, 0] {
            colour = [c[0], c[1], c[2], 255];
        }
        let flags = u16_at(p, off + 8)?;
        let text = cstr(p, off + 10, p.len().saturating_sub(off + 10));
        paragraphs.push(Paragraph { font: if font == 0xFFFF { 0 } else { font }, flags, color: colour, text });
    }
    (!paragraphs.is_empty()).then_some(TextBlock { width, height, paragraphs })
}

/// Extension file name `n` from the header's list (`XXX.COX` -> "xxx").
fn ext_name(b: &[u8], n: usize) -> String {
    let names: Vec<String> = b
        .windows(4)
        .take(0x10000)
        .enumerate()
        .filter(|(_, w)| w.eq_ignore_ascii_case(b".COX"))
        .map(|(i, _)| {
            let start = (0..i).rev().take_while(|&j| b[j].is_ascii_alphanumeric() || b[j] == b'_').last().unwrap_or(i);
            String::from_utf8_lossy(&b[start..i]).to_lowercase()
        })
        .collect();
    names.get(n).cloned().unwrap_or_default()
}

/// Active object properties: a `SPRI` movement block, then the animations.
fn read_active(p: &[u8], c: &mut ObjectCommon) {
    let Some(sp) = p.windows(4).position(|w| w == b"SPRI") else { return };
    let msize = u32_at(p, sp + 4).unwrap_or(0) as usize;
    let mv = sp + 8;
    let mtype = u16_at(p, mv).unwrap_or(0);
    let kind = match mtype {
        4 => MovementKind::Ball {
            speed: i16_at(p, mv + 10).unwrap_or(50),
            randomizer: i16_at(p, mv + 12).unwrap_or(0),
            angles: i16_at(p, mv + 14).unwrap_or(2),
            security: i16_at(p, mv + 16).unwrap_or(0),
            // After the security setting (matched against the game's Fusion rebuild: the
            // same objects have the same deceleration there).
            deceleration: i16_at(p, mv + 18).unwrap_or(0).clamp(0, 100),
        },
        _ => MovementKind::Static,
    };
    c.movements = vec![Movement { player: 0, kind, moving_at_start: true, direction_at_start: 1 }];
    // The animation header: u16 size, u16, i16 offsets[16] (unused ones negative).
    let Some(h) = (mv..(mv + msize + 64).min(p.len().saturating_sub(40))).find(|&h| {
        let first = i16_at(p, h + 4).unwrap_or(0);
        first > 0 && first < 4000 && (0..16).all(|k| i16_at(p, h + 4 + 2 * k).is_some_and(|v| v > 0 || v == -first))
    }) else {
        return;
    };
    let mut anims = Vec::new();
    for k in 0..16 {
        let off = i16_at(p, h + 4 + 2 * k).unwrap_or(-1);
        if off <= 0 {
            anims.push(None);
            continue;
        }
        let a = h + off as usize;
        let mut dirs = Vec::with_capacity(32);
        for d in 0..32 {
            let dof = i16_at(p, a + 2 * d).unwrap_or(-1);
            if dof <= 0 {
                dirs.push(None);
                continue;
            }
            let dp = a + dof as usize;
            let count = u16_at(p, dp + 6).unwrap_or(0) as usize;
            dirs.push(Some(AnimationDirection {
                min_speed: p.get(dp).copied().unwrap_or(0),
                max_speed: p.get(dp + 1).copied().unwrap_or(0),
                repeat: u16_at(p, dp + 2).unwrap_or(0),
                back_to: u16_at(p, dp + 4).unwrap_or(0),
                frames: (0..count).filter_map(|i| u16_at(p, dp + 8 + 2 * i)).collect(),
            }));
        }
        anims.push(Some(Animation { directions: dirs }));
    }
    c.animations = anims;
}

/// Click & Create numbers object conditions and actions differently from Multimedia
/// Fusion; this maps them (worked out by lining up the same events in a game's Click &
/// Create build and its later Fusion rebuild). Numbers not listed are the same.
fn mmf_number(object_type: i16, num: i16, condition: bool) -> i16 {
    let counter_like = matches!(object_type, 3 | 5 | 6 | 7);
    if condition {
        match (counter_like, num) {
            (true, n) if n > -20 => n - 80,
            (false, -2) => -32,
            (false, -13) => -2,
            (false, -15) => -34,
            (false, -17) => -13,
            (false, -18) => -14,
            (false, -27) => -30,
            (false, -28) => -29,
            (false, -29) => -28,
            // Inferred from use: a ball destroyed with an explosion, a ship that dies, on touching
            // the scenery (-37); something falling in pasted into the scenery on landing (-35).
            (false, -35) => -23,
            (false, -37) => -13,
            (_, n) => n,
        }
    } else {
        match (counter_like, num) {
            (true, n) if n < 20 => n + 80,
            (false, 0) => 27,
            (false, 1) => 26,
            (false, 2) => 23,
            (false, 3) => 1,
            (false, 6) => 4,
            (false, 7) => 5,
            (false, 8) => 6,
            (false, 9) => 7,
            (false, 10) => 8,
            (false, 11) => 9,
            (false, 25) => 14,
            (false, 26) => 29,
            (false, 43) => 28,
            (false, 44) => 30,
            // Paste into the background (its parameter is the paste kind).
            (false, 45) => 80,
            (false, 46) => 81,
            (false, 47) => 82,
            (_, n) => n,
        }
    }
}

/// Objects are numbered per frame in the file: move every object reference in a frame's
/// events to the frame's handle range.
fn renumber(e: &mut Events, base: u16) {
    let fix = |oi: &mut u16| {
        if *oi < 0x8000 {
            *oi += base;
        }
    };
    for g in &mut e.groups {
        let counter_like = |t: i16| matches!(t, 3 | 5 | 6 | 7);
        let index_param = |i: i16| events::ParamEntry { code: 50, value: Param::Short(i) };
        for c in &mut g.conditions {
            if (2..32).contains(&c.object_type) {
                // Compare alterable value A / B / C: Fusion's single condition takes the index.
                if !counter_like(c.object_type) && (-21..=-19).contains(&c.num) {
                    let i = -19 - c.num;
                    c.num = -27;
                    c.params.insert(0, index_param(i));
                } else if !counter_like(c.object_type) && matches!(c.num, -25 | -26) {
                    // Compare value B (-25) / C (-26), the pairs of actions 37-39 / 40-42
                    // (seen: "while it's above 0, shoot and subtract 1").
                    let i = if c.num == -25 { 1 } else { 2 };
                    c.num = -27;
                    c.params.insert(0, index_param(i));
                } else {
                    c.num = mmf_number(c.object_type, c.num, true);
                }
            }
        }
        for a in &mut g.actions {
            if (2..32).contains(&a.object_type) {
                // Set / add / subtract alterable value A: Fusion's actions take the index.
                if !counter_like(a.object_type) && (27..=29).contains(&a.num) {
                    a.num += 4;
                    a.params.insert(0, index_param(0));
                } else if !counter_like(a.object_type) && (37..=42).contains(&a.num) {
                    // Set / add / subtract value B (37-39) and C (40-42).
                    let (i, first) = if a.num <= 39 { (1, 37) } else { (2, 40) };
                    a.num = 31 + (a.num - first);
                    a.params.insert(0, index_param(i));
                } else {
                    a.num = mmf_number(a.object_type, a.num, false);
                }
            }
        }
        for a in g.conditions.iter_mut().chain(g.actions.iter_mut()) {
            if a.object_type >= 0 {
                fix(&mut a.object_info);
            }
            for p in &mut a.params {
                match &mut p.value {
                    Param::Object { object_info, .. } => fix(object_info),
                    Param::Create { object_info, position, .. } => {
                        fix(object_info);
                        if position.parent_object_info != 0xFFFF {
                            fix(&mut position.parent_object_info);
                        }
                    }
                    Param::Shoot { object_info, position, .. } => {
                        fix(object_info);
                        if position.parent_object_info != 0xFFFF {
                            fix(&mut position.parent_object_info);
                        }
                    }
                    Param::Position(position) => {
                        if position.parent_object_info != 0xFFFF {
                            fix(&mut position.parent_object_info);
                        }
                    }
                    Param::Expression { tokens, .. } => {
                        for t in tokens.iter_mut() {
                            // Counter-like objects' expressions are numbered from 0 here, 80 in Fusion.
                            if matches!(t.object_type, 3 | 5 | 6 | 7) && (0..20).contains(&t.num) {
                                t.num += 80;
                            } else if t.object_type == 2 && t.num == 0 {
                                // X position (Fusion's 11; 1 = Y is the same).
                                t.num = 11;
                            }
                            if t.object_type >= 0 && !matches!(t.value, TokenValue::None) || t.object_type >= 2 {
                                fix(&mut t.object_info);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_length_streams() {
        // 2 × 7, then the literal bytes 1 2 3, end.
        assert_eq!(decode_stream(&[2, 7, 0x83, 1, 2, 3, 0], 1), vec![7, 7, 1, 2, 3]);
        // 15-bit colour: 3 × 0x7EE0, then the literal pixels 0x0401 0x0422.
        assert_eq!(decode_stream(&[3, 0xE0, 0x7E, 0x82, 1, 4, 0x22, 4, 0], 2), vec![0x7EE0, 0x7EE0, 0x7EE0, 0x0401, 0x0422]);
        // A 3 × 2 picture, rows padded to an even number of bytes (4 per row): 1 2 3 _ 4 5 6 _.
        assert_eq!(decode_rle(&[0x88, 1, 2, 3, 0, 4, 5, 6, 0, 0], 3, 2, 1), vec![1, 2, 3, 4, 5, 6]);
        // 15-bit pixels are 2 bytes, so rows need no padding.
        assert_eq!(decode_rle(&[3, 1, 0, 0], 3, 1, 2), vec![1, 1, 1]);
    }

    #[test]
    fn text_paragraphs() {
        // "TEXT", u16 size, u16 ?, u16 count 2, offsets 10 and 23 (from the size field).
        let mut p = b"TEXT".to_vec();
        p.extend_from_slice(&[40, 0, 5, 0, 2, 0, 10, 0, 23, 0]);
        // Paragraph 0: size, font 0, white, centred, "Hi".
        p.extend_from_slice(&[12, 0, 0, 0, 255, 255, 255, 0, 1, 0]);
        p.extend_from_slice(b"Hi\0");
        // Paragraph 1: same font and colour as before (00 00 80 00), "Yo".
        p.extend_from_slice(&[12, 0, 0xFF, 0xFF, 0, 0, 0x80, 0, 0, 0]);
        p.extend_from_slice(b"Yo\0");
        let t = read_text(&p, 100, 20).unwrap();
        assert_eq!(t.paragraphs.len(), 2);
        assert_eq!((t.paragraphs[0].text.as_str(), t.paragraphs[0].flags), ("Hi", 1));
        assert_eq!(t.paragraphs[1].text, "Yo");
        assert_eq!(t.paragraphs[1].color, [255, 255, 255, 255]);
    }

    #[test]
    fn masked_image() {
        // 3 × 1: skip 1, draw 2; pixels 9, 9.
        let mut d = Vec::new();
        d.extend_from_slice(&0u32.to_le_bytes()); // decoded size (unused)
        d.extend_from_slice(&8u32.to_le_bytes()); // row 0 runs at data + 4 + 8
        d.extend_from_slice(&100u32.to_le_bytes()); // row 0 first pixel (the base)
        d.extend_from_slice(&[1, 2, 0, 0]); // runs: skip 1, draw 2; end
        // pixel stream at base + 4 = 104: pad up to it
        d.resize(104, 0);
        d.extend_from_slice(&[2, 9, 0]);
        let (px, mask) = decode_masked(&d, 3, 1, 1);
        assert_eq!(mask, vec![false, true, true]);
        assert_eq!(&px[1..], &[9, 9]);
    }
}
