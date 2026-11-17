//! Audio output: executes the runtime's queued audio commands on a small mixer.
//!
//! Nothing is decoded ahead of time. Each playing sound has a decoder thread that streams
//! it (resampled to the device rate) a fraction of a second ahead of the mixer, so even
//! long music costs only its compressed bytes plus a short buffer. Start offsets, pan and
//! volume are applied while streaming / mixing.

use crate::audio::{Resampler, Stream};
use crate::runtime::audio::AudioCommand;
use crate::runtime::Runtime;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::HashSet;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};

/// Decoded blocks buffered per sound (a block is ~1k frames, so this is ~0.3 s).
const AHEAD: usize = 12;

/// A sound being mixed.
struct Voice {
    handle: u32,
    rx: Receiver<Vec<f32>>,
    /// Current block (interleaved stereo) and the read position in it.
    buf: Vec<f32>,
    pos: usize,
    volume: f32,
    pan: f32,
}

#[derive(Default)]
struct Mix {
    voices: Vec<Voice>,
    /// Ctrl+M in the viewer: mix silence (sounds keep their place).
    muted: bool,
    /// The player's volume setting (0-1), applied to everything.
    master: f32,
    /// Music bank tune on the software synthesizer.
    #[cfg(not(windows))]
    music: Option<super::music::SynthTrack>,
    /// The mixed stereo frames of the current callback.
    scratch: Vec<f32>,
}

impl Mix {
    /// Mix all voices into `scratch` (interleaved stereo); drops voices that ended.
    fn render(&mut self, frames: usize) {
        self.scratch.clear();
        self.scratch.resize(frames * 2, 0.0);
        #[cfg(not(windows))]
        if let Some(t) = &mut self.music {
            t.render(&mut self.scratch, frames);
            if t.finished {
                self.music = None;
            }
        }
        let out = &mut self.scratch;
        self.voices.retain_mut(|v| {
            let (gl, gr) = (v.volume * (1.0 - v.pan.max(0.0)), v.volume * (1.0 + v.pan.min(0.0)));
            let mut i = 0;
            while i < frames {
                if v.pos >= v.buf.len() {
                    match v.rx.try_recv() {
                        Ok(b) => {
                            v.buf = b;
                            v.pos = 0;
                            continue;
                        }
                        Err(TryRecvError::Empty) => return true, // decoder is behind; catch up next time
                        Err(TryRecvError::Disconnected) => return false, // finished
                    }
                }
                let n = ((v.buf.len() - v.pos) / 2).min(frames - i);
                for k in 0..n {
                    out[(i + k) * 2] += v.buf[v.pos + k * 2] * gl;
                    out[(i + k) * 2 + 1] += v.buf[v.pos + k * 2 + 1] * gr;
                }
                v.pos += n * 2;
                i += n;
            }
            true
        });
    }
}

/// What decoder threads report back.
enum Report {
    /// Exact length of a sound in ms.
    Duration(u32, f64),
    Failed(u32, String),
}

pub struct AudioOut {
    mix: Arc<Mutex<Mix>>,
    /// The device stream (None when muted or no output device is available).
    stream: Option<cpal::Stream>,
    rate: u32,
    tx: Sender<Report>,
    rx: Receiver<Report>,
    /// Sounds that could not be decoded (not retried).
    failed: HashSet<u32>,
    pub enabled: bool,
    /// Replacement sounds from mods.
    pub mods: Arc<crate::mods::Mods>,
    /// The music bank tune playing, and the game's music volume (0-1).
    music: Option<Arc<super::music::Control>>,
    music_volume: f32,
    /// A tune waiting for the software synthesizer's SoundFont.
    #[cfg(not(windows))]
    pending_music: Option<(crate::midi::Song, u32, Arc<super::music::Control>)>,
}

impl Drop for AudioOut {
    fn drop(&mut self) {
        self.stop_music();
    }
}

fn open_device(mix: Arc<Mutex<Mix>>) -> anyhow::Result<(cpal::Stream, u32)> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no audio output device"))?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate.0;
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, mix)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, mix)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, mix)?,
        other => anyhow::bail!("unsupported output sample format {other:?}"),
    };
    stream.play()?;
    Ok((stream, rate))
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, mix: Arc<Mutex<Mix>>) -> anyhow::Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let ch = config.channels.max(1) as usize;
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / ch;
            let Ok(mut m) = mix.lock() else { return };
            m.render(frames);
            let gain = if m.muted { 0.0 } else { m.master };
            if gain != 1.0 {
                m.scratch.iter_mut().for_each(|s| *s *= gain);
            }
            for (f, out) in data.chunks_exact_mut(ch).enumerate() {
                let (l, r) = (m.scratch[f * 2].clamp(-1.0, 1.0), m.scratch[f * 2 + 1].clamp(-1.0, 1.0));
                if ch == 1 {
                    out[0] = T::from_sample((l + r) * 0.5);
                } else {
                    out[0] = T::from_sample(l);
                    out[1] = T::from_sample(r);
                    for o in &mut out[2..] {
                        *o = T::from_sample(0.0f32);
                    }
                }
            }
        },
        |e| eprintln!("audio output: {e}"),
        None,
    )?)
}

