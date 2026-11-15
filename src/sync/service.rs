//! The sync service: finds Rekindle on other devices of the local network, pairs with
//! them, keeps paired devices' saves in sync and copies game files between them. All
//! network work happens on background threads; the launcher reads and drives `Shared`.

use super::identity::{self, Identity, Peer};
use super::saves::{self, SaveEntry, Step};
use super::wire::{self, Channel, PairKeys};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DISCOVERY_PORT: u16 = 48631;
pub const SYNC_PORT: u16 = 48632;
const PAIR_TIMEOUT: Duration = Duration::from_secs(120);
const CHUNK: u64 = 4 * 1024 * 1024;

/// A device seen on the network.
#[derive(Debug, Clone)]
pub struct Nearby {
    pub id: String,
    pub name: String,
    pub addr: SocketAddr,
    /// "windows", "linux", "android", ...
    pub os: String,
    pub seen: Instant,
}

/// A pairing in progress (shown by the launcher).
#[derive(Debug, Clone)]
pub struct PairPrompt {
    pub peer_name: String,
    /// The other device asked (true) or we did.
    pub incoming: bool,
    /// The 6-digit code both screens show.
    pub code: Option<String>,
    /// The user's answer: Some(true) pair, Some(false) cancel.
    pub decision: Option<bool>,
    /// Outcome once finished.
    pub result: Option<Result<String, String>>,
}

/// A game on another device that can be copied here.
#[derive(Debug, Clone)]
pub struct RemoteGame {
    pub index: usize,
    pub title: String,
    pub size: u64,
    pub files: usize,
}

/// A game being copied here.
#[derive(Debug, Clone)]
pub struct Transfer {
    pub title: String,
    pub done: u64,
    pub total: u64,
    pub result: Option<Result<PathBuf, String>>,
}

#[derive(Debug, Default)]
pub struct Shared {
    pub peers: Vec<Peer>,
    pub nearby: HashMap<String, Nearby>,
    pub pair: Option<PairPrompt>,
    /// Save folders of games running here (never synced while running).
    pub running: Vec<String>,
    /// This device's library, offered to paired devices: (title, game file).
    pub library: Vec<(String, PathBuf)>,
    /// Per paired device: what the last sync did.
    pub status: HashMap<String, String>,
    /// Paired devices being synced right now.
    pub syncing: HashSet<String>,
    /// Short messages for the launcher to show (taken by it).
    pub messages: Vec<String>,
    /// Game lists fetched from paired devices.
    pub remote_library: HashMap<String, Result<Vec<RemoteGame>, String>>,
    pub transfers: Vec<Transfer>,
    /// Games copied here; the launcher adds them to its library (taken by it).
    pub new_games: Vec<PathBuf>,
    /// Addresses added by hand (devices on another subnet, which broadcasts don't reach).
    pub manual: Vec<IpAddr>,
}

/// Handle to the running service (cheap to clone).
#[derive(Clone)]
pub struct Service {
    pub me: Identity,
    pub port: u16,
    pub shared: Arc<Mutex<Shared>>,
    /// The discovery socket (also used to ask a typed address directly).
    udp: Option<Arc<UdpSocket>>,
}

fn manual_file() -> PathBuf {
    identity::dir().join("addresses.txt")
}

/// This device's address on the local network (the one used to reach the internet).
pub fn local_address() -> Option<IpAddr> {
    let s = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    s.connect(("192.0.2.1", 9)).ok()?; // nothing is sent
    s.local_addr().ok().map(|a| a.ip())
}

fn os_name() -> &'static str {
    std::env::consts::OS
}

fn lock(s: &Arc<Mutex<Shared>>) -> std::sync::MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

