//! Messages between devices: length-prefixed frames over TCP, a JSON header plus an
//! optional binary body, encrypted with ChaCha20-Poly1305 once two paired devices have
//! proven they share their pairing key.
//!
//! Handshake (paired): the client sends `hello {id, nonce, time}` in the clear, the server
//! answers the same if it knows the client. Both derive per-direction keys with HKDF from
//! the pairing key and the two nonces, then exchange an encrypted `auth` message: a device
//! without the key can't produce or read it.
//!
//! Pairing: both send an X25519 public key; each screen shows a 6-digit code derived from
//! the shared secret and both public keys. Someone intercepting the connection can't make
//! the two codes match, so the user comparing them is what makes pairing safe.

use super::identity::{hex, unhex};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpStream;

/// Largest frame accepted (a save file or a game file chunk plus its header).
const MAX_FRAME: usize = 16 * 1024 * 1024;

pub type Result<T> = std::result::Result<T, String>;

fn io<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub fn write_frame(s: &mut TcpStream, data: &[u8]) -> Result<()> {
    s.write_all(&(data.len() as u32).to_le_bytes()).map_err(io)?;
    s.write_all(data).map_err(io)
}

pub fn read_frame(s: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).map_err(io)?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(format!("frame too large ({n} bytes)"));
    }
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf).map_err(io)?;
    Ok(buf)
}

