//! KlikCore library: a clean-room parser and runtime for Clickteam Fusion games.
//! Contains no game data; everything is loaded from user-supplied files at runtime.

/// Sound decoding (format-independent PCM / WAV conversion).
pub mod audio;
pub mod fxhash;
/// Format parser. Has no rendering dependencies.
pub mod ctf_loader;
pub mod discord;
/// The game library / launcher shown when no game is given.
pub mod launcher;
/// Replacement packs (mods) for images and sounds.
pub mod mods;
/// Log file next to the saves, and panic capture.
pub mod log;
pub mod midi;
/// Macroquad-based frame viewer.
pub mod render;
/// Object runtime and event interpreter (no rendering dependencies).
pub mod runtime;
pub mod sync;
/// Android entry point (the app is this library, loaded by miniquad's Java activity).
#[cfg(target_os = "android")]
mod android;
