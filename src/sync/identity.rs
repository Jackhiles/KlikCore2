//! This device's identity and the devices it is paired with, kept in
//! `<data dir>/Rekindle/sync/`.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Folder for pairing and sync state.
pub fn dir() -> PathBuf {
    crate::runtime::files::data_root().join("sync")
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// This device.
#[derive(Debug, Clone)]
pub struct Identity {
    /// Random, 16 hex digits; stays the same for the lifetime of the data folder.
    pub id: String,
    /// Shown to other devices.
    pub name: String,
}

/// The computer's name, as other devices will see it.
pub fn device_name() -> String {
    if cfg!(target_os = "android") {
        return "Android device".into();
    }
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Rekindle device".into())
}

impl Identity {
    /// Read the identity, creating one on first use.
    pub fn load_or_create() -> Self {
        let file = dir().join("device.txt");
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).map(|v| v.trim().to_string());
        if let Some(id) = get("id").filter(|v| v.len() == 16) {
            return Self { id, name: get("name").filter(|n| !n.is_empty()).unwrap_or_else(device_name) };
        }
        let id = hex(&rand::random::<[u8; 8]>());
        let me = Self { id, name: device_name() };
        me.save();
        me
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(dir().join("device.txt"), format!("id={}\nname={}\n", self.id, self.name));
    }
}

/// A paired device. `key` is the secret both devices derived when pairing.
#[derive(Debug, Clone)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub key: [u8; 32],
    /// Where it was last reached (used when discovery hasn't seen it yet).
    pub last_addr: Option<SocketAddr>,
    /// Unix time (seconds) of the last successful sync.
    pub last_sync: u64,
}

fn peers_file() -> PathBuf {
    dir().join("paired.txt")
}

/// Paired devices. One per line: id, name, key (hex), last address, last sync.
pub fn load_peers() -> Vec<Peer> {
    let text = std::fs::read_to_string(peers_file()).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let key: [u8; 32] = unhex(f.get(2)?)?.try_into().ok()?;
            Some(Peer {
                id: f.first()?.to_string(),
                name: f.get(1)?.to_string(),
                key,
                last_addr: f.get(3).and_then(|a| a.parse().ok()),
                last_sync: f.get(4).and_then(|t| t.parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

pub fn save_peers(peers: &[Peer]) {
    let _ = std::fs::create_dir_all(dir());
    let text: String = peers
        .iter()
        .map(|p| {
            format!(
                "{}\t{}\t{}\t{}\t{}\n",
                p.id,
                p.name.replace(['\t', '\n'], " "),
                hex(&p.key),
                p.last_addr.map(|a| a.to_string()).unwrap_or_default(),
                p.last_sync
            )
        })
        .collect();
    let _ = std::fs::write(peers_file(), text);
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_round_trip() {
        let b = [0u8, 1, 0xab, 0xff];
        assert_eq!(super::unhex(&super::hex(&b)).unwrap(), b);
        assert!(super::unhex("abc").is_none());
    }
}
