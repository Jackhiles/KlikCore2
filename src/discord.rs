//! Discord Rich Presence: while a game runs, the player's Discord status shows
//! "Playing KlikCore 2" with the game's title and how long it has been running.
//!
//! This talks to the Discord desktop app on the same computer over its local IPC
//! (`\\.\pipe\discord-ipc-N` on Windows, a `discord-ipc-N` socket on Linux); nothing goes
//! over the network from here and there's no login. If Discord isn't running, KlikCore
//! tries again now and then. Off with `discord=off` in `launcher.ini` (the launcher's
//! **...** menu) or `KLIKCORE_DISCORD=0`.
//!
//! Discord shows the name of the Discord application whose id is used ([`CLIENT_ID`]).

use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// The "KlikCore 2" application on Discord's developer portal. Empty = not set up (Rich
/// Presence stays off); `KLIKCORE_DISCORD_CLIENT_ID` overrides it.
const CLIENT_ID: &str = "1571734954315096094";
/// Pictures (Discord takes https links for them).
const ICON_URL: &str = "https://takobyte.net/assets/img/klikcore/icon.png";

/// What to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    /// The game's title.
    pub details: String,
    /// When play started (Unix seconds): Discord counts up from it.
    pub start: i64,
    /// A picture of the game (an https link), if there is one.
    pub image: Option<String>,
}

/// Keeps the status shown while alive; dropping it (the game ends) clears it.
pub struct Presence {
    _tx: Sender<Activity>,
}

fn client_id() -> Option<String> {
    let id = std::env::var("KLIKCORE_DISCORD_CLIENT_ID").unwrap_or_else(|_| CLIENT_ID.to_string());
    (!id.trim().is_empty()).then(|| id.trim().to_string())
}

/// Is Rich Presence set up in this build (and possible on this system)?
pub fn available() -> bool {
    !cfg!(target_os = "android") && client_id().is_some()
}

/// Is Rich Presence turned on (and set up)?
pub fn enabled() -> bool {
    if matches!(std::env::var("KLIKCORE_DISCORD").as_deref(), Ok("0" | "off")) || cfg!(target_os = "android") {
        return false;
    }
    let ini = crate::runtime::files::data_root().join("launcher.ini");
    let off = std::fs::read_to_string(ini).is_ok_and(|t| t.lines().any(|l| l.trim() == "discord=off"));
    !off && client_id().is_some()
}

/// Show `activity` while the returned value lives (None when off or not set up).
pub fn start(activity: Activity) -> Option<Presence> {
    if !enabled() {
        return None;
    }
    let id = client_id()?;
    let (tx, rx) = channel();
    std::thread::Builder::new().name("discord".into()).spawn(move || run(&id, activity, rx)).ok()?;
    Some(Presence { _tx: tx })
}