impl Service {
    /// Start listening and looking for other devices.
    pub fn start() -> Self {
        let me = Identity::load_or_create();
        let listener = TcpListener::bind(("0.0.0.0", SYNC_PORT)).or_else(|_| TcpListener::bind(("0.0.0.0", 0))).ok();
        let port = listener.as_ref().and_then(|l| l.local_addr().ok()).map_or(0, |a| a.port());
        let manual = std::fs::read_to_string(manual_file()).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect();
        let shared = Arc::new(Mutex::new(Shared { peers: identity::load_peers(), manual, ..Default::default() }));
        let udp = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).or_else(|_| UdpSocket::bind(("0.0.0.0", 0))).ok().map(Arc::new);
        let svc = Self { me, port, shared, udp };
        if let Some(l) = listener {
            let s = svc.clone();
            std::thread::spawn(move || s.serve(l));
        }
        let s = svc.clone();
        std::thread::spawn(move || s.discover());
        svc
    }

    // ------------------------------------------------------------ discovery

    /// Every few seconds ask "who's there?" on the local network; answer others' questions
    /// directly (answers are ordinary packets, which also reach phones that filter
    /// broadcasts).
    fn discover(&self) {
        let Some(sock) = self.udp.clone() else { return };
        let _ = sock.set_broadcast(true);
        let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
        let msg = |t: &str| json!({"t": t, "app": "rekindle", "v": 1, "id": self.me.id, "name": self.me.name, "port": self.port, "os": os_name()}).to_string();
        let mut last_query = Instant::now() - Duration::from_secs(60);
        let mut buf = [0u8; 2048];
        loop {
            if last_query.elapsed() > Duration::from_secs(3) {
                let _ = sock.send_to(msg("q").as_bytes(), ("255.255.255.255", DISCOVERY_PORT));
                // Devices broadcasts don't reach (another subnet): ask them directly.
                let direct: Vec<IpAddr> = {
                    let s = lock(&self.shared);
                    s.peers.iter().filter_map(|p| p.last_addr.map(|a| a.ip())).chain(s.manual.iter().copied()).collect()
                };
                for ip in direct {
                    let _ = sock.send_to(msg("q").as_bytes(), (ip, DISCOVERY_PORT));
                }
                last_query = Instant::now();
                let mut s = lock(&self.shared);
                s.nearby.retain(|_, n| n.seen.elapsed() < Duration::from_secs(15));
            }
            let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
            let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
            if v["app"] != "rekindle" || v["id"].as_str() == Some(self.me.id.as_str()) {
                continue;
            }
            if v["t"] == "q" {
                let _ = sock.send_to(msg("a").as_bytes(), from);
            }
            let (Some(id), Some(port)) = (v["id"].as_str(), v["port"].as_u64()) else { continue };
            let nearby = Nearby {
                id: id.to_string(),
                name: v["name"].as_str().unwrap_or("?").to_string(),
                addr: SocketAddr::new(from.ip(), port as u16),
                os: v["os"].as_str().unwrap_or("").to_string(),
                seen: Instant::now(),
            };
            lock(&self.shared).nearby.insert(id.to_string(), nearby);
        }
    }

    /// Where a paired device can be reached: seen on the network, else its last address.
    fn address_of(&self, peer: &str) -> Option<SocketAddr> {
        let s = lock(&self.shared);
        s.nearby.get(peer).map(|n| n.addr).or_else(|| s.peers.iter().find(|p| p.id == peer).and_then(|p| p.last_addr))
    }

    /// Look for Rekindle at an address typed by the user (remembered for next time).
    pub fn add_address(&self, ip: IpAddr) {
        {
            let mut s = lock(&self.shared);
            if !s.manual.contains(&ip) {
                s.manual.push(ip);
                let text: String = s.manual.iter().map(|a| format!("{a}\n")).collect();
                let _ = std::fs::create_dir_all(identity::dir());
                let _ = std::fs::write(manual_file(), text);
            }
        }
        if let Some(sock) = &self.udp {
            let q = json!({"t": "q", "app": "rekindle", "v": 1, "id": self.me.id, "name": self.me.name, "port": self.port, "os": os_name()}).to_string();
            let _ = sock.send_to(q.as_bytes(), (ip, DISCOVERY_PORT));
        }
    }

    pub fn is_online(&self, peer: &str) -> bool {
        lock(&self.shared).nearby.contains_key(peer)
    }

    // ------------------------------------------------------------ server

    fn serve(&self, listener: TcpListener) {
        for stream in listener.incoming().flatten() {
            let s = self.clone();
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
                if let Err(e) = s.handle(stream) {
                    eprintln!("sync: {e}");
                }
            });
        }
    }

    fn handle(&self, mut stream: TcpStream) -> wire::Result<()> {
        let first = wire::recv_plain(&mut stream)?;
        match first["t"].as_str() {
            Some("pair") => self.pair_incoming(stream, &first),
            Some("hello") => {
                let peers = lock(&self.shared).peers.clone();
                let mut ch = Channel::accept(stream, &first, &self.me.id, |id| peers.iter().find(|p| p.id == id).map(|p| p.key))?;
                loop {
                    let (req, body) = match ch.recv() {
                        Ok(m) => m,
                        Err(_) => return Ok(()), // closed
                    };
                    let (resp, data) = self.answer(&req, &body, ch.clock_offset_ms).unwrap_or_else(|e| (json!({"t": "error", "message": e}), Vec::new()));
                    ch.send(&resp, &data)?;
                }
            }
            _ => Err("unexpected first message".into()),
        }
    }

    /// One request from a paired device.
    fn answer(&self, req: &Value, body: &[u8], _offset: i64) -> Result<(Value, Vec<u8>), String> {
        let s = |k: &str| req[k].as_str().unwrap_or("").to_string();
        match req["t"].as_str().unwrap_or("") {
            "manifest" => {
                let only = req["game"].as_str();
                let files: Vec<Value> = saves::manifest(only).iter().map(SaveEntry::to_json).collect();
                let running = lock(&self.shared).running.clone();
                Ok((json!({"t": "manifest", "files": files, "running": running}), Vec::new()))
            }
            "get" => Ok((json!({"t": "file"}), saves::read(&s("game"), &s("path"))?)),
            "put" => {
                let game = s("game");
                if lock(&self.shared).running.contains(&game) {
                    return Err("that game is running on the other device".into());
                }
                saves::store(&game, &s("path"), body, req["mtime"].as_i64().unwrap_or(0), &saves::stamp())?;
                Ok((json!({"t": "ok"}), Vec::new()))
            }
            "del" => {
                let game = s("game");
                if lock(&self.shared).running.contains(&game) {
                    return Err("that game is running on the other device".into());
                }
                saves::remove(&game, &s("path"), &saves::stamp())?;
                Ok((json!({"t": "ok"}), Vec::new()))
            }
            "library" => {
                let lib = lock(&self.shared).library.clone();
                let games: Vec<Value> = lib
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, p))| p.is_file())
                    .map(|(i, (t, p))| {
                        let files = game_files(p).map(|(_, f, _)| f).unwrap_or_default();
                        json!({"index": i, "title": t, "size": files.iter().map(|f| f.1).sum::<u64>(), "files": files.len()})
                    })
                    .collect();
                Ok((json!({"t": "library", "games": games}), Vec::new()))
            }
            "game_files" => {
                let (_, files, main) = self.library_game(req)?;
                let list: Vec<Value> = files.iter().map(|(r, n)| json!({"path": r, "size": n})).collect();
                Ok((json!({"t": "game_files", "files": list, "main": main}), Vec::new()))
            }
            "game_chunk" => {
                let (base, files, _) = self.library_game(req)?;
                let rel = s("path");
                // Only files of that game, as listed.
                if !files.iter().any(|(r, _)| *r == rel) {
                    return Err("not a file of that game".into());
                }
                let mut f = std::fs::File::open(base.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))).map_err(|e| e.to_string())?;
                let offset = req["offset"].as_u64().unwrap_or(0);
                std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
                let mut buf = Vec::new();
                f.take(CHUNK).read_to_end(&mut buf).map_err(|e| e.to_string())?;
                Ok((json!({"t": "chunk"}), buf))
            }
            other => Err(format!("unknown request {other}")),
        }
    }

    fn library_game(&self, req: &Value) -> Result<(PathBuf, Vec<(String, u64)>, String), String> {
        let i = req["index"].as_u64().ok_or("no game index")? as usize;
        let path = lock(&self.shared).library.get(i).map(|(_, p)| p.clone()).ok_or("no such game")?;
        game_files(&path).ok_or_else(|| "the game's files can't be read".into())
    }

    // ------------------------------------------------------------ pairing

    /// Ask a device seen on the network to pair.
    pub fn pair_with(&self, nearby_id: &str) {
        let Some(n) = lock(&self.shared).nearby.get(nearby_id).cloned() else { return };
        lock(&self.shared).pair = Some(PairPrompt { peer_name: n.name.clone(), incoming: false, code: None, decision: None, result: None });
        let s = self.clone();
        std::thread::spawn(move || {
            let r = s.pair_outgoing(&n);
            s.finish_pair(r);
        });
    }

    /// The user's answer to the pairing prompt.
    pub fn answer_pair(&self, accept: bool) {
        if let Some(p) = lock(&self.shared).pair.as_mut() {
            if p.result.is_none() {
                p.decision = Some(accept);
            } else {
                // Dismissing a finished prompt.
            }
        }
    }

    pub fn dismiss_pair(&self) {
        let mut s = lock(&self.shared);
        if s.pair.as_ref().is_some_and(|p| p.result.is_some()) {
            s.pair = None;
        }
    }

    fn finish_pair(&self, r: Result<Peer, String>) {
        let mut s = lock(&self.shared);
        let result = match r {
            Ok(peer) => {
                let name = peer.name.clone();
                s.peers = identity::update_peers(|peers| {
                    peers.retain(|p| p.id != peer.id);
                    peers.push(peer);
                });
                Ok(name)
            }
            Err(e) => Err(e),
        };
        if let Some(p) = s.pair.as_mut() {
            p.result = Some(result);
        }
    }

    /// Wait for the local user's decision; meanwhile watch for the other side cancelling.
    fn wait_decision(&self, stream: &mut TcpStream, early: &mut Option<Value>) -> Result<bool, String> {
        let start = Instant::now();
        loop {
            if let Some(d) = lock(&self.shared).pair.as_ref().and_then(|p| p.decision) {
                return Ok(d);
            }
            if start.elapsed() > PAIR_TIMEOUT {
                return Err("pairing timed out".into());
            }
            if early.is_none() {
                let _ = stream.set_nonblocking(true);
                let mut b = [0u8; 1];
                let peeked = stream.peek(&mut b);
                let _ = stream.set_nonblocking(false);
                match peeked {
                    Ok(0) => return Err("the other device cancelled".into()),
                    Ok(_) => {
                        let v = wire::recv_plain(stream)?;
                        if v["t"] == "decline" {
                            return Err("the other device cancelled".into());
                        }
                        *early = Some(v);
                    }
                    Err(_) => {}
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn pair_outgoing(&self, n: &Nearby) -> Result<Peer, String> {
        let mut stream = TcpStream::connect_timeout(&n.addr, Duration::from_secs(5)).map_err(|e| format!("can't reach {}: {e}", n.name))?;
        let _ = stream.set_read_timeout(Some(PAIR_TIMEOUT));
        let keys = PairKeys::new();
        wire::send_plain(&mut stream, &json!({"t": "pair", "id": self.me.id, "name": self.me.name, "pub": identity::hex(&keys.public)}))?;
        let r = wire::recv_plain(&mut stream)?;
        if r["t"] == "decline" {
            return Err(format!("{} declined", n.name));
        }
        let their: [u8; 32] = identity::unhex(r["pub"].as_str().unwrap_or("")).and_then(|v| v.try_into().ok()).ok_or("bad answer")?;
        let (code, key) = keys.finish(&their, &keys.public, &their);
        if let Some(p) = lock(&self.shared).pair.as_mut() {
            p.code = Some(code);
        }
        let mut early = None;
        if !self.wait_decision(&mut stream, &mut early)? {
            let _ = wire::send_plain(&mut stream, &json!({"t": "decline"}));
            return Err("cancelled".into());
        }
        wire::send_plain(&mut stream, &json!({"t": "confirm", "proof": wire::confirm_proof(&key, "client")}))?;
        let v = match early {
            Some(v) => v,
            None => wire::recv_plain(&mut stream)?,
        };
        if v["t"] != "confirm" || v["proof"].as_str() != Some(wire::confirm_proof(&key, "server").as_str()) {
            return Err("the codes didn't match or the other device cancelled".into());
        }
        Ok(Peer { id: r["id"].as_str().unwrap_or(&n.id).to_string(), name: r["name"].as_str().unwrap_or(&n.name).to_string(), key, last_addr: Some(n.addr), last_sync: 0 })
    }

    fn pair_incoming(&self, mut stream: TcpStream, req: &Value) -> wire::Result<()> {
        let name = req["name"].as_str().unwrap_or("Another device").to_string();
        {
            let mut s = lock(&self.shared);
            if s.pair.as_ref().is_some_and(|p| p.result.is_none()) {
                drop(s);
                return wire::send_plain(&mut stream, &json!({"t": "decline"}));
            }
            s.pair = Some(PairPrompt { peer_name: name.clone(), incoming: true, code: None, decision: None, result: None });
        }
        let result = (|| -> Result<Peer, String> {
            let their: [u8; 32] = identity::unhex(req["pub"].as_str().unwrap_or("")).and_then(|v| v.try_into().ok()).ok_or("bad request")?;
            let keys = PairKeys::new();
            wire::send_plain(&mut stream, &json!({"t": "pair", "id": self.me.id, "name": self.me.name, "pub": identity::hex(&keys.public)}))?;
            let (code, key) = keys.finish(&their, &their, &keys.public);
            if let Some(p) = lock(&self.shared).pair.as_mut() {
                p.code = Some(code);
            }
            let mut early = None;
            if !self.wait_decision(&mut stream, &mut early)? {
                let _ = wire::send_plain(&mut stream, &json!({"t": "decline"}));
                return Err("cancelled".into());
            }
            wire::send_plain(&mut stream, &json!({"t": "confirm", "proof": wire::confirm_proof(&key, "server")}))?;
            let v = match early {
                Some(v) => v,
                None => wire::recv_plain(&mut stream)?,
            };
            if v["t"] != "confirm" || v["proof"].as_str() != Some(wire::confirm_proof(&key, "client").as_str()) {
                return Err("the codes didn't match or the other device cancelled".into());
            }
            let addr = stream.peer_addr().ok().map(|a| SocketAddr::new(a.ip(), SYNC_PORT));
            Ok(Peer { id: req["id"].as_str().unwrap_or("").to_string(), name, key, last_addr: addr, last_sync: 0 })
        })();
        self.finish_pair(result);
        Ok(())
    }

    pub fn unpair(&self, peer: &str) {
        let mut s = lock(&self.shared);
        s.peers = identity::update_peers(|peers| peers.retain(|p| p.id != peer));
        s.status.remove(peer);
        let _ = std::fs::remove_file(identity::dir().join(format!("state-{peer}.txt")));
    }

    // ------------------------------------------------------------ client side

    fn open(&self, peer: &str) -> Result<Channel, String> {
        let (key, name) = {
            let s = lock(&self.shared);
            let p = s.peers.iter().find(|p| p.id == peer).ok_or("not paired")?;
            (p.key, p.name.clone())
        };
        let addr = self.address_of(peer).ok_or_else(|| format!("{name} isn't on the network"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(4)).map_err(|_| format!("{name} isn't reachable"))?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
        let ch = Channel::connect(stream, &self.me.id, peer, &key)?;
        let mut s = lock(&self.shared);
        if let Some(p) = s.peers.iter_mut().find(|p| p.id == peer) {
            p.last_addr = Some(addr);
        }
        Ok(ch)
    }

    /// Sync saves with every paired device that's on the network (or one game's only).
    pub fn sync_all(&self, only_game: Option<String>) {
        let peers: Vec<String> = {
            let s = lock(&self.shared);
            s.peers.iter().filter(|p| s.nearby.contains_key(&p.id)).map(|p| p.id.clone()).collect()
        };
        for p in peers {
            self.sync_with(&p, only_game.clone());
        }
    }

    /// Sync saves with one paired device, in the background.
    pub fn sync_with(&self, peer: &str, only_game: Option<String>) {
        if !lock(&self.shared).syncing.insert(peer.to_string()) {
            return;
        }
        let s = self.clone();
        let peer = peer.to_string();
        std::thread::spawn(move || {
            let r = s.sync_inner(&peer, only_game.as_deref());
            let mut sh = lock(&s.shared);
            sh.syncing.remove(&peer);
            let name = sh.peers.iter().find(|p| p.id == peer).map(|p| p.name.clone()).unwrap_or_default();
            let msg = match &r {
                Ok((0, 0)) => "Up to date".to_string(),
                Ok((got, sent)) => {
                    let m = format!("{got} received, {sent} sent");
                    sh.messages.push(format!("Saves synced with {name}: {m}"));
                    m
                }
                Err(e) => e.clone(),
            };
            sh.status.insert(peer.clone(), msg);
            if r.is_ok() {
                let addr = sh.peers.iter().find(|p| p.id == peer).and_then(|p| p.last_addr);
                sh.peers = identity::update_peers(|peers| {
                    if let Some(p) = peers.iter_mut().find(|p| p.id == peer) {
                        p.last_sync = identity::now_secs();
                        if addr.is_some() {
                            p.last_addr = addr;
                        }
                    }
                });
            }
        });
    }

    /// (files received, files sent)
    fn sync_inner(&self, peer: &str, only_game: Option<&str>) -> Result<(usize, usize), String> {
        let mut ch = self.open(peer)?;
        let (m, _) = ch.call(&json!({"t": "manifest", "game": only_game}), &[])?;
        let remote: Vec<SaveEntry> = m["files"].as_array().map(|a| a.iter().filter_map(SaveEntry::from_json).collect()).unwrap_or_default();
        let mut skip: Vec<String> = lock(&self.shared).running.clone();
        skip.extend(m["running"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)));
        let local = saves::manifest(only_game);
        let mut state = saves::load_state(peer);
        let steps = saves::plan(&local, &remote, &state, ch.clock_offset_ms, &skip);
        let stamp = saves::stamp();
        let (mut got, mut sent) = (0, 0);
        for step in &steps {
            match step {
                Step::Pull(e) => {
                    let (_, data) = ch.call(&json!({"t": "get", "game": e.game, "path": e.path}), &[])?;
                    saves::store(&e.game, &e.path, &data, e.mtime_ms - ch.clock_offset_ms, &stamp)?;
                    state.insert(e.key(), e.hash.clone());
                    got += 1;
                }
                Step::Push(e) => {
                    let data = saves::read(&e.game, &e.path)?;
                    ch.call(&json!({"t": "put", "game": e.game, "path": e.path, "mtime": e.mtime_ms + ch.clock_offset_ms}), &data)?;
                    state.insert(e.key(), e.hash.clone());
                    sent += 1;
                }
                Step::DeleteHere(e) => {
                    saves::remove(&e.game, &e.path, &stamp)?;
                    state.remove(&e.key());
                    got += 1;
                }
                Step::DeleteThere(e) => {
                    ch.call(&json!({"t": "del", "game": e.game, "path": e.path}), &[])?;
                    state.remove(&e.key());
                    sent += 1;
                }
            }
        }
        // Files that were already equal also count as synced.
        let theirs: HashMap<String, String> = remote.iter().map(|e| (e.key(), e.hash.clone())).collect();
        for e in &local {
            if theirs.get(&e.key()) == Some(&e.hash) {
                state.insert(e.key(), e.hash.clone());
            }
        }
        saves::save_state(peer, &state);
        Ok((got, sent))
    }

    /// Fetch a paired device's game list (into `remote_library`).
    pub fn request_library(&self, peer: &str) {
        let s = self.clone();
        let peer = peer.to_string();
        lock(&self.shared).remote_library.remove(&peer);
        std::thread::spawn(move || {
            let r = (|| -> Result<Vec<RemoteGame>, String> {
                let mut ch = s.open(&peer)?;
                let (v, _) = ch.call(&json!({"t": "library"}), &[])?;
                Ok(v["games"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|g| {
                        Some(RemoteGame {
                            index: g["index"].as_u64()? as usize,
                            title: g["title"].as_str()?.to_string(),
                            size: g["size"].as_u64().unwrap_or(0),
                            files: g["files"].as_u64().unwrap_or(0) as usize,
                        })
                    })
                    .collect())
            })();
            lock(&s.shared).remote_library.insert(peer, r);
        });
    }

    /// Copy a game from a paired device into this device's games folder.
    pub fn fetch_game(&self, peer: &str, game: &RemoteGame) {
        let s = self.clone();
        let peer = peer.to_string();
        let game = game.clone();
        let slot = {
            let mut sh = lock(&self.shared);
            sh.transfers.push(Transfer { title: game.title.clone(), done: 0, total: game.size, result: None });
            sh.transfers.len() - 1
        };
        std::thread::spawn(move || {
            let r = s.fetch_inner(&peer, &game, slot);
            let mut sh = lock(&s.shared);
            if let Ok(p) = &r {
                sh.new_games.push(p.clone());
                sh.messages.push(format!("{} copied to this device", game.title));
            }
            if let Some(t) = sh.transfers.get_mut(slot) {
                t.result = Some(r);
            }
        });
    }

    fn fetch_inner(&self, peer: &str, game: &RemoteGame, slot: usize) -> Result<PathBuf, String> {
        let mut ch = self.open(peer)?;
        let (v, _) = ch.call(&json!({"t": "game_files", "index": game.index}), &[])?;
        let main = v["main"].as_str().ok_or("no main file")?.to_string();
        let safe: String = game.title.chars().map(|c| if c.is_alphanumeric() || " -_.'()".contains(c) { c } else { '_' }).collect();
        let dest = games_dir().join(safe.trim());
        for f in v["files"].as_array().into_iter().flatten() {
            let rel = f["path"].as_str().ok_or("bad file list")?.to_string();
            let size = f["size"].as_u64().unwrap_or(0);
            let rel_path = Path::new(&rel);
            if !rel_path.components().all(|c| matches!(c, std::path::Component::Normal(_))) || rel.contains(['\\', ':']) {
                return Err("unsafe file name from the other device".into());
            }
            let target = dest.join(rel_path);
            if let Some(d) = target.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            // Already here with the same size (an earlier, interrupted copy): skip.
            if std::fs::metadata(&target).is_ok_and(|m| m.len() == size) {
                if let Some(t) = lock(&self.shared).transfers.get_mut(slot) {
                    t.done += size;
                }
                continue;
            }
            let part = target.with_extension("part");
            let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let mut offset = 0u64;
            while offset < size || (size == 0 && offset == 0) {
                let (_, data) = ch.call(&json!({"t": "game_chunk", "index": game.index, "path": rel, "offset": offset}), &[])?;
                if data.is_empty() {
                    break;
                }
                std::io::Write::write_all(&mut out, &data).map_err(|e| e.to_string())?;
                offset += data.len() as u64;
                if let Some(t) = lock(&self.shared).transfers.get_mut(slot) {
                    t.done += data.len() as u64;
                }
            }
            drop(out);
            std::fs::rename(&part, &target).map_err(|e| e.to_string())?;
        }
        Ok(dest.join(main))
    }
}

/// Where games copied from other devices go.
pub fn games_dir() -> PathBuf {
    crate::runtime::files::data_root().join("games")
}

fn is_game_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| ["exe", "ccn"].iter().any(|g| e.eq_ignore_ascii_case(g)))
}

/// The files that make up a game, to copy it to another device:
/// (folder, [(path inside it with '/', size)], main file's path). A folder holding only
/// this game is copied whole (data folders and all); in a folder of several games, only
/// the game's own files (same name, e.g. `game.exe` + `game.dat`).
pub fn game_files(game: &Path) -> Option<(PathBuf, Vec<(String, u64)>, String)> {
    let folder = game.parent()?.to_path_buf();
    let main = game.file_name()?.to_string_lossy().into_owned();
    let stem = game.file_stem()?.to_string_lossy().to_lowercase();
    let others = std::fs::read_dir(&folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_game_file(p) && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() != stem))
        .count();
    let mut files = Vec::new();
    if others == 0 {
        let mut stack = vec![folder.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).ok()?.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let (Ok(rel), Ok(m)) = (p.strip_prefix(&folder), e.metadata()) {
                    let rel = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
                    files.push((rel, m.len()));
                }
            }
        }
    } else {
        for e in std::fs::read_dir(&folder).ok()?.flatten() {
            let p = e.path();
            if p.is_file() && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == stem) {
                files.push((e.file_name().to_string_lossy().into_owned(), e.metadata().map_or(0, |m| m.len())));
            }
        }
    }
    files.sort();
    Some((folder, files, main))
}
