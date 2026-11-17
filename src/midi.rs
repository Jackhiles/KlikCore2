//! Standard MIDI files (the music bank of older games): read into a list of timed channel
//! messages, for the music player (`render::music`).

/// A song: channel messages (status, data 1, data 2) at times in seconds, sorted.
#[derive(Debug, Clone, Default)]
pub struct Song {
    pub events: Vec<(f64, [u8; 3])>,
    /// Seconds until the last event.
    pub duration: f64,
}

fn u16_be(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}

fn u32_be(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// A variable-length quantity at `*i`.
fn varlen(b: &[u8], i: &mut usize) -> Option<u32> {
    let mut v = 0u32;
    for _ in 0..4 {
        let c = *b.get(*i)?;
        *i += 1;
        v = (v << 7) | (c & 0x7F) as u32;
        if c & 0x80 == 0 {
            return Some(v);
        }
    }
    Some(v)
}

/// Read a MIDI file (format 0, 1 or 2; tempo changes; running status). None if it isn't one.
pub fn parse(data: &[u8]) -> Option<Song> {
    if !data.starts_with(b"MThd") {
        return None;
    }
    let header_len = u32_be(data, 4)? as usize;
    let tracks = u16_be(data, 10)? as usize;
    let division = u16_be(data, 12)?;
    // Ticks per quarter note, or (SMPTE) a fixed number of ticks per second.
    let smpte_tps = (division & 0x8000 != 0).then(|| {
        let fps = 256 - (division >> 8) as u32;
        (fps * (division & 0xFF) as u32).max(1) as f64
    });
    let tpq = (division & 0x7FFF).max(1) as f64;

    // (tick, order, kind): kind = Ok(message) or Err(tempo in microseconds per quarter).
    let mut all: Vec<(u64, usize, Result<[u8; 3], u32>)> = Vec::new();
    let mut p = 8 + header_len;
    for _ in 0..tracks {
        while p + 8 <= data.len() && &data[p..p + 4] != b"MTrk" {
            // Skip unknown chunks.
            p += 8 + u32_be(data, p + 4)? as usize;
        }
        if p + 8 > data.len() {
            break;
        }
        let len = u32_be(data, p + 4)? as usize;
        let t = data.get(p + 8..(p + 8 + len).min(data.len()))?;
        p += 8 + len;
        let (mut i, mut tick, mut status) = (0usize, 0u64, 0u8);
        while i < t.len() {
            tick += varlen(t, &mut i)? as u64;
            let mut b = *t.get(i)?;
            if b & 0x80 != 0 {
                i += 1;
                if b < 0xF0 {
                    status = b;
                }
            } else {
                // Running status: this byte is the first data byte.
                b = status;
            }
            match b {
                0xFF => {
                    let kind = *t.get(i)?;
                    i += 1;
                    let n = varlen(t, &mut i)? as usize;
                    if kind == 0x51 && n >= 3 {
                        let us = u32::from_be_bytes([0, t[i], t[i + 1], t[i + 2]]);
                        all.push((tick, all.len(), Err(us)));
                    }
                    if kind == 0x2F {
                        break;
                    }
                    i += n;
                }
                0xF0 | 0xF7 => {
                    let n = varlen(t, &mut i)? as usize;
                    i += n;
                }
                0x80..=0xEF => {
                    let two = !matches!(b & 0xF0, 0xC0 | 0xD0);
                    let d1 = *t.get(i)?;
                    let d2 = if two { *t.get(i + 1)? } else { 0 };
                    i += 1 + two as usize;
                    all.push((tick, all.len(), Ok([b, d1, d2])));
                }
                _ => break,
            }
        }
    }
    all.sort_by_key(|e| (e.0, e.1));

    let mut song = Song::default();
    let (mut us_per_q, mut last_tick, mut secs) = (500_000f64, 0u64, 0f64);
    for (tick, _, e) in all {
        let dt = (tick - last_tick) as f64;
        secs += match smpte_tps {
            Some(tps) => dt / tps,
            None => dt * us_per_q / tpq / 1_000_000.0,
        };
        last_tick = tick;
        match e {
            Ok(m) => song.events.push((secs, m)),
            Err(us) => us_per_q = us.max(1) as f64,
        }
    }
    song.duration = secs;
    Some(song)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tempo_running_status_and_length() {
        // Format 0, 96 ticks per quarter; tempo 1 s per quarter; note on C4, then (running
        // status) its note off one quarter later; end of track.
        let mut f = b"MThd".to_vec();
        f.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 96]);
        let track = [
            0x00, 0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40, // tempo 1,000,000 us
            0x00, 0x90, 60, 100, // note on
            0x60, 60, 0, // 96 ticks later, running status: velocity 0
            0x00, 0xFF, 0x2F, 0x00,
        ];
        f.extend_from_slice(b"MTrk");
        f.extend_from_slice(&(track.len() as u32).to_be_bytes());
        f.extend_from_slice(&track);
        let s = parse(&f).unwrap();
        assert_eq!(s.events, vec![(0.0, [0x90, 60, 100]), (1.0, [0x90, 60, 0])]);
        assert!((s.duration - 1.0).abs() < 1e-9);
        assert!(parse(b"RIFF....").is_none());
    }
}