/// Connect (retrying while Discord isn't running) and keep the activity set until the
/// game ends (the sender is dropped).
fn run(id: &str, mut activity: Activity, rx: Receiver<Activity>) {
    loop {
        let debug = std::env::var("KLIKCORE_DISCORD").is_ok_and(|v| v == "debug");
        if let Some(mut conn) = connect() {
            let shown = handshake(&mut conn, id).and_then(|_| set_activity(&mut conn, &activity));
            if debug {
                eprintln!("discord: {shown:?}");
            }
            if shown.is_ok() {
                // Wait for changes; a failed write means Discord went away: reconnect.
                loop {
                    match rx.recv_timeout(Duration::from_secs(30)) {
                        Ok(a) => {
                            activity = a;
                            if set_activity(&mut conn, &activity).is_err() {
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        } else if debug {
            eprintln!("discord: not running");
        }
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(a) => activity = a,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

trait Conn: Read + Write {}
impl<T: Read + Write> Conn for T {}

fn connect() -> Option<Box<dyn Conn>> {
    #[cfg(windows)]
    for n in 0..10 {
        if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{n}")) {
            return Some(Box::new(f));
        }
    }
    #[cfg(unix)]
    {
        let mut bases: Vec<std::path::PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(|v| std::env::var_os(v)).map(Into::into).collect();
        bases.push("/tmp".into());
        // The plain app, then Flatpak and Snap installs.
        for base in bases {
            for sub in ["", "app/com.discordapp.Discord", "snap.discord", ".flatpak/dev.vencord.Vesktop/xdg-run"] {
                for n in 0..10 {
                    if let Ok(s) = std::os::unix::net::UnixStream::connect(base.join(sub).join(format!("discord-ipc-{n}"))) {
                        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                        return Some(Box::new(s));
                    }
                }
            }
        }
    }
    None
}

/// One IPC frame: opcode, length, JSON.
fn send(conn: &mut dyn Conn, op: u32, json: &serde_json::Value) -> std::io::Result<()> {
    let body = json.to_string();
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body.as_bytes());
    conn.write_all(&frame)?;
    conn.flush()
}

fn receive(conn: &mut dyn Conn) -> std::io::Result<(u32, serde_json::Value)> {
    let mut head = [0u8; 8];
    conn.read_exact(&mut head)?;
    let op = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    if len > 1 << 20 {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut body = vec![0u8; len];
    conn.read_exact(&mut body)?;
    Ok((op, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)))
}

fn handshake(conn: &mut dyn Conn, id: &str) -> std::io::Result<()> {
    send(conn, 0, &serde_json::json!({ "v": 1, "client_id": id }))?;
    // READY (opcode 1), or a close (opcode 2) with the reason (e.g. a wrong id).
    let (op, msg) = receive(conn)?;
    if op == 1 && msg["evt"] == "READY" {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("Discord refused: {msg}")))
    }
}

fn activity_json(a: &Activity, pid: u32, nonce: u64) -> serde_json::Value {
    let mut assets = serde_json::json!({ "large_image": ICON_URL, "large_text": "KlikCore 2" });
    if let Some(img) = &a.image {
        assets = serde_json::json!({
            "large_image": img, "large_text": a.details,
            "small_image": ICON_URL, "small_text": "KlikCore 2",
        });
    }
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": {
                "details": a.details,
                "timestamps": { "start": a.start },
                "assets": assets,
                "type": 0,
            },
        },
        "nonce": nonce.to_string(),
    })
}

/// Set the activity; returns Discord's reply (an error reply means it was refused).
fn set_activity(conn: &mut dyn Conn, a: &Activity) -> std::io::Result<serde_json::Value> {
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    send(conn, 1, &activity_json(a, std::process::id(), nonce))?;
    let (_, reply) = receive(conn)?;
    if reply["evt"] == "ERROR" {
        return Err(std::io::Error::other(reply["data"]["message"].as_str().unwrap_or("refused").to_string()));
    }
    Ok(reply["data"]["details"].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_message() {
        let a = Activity { details: "Some Game".into(), start: 1_700_000_000, image: None };
        let j = activity_json(&a, 42, 7);
        assert_eq!(j["cmd"], "SET_ACTIVITY");
        assert_eq!(j["args"]["pid"], 42);
        assert_eq!(j["args"]["activity"]["details"], "Some Game");
        assert_eq!(j["args"]["activity"]["timestamps"]["start"], 1_700_000_000);
        assert_eq!(j["args"]["activity"]["assets"]["large_image"], ICON_URL);
        let a = Activity { image: Some("https://example.invalid/x.jpg".into()), ..a };
        let j = activity_json(&a, 42, 8);
        assert_eq!(j["args"]["activity"]["assets"]["small_image"], ICON_URL);
    }

    #[test]
    fn frames_round_trip() {
        // A fake connection: what's written can be read back.
        struct Loop(std::collections::VecDeque<u8>);
        impl Read for Loop {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = buf.len().min(self.0.len());
                for b in buf.iter_mut().take(n) {
                    *b = self.0.pop_front().unwrap();
                }
                Ok(n)
            }
        }
        impl Write for Loop {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.extend(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut c = Loop(Default::default());
        send(&mut c, 1, &serde_json::json!({"evt": "READY"})).unwrap();
        let (op, v) = receive(&mut c).unwrap();
        assert_eq!((op, v["evt"].as_str()), (1, Some("READY")));
    }
}
