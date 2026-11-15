//! Pairing devices on the local network and keeping their saves in sync (and copying
//! games to devices without a file picker, such as phones).
//!
//! - `identity`: this device's id and name, and the paired devices.
//! - `wire`: framed, encrypted messages; the pairing key exchange.
//! - `saves`: which files are saves, their manifests and the sync plan.
//! - `service`: discovery, the server answering paired devices, and the client side.

pub mod identity;
pub mod saves;
pub mod service;
pub mod wire;

pub use service::{Nearby, PairPrompt, RemoteGame, Service, Shared, Transfer};