/// Decoder thread body: stream `bytes` into `out` from `start_ms`, repeating if `looped`.
fn feed(handle: u32, bytes: Arc<[u8]>, rate: u32, start_ms: f64, looped: bool, out: SyncSender<Vec<f32>>, report: Sender<Report>) {
    let mut resampler = Resampler::new(rate);
    let mut skip = (start_ms.max(0.0) * rate as f64 / 1000.0) as usize * 2;
    let mut first = true;
    loop {
        let mut stream = match Stream::open(bytes.clone()) {
            Ok(s) => s,
            Err(e) => {
                let _ = report.send(Report::Failed(handle, format!("{e:#}")));
                return;
            }
        };
        if first {
            if let Some(ms) = stream.duration_ms() {
                let _ = report.send(Report::Duration(handle, ms));
            }
        }
        let (mut src_frames, mut src_rate, mut any) = (0u64, 0u32, false);
        loop {
            let (r, ch, samples) = match stream.next_block() {
                Ok(Some(b)) => b,
                Ok(None) => break,
                Err(e) => {
                    if !any {
                        let _ = report.send(Report::Failed(handle, format!("{e:#}")));
                        return;
                    }
                    break;
                }
            };
            any = true;
            let ch = ch.max(1);
            src_rate = r;
            src_frames += (samples.len() / ch) as u64;
            let mut buf = Vec::with_capacity((samples.len() / ch) * 2 * rate as usize / r.max(1) as usize + 4);
            resampler.push(r, ch, &samples, &mut buf);
            if skip > 0 {
                let n = skip.min(buf.len());
                buf.drain(..n);
                skip -= n;
            }
            if !buf.is_empty() && out.send(buf).is_err() {
                return; // stopped
            }
        }
        if !any {
            let _ = report.send(Report::Failed(handle, "no audio decoded".into()));
            return;
        }
        if first && src_rate > 0 {
            let _ = report.send(Report::Duration(handle, src_frames as f64 / src_rate as f64 * 1000.0));
        }
        first = false;
        if !looped {
            return;
        }
    }
}

impl AudioOut {
    pub fn new(enabled: bool) -> Self {
        let (tx, rx) = channel();
        let mix = Arc::new(Mutex::new(Mix { master: 1.0, ..Mix::default() }));
        let (stream, rate) = if enabled {
            match open_device(mix.clone()) {
                Ok((s, r)) => (Some(s), r),
                Err(e) => {
                    eprintln!("audio disabled: {e:#}");
                    (None, crate::audio::OUTPUT_RATE)
                }
            }
        } else {
            (None, crate::audio::OUTPUT_RATE)
        };
        Self {
            mix,
            enabled: stream.is_some(),
            stream,
            rate,
            tx,
            rx,
            failed: HashSet::new(),
            mods: Default::default(),
            music: None,
            music_volume: 1.0,
            #[cfg(not(windows))]
            pending_music: None,
        }
    }

    pub fn muted(&self) -> bool {
        self.mix.lock().is_ok_and(|m| m.muted)
    }

    pub fn set_muted(&self, on: bool) {
        self.with_mix(|m| m.muted = on);
        self.refresh_music_gain();
    }

    /// Overall volume, 0-100.
    pub fn set_volume(&self, percent: u8) {
        self.with_mix(|m| m.master = percent.min(100) as f32 / 100.0);
        self.refresh_music_gain();
    }

    /// How loud music plays. The software synthesizer goes through the mixer (which applies
    /// the volume setting and mute); the Windows synthesizer doesn't.
    fn music_gain(&self) -> f32 {
        #[cfg(windows)]
        {
            let (master, muted) = self.mix.lock().map_or((1.0, false), |m| (m.master, m.muted));
            if muted {
                0.0
            } else {
                self.music_volume * master
            }
        }
        #[cfg(not(windows))]
        {
            self.music_volume
        }
    }

    fn refresh_music_gain(&self) {
        if let Some(c) = &self.music {
            c.set_gain(self.music_gain());
        }
    }

