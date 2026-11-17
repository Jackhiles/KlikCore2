//! Music bank playback (MIDI files, used by older games).
//!
//! Windows plays them on the system's General MIDI synthesizer, as the original runtimes
//! did. Elsewhere a software synthesizer (rustysynth) renders them into the mixer with a
//! General MIDI SoundFont, downloaded into the data folder the first time music plays
//! (GeneralUser GS by S. Christian Collins, free to use and share); music stays silent until
//! it is there.

use crate::midi::Song;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Shared with the playing tune: stop, pause, and loudness (0-1).
#[derive(Debug, Default)]
pub struct Control {
    pub stop: AtomicBool,
    pub paused: AtomicBool,
    gain: AtomicU32,
}

impl Control {
    pub fn new(gain: f32) -> Arc<Self> {
        let c = Self::default();
        c.set_gain(gain);
        Arc::new(c)
    }

    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    pub fn set_gain(&self, g: f32) {
        self.gain.store(g.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

/// Play `song` `loops` times (0 = forever) on the Windows MIDI synthesizer, on a thread of
/// its own. Velocities are scaled by the gain; muting silences the notes that are sounding.
#[cfg(windows)]
pub fn play_system(song: Song, loops: u32, ctl: Arc<Control>) {
    use std::ffi::c_void;
    use std::time::{Duration, Instant};
    #[link(name = "winmm")]
    extern "system" {
        fn midiOutOpen(out: *mut *mut c_void, device: u32, callback: usize, instance: usize, flags: u32) -> u32;
        fn midiOutShortMsg(out: *mut c_void, msg: u32) -> u32;
        fn midiOutReset(out: *mut c_void) -> u32;
        fn midiOutClose(out: *mut c_void) -> u32;
    }
    const MIDI_MAPPER: u32 = 0xFFFF_FFFF;
    std::thread::spawn(move || {
        let mut h: *mut c_void = std::ptr::null_mut();
        // SAFETY: winmm writes the handle into `h`; no callback is used.
        if unsafe { midiOutOpen(&mut h, MIDI_MAPPER, 0, 0, 0) } != 0 || h.is_null() {
            eprintln!("music: no MIDI output device");
            return;
        }
        // SAFETY: `h` is the open device for this whole thread.
        let send = |m: [u8; 3]| unsafe {
            midiOutShortMsg(h, m[0] as u32 | (m[1] as u32) << 8 | (m[2] as u32) << 16);
        };
        let silence = || {
            for ch in 0..16u8 {
                send([0xB0 | ch, 123, 0]); // all notes off
                send([0xB0 | ch, 121, 0]); // reset controllers
            }
        };
        let mut pass = 0;
        'song: loop {
            let start = Instant::now();
            let mut paused_for = Duration::ZERO;
            let mut was_silent = false;
            for &(t, msg) in &song.events {
                loop {
                    if ctl.stop.load(Ordering::Relaxed) {
                        break 'song;
                    }
                    if ctl.paused.load(Ordering::Relaxed) {
                        let since = Instant::now();
                        for ch in 0..16u8 {
                            send([0xB0 | ch, 123, 0]);
                        }
                        while ctl.paused.load(Ordering::Relaxed) && !ctl.stop.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(15));
                        }
                        paused_for += since.elapsed();
                        continue;
                    }
                    let silent = ctl.gain() <= 0.0;
                    if silent && !was_silent {
                        for ch in 0..16u8 {
                            send([0xB0 | ch, 123, 0]);
                        }
                    }
                    was_silent = silent;
                    let now = (start.elapsed() - paused_for).as_secs_f64();
                    if now >= t {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs_f64((t - now).min(0.01)));
                }
                let mut m = msg;
                if m[0] & 0xF0 == 0x90 && m[2] > 0 {
                    m[2] = (m[2] as f32 * ctl.gain()).round().clamp(0.0, 127.0) as u8;
                }
                send(m);
            }
            pass += 1;
            if loops != 0 && pass >= loops {
                break;
            }
            silence();
        }
        silence();
        // SAFETY: closing the device opened above.
        unsafe {
            midiOutReset(h);
            midiOutClose(h);
        }
    });
}

/// A tune on the software synthesizer, rendered from the mixer's callback.
#[cfg(not(windows))]
pub struct SynthTrack {
    synth: rustysynth::Synthesizer,
    song: Song,
    next: usize,
    time: f64,
    rate: f64,
    loops: u32,
    pass: u32,
    pub ctl: Arc<Control>,
    left: Vec<f32>,
    right: Vec<f32>,
    pub finished: bool,
}