/// JSON header + body in one buffer: `u32 header length, header, body`.
pub fn pack(header: &Value, body: &[u8]) -> Vec<u8> {
    let h = header.to_string().into_bytes();
    let mut out = Vec::with_capacity(4 + h.len() + body.len());
    out.extend_from_slice(&(h.len() as u32).to_le_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(body);
    out
}

pub fn unpack(buf: &[u8]) -> Result<(Value, Vec<u8>)> {
    let n = u32::from_le_bytes(buf.get(..4).ok_or("short message")?.try_into().map_err(io)?) as usize;
    let h = buf.get(4..4 + n).ok_or("short message")?;
    let v: Value = serde_json::from_slice(h).map_err(io)?;
    Ok((v, buf[4 + n..].to_vec()))
}

/// Plain (unencrypted) message, for the first step of a handshake.
pub fn send_plain(s: &mut TcpStream, v: &Value) -> Result<()> {
    write_frame(s, &pack(v, &[]))
}

pub fn recv_plain(s: &mut TcpStream) -> Result<Value> {
    Ok(unpack(&read_frame(s)?)?.0)
}

/// An encrypted connection to a paired device.
pub struct Channel {
    pub stream: TcpStream,
    send: ChaCha20Poly1305,
    recv: ChaCha20Poly1305,
    send_n: u64,
    recv_n: u64,
    /// The other device's id.
    pub peer: String,
    /// Their clock minus ours, in ms (for comparing file times).
    pub clock_offset_ms: i64,
}

fn nonce(n: u64) -> Nonce {
    let mut b = [0u8; 12];
    b[..8].copy_from_slice(&n.to_le_bytes());
    *Nonce::from_slice(&b)
}

fn session_keys(pair_key: &[u8; 32], client_nonce: &[u8], server_nonce: &[u8]) -> ([u8; 32], [u8; 32]) {
    let salt = [client_nonce, server_nonce].concat();
    let hk = Hkdf::<Sha256>::new(Some(&salt), pair_key);
    let (mut c2s, mut s2c) = ([0u8; 32], [0u8; 32]);
    hk.expand(b"rekindle sync c2s", &mut c2s).expect("32 bytes is a valid length");
    hk.expand(b"rekindle sync s2c", &mut s2c).expect("32 bytes is a valid length");
    (c2s, s2c)
}

impl Channel {
    fn new(stream: TcpStream, send_key: [u8; 32], recv_key: [u8; 32], peer: String, clock_offset_ms: i64) -> Self {
        Self {
            stream,
            send: ChaCha20Poly1305::new(Key::from_slice(&send_key)),
            recv: ChaCha20Poly1305::new(Key::from_slice(&recv_key)),
            send_n: 0,
            recv_n: 0,
            peer,
            clock_offset_ms,
        }
    }

    pub fn send(&mut self, header: &Value, body: &[u8]) -> Result<()> {
        let ct = self.send.encrypt(&nonce(self.send_n), pack(header, body).as_slice()).map_err(|_| "encryption failed")?;
        self.send_n += 1;
        write_frame(&mut self.stream, &ct)
    }

    pub fn recv(&mut self) -> Result<(Value, Vec<u8>)> {
        let ct = read_frame(&mut self.stream)?;
        let pt = self.recv.decrypt(&nonce(self.recv_n), ct.as_slice()).map_err(|_| "message failed authentication")?;
        self.recv_n += 1;
        unpack(&pt)
    }

    /// Send a request and wait for its answer; an `error` answer becomes Err.
    pub fn call(&mut self, header: &Value, body: &[u8]) -> Result<(Value, Vec<u8>)> {
        self.send(header, body)?;
        let (v, b) = self.recv()?;
        if v["t"] == "error" {
            return Err(v["message"].as_str().unwrap_or("the other device reported an error").to_string());
        }
        Ok((v, b))
    }

    /// Client side of the paired handshake. `expect` is the device we meant to reach.
    pub fn connect(mut stream: TcpStream, my_id: &str, expect: &str, key: &[u8; 32]) -> Result<Self> {
        let my_nonce: [u8; 32] = rand::random();
        let sent_at = super::identity::now_ms();
        send_plain(&mut stream, &json!({"t": "hello", "v": 1, "id": my_id, "nonce": hex(&my_nonce), "time": sent_at}))?;
        let r = recv_plain(&mut stream)?;
        if r["t"] != "hello" {
            return Err(match r["t"].as_str() {
                Some("unknown") => "the other device doesn't know this one (pair again)".into(),
                _ => "unexpected answer".into(),
            });
        }
        if r["id"].as_str() != Some(expect) {
            return Err("a different device answered".into());
        }
        let their_nonce = unhex(r["nonce"].as_str().unwrap_or("")).filter(|n| n.len() == 32).ok_or("bad nonce")?;
        let now = super::identity::now_ms();
        let offset = r["time"].as_i64().map_or(0, |t| t - (sent_at + now) / 2);
        let (c2s, s2c) = session_keys(key, &my_nonce, &their_nonce);
        let mut ch = Channel::new(stream, c2s, s2c, expect.to_string(), offset);
        ch.send(&json!({"t": "auth"}), &[])?;
        let (a, _) = ch.recv()?;
        if a["t"] != "auth" {
            return Err("the other device failed to prove its key".into());
        }
        Ok(ch)
    }

    /// Server side, after the client's `hello` was read. `lookup` gives the key for an id.
    pub fn accept(mut stream: TcpStream, hello: &Value, my_id: &str, lookup: impl Fn(&str) -> Option<[u8; 32]>) -> Result<Self> {
        let id = hello["id"].as_str().unwrap_or("").to_string();
        let Some(key) = lookup(&id) else {
            let _ = send_plain(&mut stream, &json!({"t": "unknown"}));
            return Err(format!("unknown device {id}"));
        };
        let their_nonce = unhex(hello["nonce"].as_str().unwrap_or("")).filter(|n| n.len() == 32).ok_or("bad nonce")?;
        let my_nonce: [u8; 32] = rand::random();
        let now = super::identity::now_ms();
        send_plain(&mut stream, &json!({"t": "hello", "v": 1, "id": my_id, "nonce": hex(&my_nonce), "time": now}))?;
        let offset = hello["time"].as_i64().map_or(0, |t| t - now);
        let (c2s, s2c) = session_keys(&key, &their_nonce, &my_nonce);
        let mut ch = Channel::new(stream, s2c, c2s, id, offset);
        let (a, _) = ch.recv()?;
        if a["t"] != "auth" {
            return Err("the other device failed to prove its key".into());
        }
        ch.send(&json!({"t": "auth"}), &[])?;
        Ok(ch)
    }
}

/// Pairing secrets for one attempt.
pub struct PairKeys {
    secret: x25519_dalek::StaticSecret,
    pub public: [u8; 32],
}

impl PairKeys {
    pub fn new() -> Self {
        let secret = x25519_dalek::StaticSecret::from(rand::random::<[u8; 32]>());
        let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
        Self { secret, public }
    }

    /// (6-digit code to compare, pairing key to keep). `client_pub` / `server_pub` in
    /// that order on both devices.
    pub fn finish(&self, their_pub: &[u8; 32], client_pub: &[u8; 32], server_pub: &[u8; 32]) -> (String, [u8; 32]) {
        let shared = self.secret.diffie_hellman(&x25519_dalek::PublicKey::from(*their_pub));
        let transcript = Sha256::new().chain_update(b"rekindle pair v1").chain_update(client_pub).chain_update(server_pub).finalize();
        let digest = Sha256::new().chain_update(shared.as_bytes()).chain_update(transcript).finalize();
        let code = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
        let hk = Hkdf::<Sha256>::new(Some(&transcript), shared.as_bytes());
        let mut key = [0u8; 32];
        hk.expand(b"rekindle pairing key", &mut key).expect("32 bytes is a valid length");
        (format!("{:03} {:03}", code / 1000, code % 1000), key)
    }
}

impl Default for PairKeys {
    fn default() -> Self {
        Self::new()
    }
}

/// Proof that a side confirmed with the same key (hash, reveals nothing about the key).
pub fn confirm_proof(key: &[u8; 32], role: &str) -> String {
    hex(&Sha256::new().chain_update(b"rekindle pair confirm").chain_update(key).chain_update(role.as_bytes()).finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_get_the_same_code_and_key() {
        let (c, s) = (PairKeys::new(), PairKeys::new());
        let (code_c, key_c) = c.finish(&s.public, &c.public, &s.public);
        let (code_s, key_s) = s.finish(&c.public, &c.public, &s.public);
        assert_eq!(code_c, code_s);
        assert_eq!(key_c, key_s);
        assert_eq!(code_c.len(), 7);
        // Someone in the middle with their own keys gets a different code.
        let m = PairKeys::new();
        let (code_m, _) = c.finish(&m.public, &c.public, &m.public);
        assert_ne!(code_m, code_s);
    }

    #[test]
    fn paired_handshake_and_encrypted_messages() {
        let key = [7u8; 32];
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let hello = recv_plain(&mut s).unwrap();
            let mut ch = Channel::accept(s, &hello, "server0000000000", |id| (id == "client0000000000").then_some(key)).unwrap();
            let (v, body) = ch.recv().unwrap();
            ch.send(&json!({"t": "echo", "n": v["n"]}), &body).unwrap();
        });
        let s = TcpStream::connect(addr).unwrap();
        let mut ch = Channel::connect(s, "client0000000000", "server0000000000", &key).unwrap();
        let (v, body) = ch.call(&json!({"t": "ping", "n": 5}), b"payload").unwrap();
        assert_eq!(v["n"], 5);
        assert_eq!(body, b"payload");
        server.join().unwrap();
    }

    #[test]
    fn wrong_key_is_rejected() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let hello = recv_plain(&mut s).unwrap();
            Channel::accept(s, &hello, "server0000000000", |_| Some([1u8; 32])).is_err()
        });
        let s = TcpStream::connect(addr).unwrap();
        assert!(Channel::connect(s, "client0000000000", "server0000000000", &[2u8; 32]).is_err());
        assert!(server.join().unwrap());
    }
}