    fn stop_music(&mut self) {
        if let Some(c) = self.music.take() {
            c.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        #[cfg(not(windows))]
        {
            self.pending_music = None;
            self.with_mix(|m| m.music = None);
        }
    }

    fn play_music(&mut self, rt: &Runtime, handle: u32, loops: u32, volume: f32) {
        self.stop_music();
        self.music_volume = volume;
        let song = rt.game.music.get(handle).and_then(|e| e.data().ok()).and_then(|d| crate::midi::parse(&d));
        let Some(song) = song else {
            eprintln!("music {handle}: not a MIDI file");
            return;
        };
        let ctl = super::music::Control::new(self.music_gain());
        self.music = Some(ctl.clone());
        #[cfg(windows)]
        super::music::play_system(song, loops, ctl);
        #[cfg(not(windows))]
        {
            self.pending_music = Some((song, loops, ctl));
            self.start_pending_music();
        }
    }

    /// Start a waiting tune once the SoundFont is ready.
    #[cfg(not(windows))]
    fn start_pending_music(&mut self) {
        if self.pending_music.is_none() {
            return;
        }
        let Some(font) = super::music::soundfont() else { return };
        let Some((song, loops, ctl)) = self.pending_music.take() else { return };
        let rate = self.rate;
        if let Some(track) = super::music::SynthTrack::new(&font, rate, song, loops, ctl) {
            self.with_mix(|m| m.music = Some(track));
        }
    }

    /// Sounds currently playing.
    pub fn voices(&self) -> usize {
        self.mix.lock().map_or(0, |m| m.voices.len())
    }

    fn with_mix(&self, f: impl FnOnce(&mut Mix)) {
        if let Ok(mut m) = self.mix.lock() {
            f(&mut m);
        }
    }

    fn play(&mut self, rt: &Runtime, handle: u32, looped: bool, volume: f32, pan: f32, start_ms: f64) {
        self.with_mix(|m| m.voices.retain(|v| v.handle != handle));
        if self.failed.contains(&handle) {
            return;
        }
        if let Some(bytes) = rt.file_sound(handle) {
            let (out, rx) = sync_channel(AHEAD);
            self.with_mix(|m| m.voices.push(Voice { handle, rx, buf: Vec::new(), pos: 0, volume, pan }));
            let (report, rate) = (self.tx.clone(), self.rate);
            std::thread::spawn(move || feed(handle, bytes, rate, start_ms, looped, out, report));
            return;
        }
        let Some(entry) = rt.game.sounds.get(handle).cloned() else { return };
        let replacement = self.mods.sound(handle);
        let (out, rx) = sync_channel(AHEAD);
        self.with_mix(|m| m.voices.push(Voice { handle, rx, buf: Vec::new(), pos: 0, volume, pan }));
        let (report, rate) = (self.tx.clone(), self.rate);
        std::thread::spawn(move || {
            let bytes = match replacement.map(Ok).unwrap_or_else(|| entry.data()) {
                Ok(b) => Arc::<[u8]>::from(b),
                Err(e) => {
                    let _ = report.send(Report::Failed(handle, format!("{e:#}")));
                    return;
                }
            };
            feed(handle, bytes, rate, start_ms, looped, out, report);
        });
    }

    /// Run queued commands and pick up decoder reports. Call once per rendered frame.
    pub async fn process(&mut self, rt: &mut Runtime) {
        let commands = std::mem::take(&mut rt.audio.commands);
        if !self.enabled || self.stream.is_none() {
            return;
        }
        for cmd in commands {
            match cmd {
                AudioCommand::Play { handle, looped, volume, pan, start_ms } => self.play(rt, handle, looped, volume, pan, start_ms),
                AudioCommand::Stop { handle } => self.with_mix(|m| m.voices.retain(|v| v.handle != handle)),
                AudioCommand::StopAll => self.with_mix(|m| m.voices.clear()),
                AudioCommand::SetVolume { handle, volume } => self.with_mix(|m| {
                    for v in m.voices.iter_mut().filter(|v| v.handle == handle) {
                        v.volume = volume;
                    }
                }),
                AudioCommand::PlayMusic { handle, loops, volume } => self.play_music(rt, handle, loops, volume),
                AudioCommand::StopMusic => self.stop_music(),
                AudioCommand::PauseMusic(p) => {
                    if let Some(c) = &self.music {
                        c.paused.store(p, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                AudioCommand::MusicVolume(v) => {
                    self.music_volume = v;
                    self.refresh_music_gain();
                }
            }
        }
        #[cfg(not(windows))]
        self.start_pending_music();
        while let Ok(r) = self.rx.try_recv() {
            match r {
                Report::Duration(handle, ms) => {
                    rt.audio.durations.insert(handle, ms);
                }
                Report::Failed(handle, e) => {
                    if self.failed.insert(handle) {
                        eprintln!("sound {handle}: {e}");
                    }
                }
            }
        }
    }
}