#[cfg(not(windows))]
impl SynthTrack {
    pub fn new(font: &Arc<rustysynth::SoundFont>, rate: u32, song: Song, loops: u32, ctl: Arc<Control>) -> Option<Self> {
        let settings = rustysynth::SynthesizerSettings::new(rate as i32);
        let synth = rustysynth::Synthesizer::new(font, &settings).ok()?;
        Some(Self { synth, song, next: 0, time: 0.0, rate: rate as f64, loops, pass: 0, ctl, left: vec![0.0; 64], right: vec![0.0; 64], finished: false })
    }

    /// Add `frames` stereo frames into `out` (interleaved).
    pub fn render(&mut self, out: &mut [f32], frames: usize) {
        if self.finished || self.ctl.paused.load(Ordering::Relaxed) {
            return;
        }
        let gain = self.ctl.gain();
        let mut done = 0;
        while done < frames {
            while let Some(&(t, m)) = self.song.events.get(self.next) {
                if t > self.time {
                    break;
                }
                self.synth.process_midi_message((m[0] & 0x0F) as i32, (m[0] & 0xF0) as i32, m[1] as i32, m[2] as i32);
                self.next += 1;
            }
            if self.next >= self.song.events.len() {
                self.pass += 1;
                if self.loops != 0 && self.pass >= self.loops {
                    self.synth.note_off_all(false);
                    self.finished = true;
                } else {
                    self.synth.note_off_all(true);
                    self.next = 0;
                    self.time = 0.0;
                }
            }
            let n = (frames - done).min(64);
            self.synth.render(&mut self.left[..n], &mut self.right[..n]);
            for k in 0..n {
                out[(done + k) * 2] += self.left[k] * gain;
                out[(done + k) * 2 + 1] += self.right[k] * gain;
            }
            done += n;
            self.time += n as f64 / self.rate;
            if self.finished {
                break;
            }
        }
    }
}

/// The SoundFont for the software synthesizer, once loaded. The first call starts loading
/// (and, if needed, downloading) it in the background.
#[cfg(not(windows))]
pub fn soundfont() -> Option<Arc<rustysynth::SoundFont>> {
    use std::sync::{Mutex, OnceLock};
    static FONT: OnceLock<Mutex<(bool, Option<Arc<rustysynth::SoundFont>>)>> = OnceLock::new();
    let cell = FONT.get_or_init(|| Mutex::new((false, None)));
    let mut g = cell.lock().ok()?;
    if let Some(f) = &g.1 {
        return Some(f.clone());
    }
    if !g.0 {
        g.0 = true;
        std::thread::spawn(move || {
            let dir = crate::runtime::files::data_root().join("tools").join("soundfont");
            let existing = std::fs::read_dir(&dir).ok().and_then(|d| {
                d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sf2")))
            });
            let path = match existing {
                Some(p) => p,
                None => {
                    let _ = std::fs::create_dir_all(&dir);
                    let part = dir.join("GeneralUser-GS.sf2.part");
                    let path = dir.join("GeneralUser-GS.sf2");
                    eprintln!("music: downloading a General MIDI SoundFont (32 MB, once)...");
                    if let Err(e) = crate::launcher::update::download(SOUNDFONT_URL, &part).and_then(|_| std::fs::rename(&part, &path).map_err(|e| e.to_string())) {
                        eprintln!("music: SoundFont download failed: {e}");
                        return;
                    }
                    path
                }
            };
            let font = std::fs::File::open(&path).ok().and_then(|mut f| rustysynth::SoundFont::new(&mut f).ok());
            match font {
                Some(f) => {
                    if let Ok(mut g) = cell.lock() {
                        g.1 = Some(Arc::new(f));
                    }
                }
                None => eprintln!("music: could not read {}", path.display()),
            }
        });
    }
    None
}

#[cfg(not(windows))]
const SOUNDFONT_URL: &str = "https://github.com/mrbumpy409/GeneralUser-GS/raw/main/GeneralUser-GS.sf2";

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Plays about a second on the Windows MIDI synthesizer (audible; run by hand with
    /// `cargo test -- --ignored windows_synth`).
    #[test]
    #[ignore]
    fn windows_synth_plays_and_stops() {
        let song = Song {
            events: vec![(0.0, [0x90, 60, 100]), (0.0, [0x90, 64, 100]), (0.0, [0x90, 67, 100]), (0.8, [0x80, 60, 0]), (0.8, [0x80, 64, 0]), (0.8, [0x80, 67, 0])],
            duration: 0.8,
        };
        let ctl = Control::new(0.8);
        play_system(song, 1, ctl.clone());
        std::thread::sleep(std::time::Duration::from_millis(1200));
        ctl.stop.store(true, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
