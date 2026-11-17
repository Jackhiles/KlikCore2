//! Android entry point. The app is this library built as `libklikcore.so`, loaded by
//! miniquad's Java activity (see scripts/android/build.py), which calls `quad_main`.
//!
//! There are no separate processes on Android: the launcher and the games share one
//! macroquad loop. Play hands the game file back here, the game runs until it ends (its
//! own "end application", or Back pressed twice), then the launcher shows again.

use crate::{ctf_loader, launcher, render, runtime};
use macroquad::miniquad::native::android::{attach_jni_env, ACTIVITY};
use macroquad::miniquad::{call_object_method, call_void_method, get_utf_str};
use std::path::{Path, PathBuf};

/// Called by miniquad once the activity is created.
#[no_mangle]
pub extern "C" fn quad_main() {
    unsafe {
        let env = attach_jni_env();
        // cpal (audio) finds the Java VM and the activity through ndk-context (replaced
        // if Android re-creates the activity in the same process).
        static CONTEXT_SET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if CONTEXT_SET.swap(true, std::sync::atomic::Ordering::SeqCst) {
            ndk_context::release_android_context();
        }
        let mut vm = std::ptr::null_mut();
        (**env).GetJavaVM.unwrap()(env, &mut vm);
        ndk_context::initialize_android_context(vm as _, ACTIVITY as _);
        runtime::files::set_android_dirs(app_dir(env, "getFilesDir"), app_dir(env, "getExternalFilesDir"));
    }
    let mut conf = launcher::window_conf();
    conf.fullscreen = true;
    macroquad::Window::from_config(conf, app());
}

async fn app() {
    // After miniquad's own hook (which logs panics to logcat), so both run.
    crate::log::install_panic_hook();
    launcher::run().await;
}

/// `Context.getFilesDir()` / `getExternalFilesDir(null)` as a path.
unsafe fn app_dir(env: *mut macroquad::miniquad::native::android::ndk_sys::JNIEnv, method: &str) -> Option<PathBuf> {
    let file = if method == "getExternalFilesDir" {
        call_object_method!(env, ACTIVITY, method, "(Ljava/lang/String;)Ljava/io/File;", std::ptr::null_mut::<std::ffi::c_void>())
    } else {
        call_object_method!(env, ACTIVITY, method, "()Ljava/io/File;")
    };
    if file.is_null() {
        return None;
    }
    let path = call_object_method!(env, file, "getAbsolutePath", "()Ljava/lang/String;");
    (!path.is_null()).then(|| PathBuf::from(get_utf_str!(env, path)))
}

/// Put the app in the background (Back on the library, like other apps).
pub(crate) fn go_home() {
    unsafe {
        let env = attach_jni_env();
        call_void_method!(env, ACTIVITY, "goHome", "()V");
    }
}

/// A game a home-screen shortcut asked to open (once per request).
pub(crate) fn take_launch_game() -> Option<PathBuf> {
    unsafe {
        let env = attach_jni_env();
        let s = call_object_method!(env, ACTIVITY, "takeLaunchGame", "()Ljava/lang/String;");
        if s.is_null() {
            return None;
        }
        let path = get_utf_str!(env, s).to_string();
        (!path.is_empty()).then(|| PathBuf::from(path))
    }
}

/// Ask the home screen to pin a shortcut that opens `game` (Android 8+). `icon` is a PNG
/// file (or None for KlikCore's icon). Returns false where pinning isn't supported.
pub(crate) fn pin_shortcut(id: &str, label: &str, icon: Option<&Path>, game: &Path) -> bool {
    use std::ffi::CString;
    unsafe {
        let env = attach_jni_env();
        let jstr = |s: &str| {
            let c = CString::new(s.replace('\0', "")).unwrap_or_default();
            (**env).NewStringUTF.unwrap()(env, c.as_ptr())
        };
        let icon = icon.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let (a, b, c, d) = (jstr(id), jstr(label), jstr(&icon), jstr(&game.to_string_lossy()));
        macroquad::miniquad::call_bool_method!(env, ACTIVITY, "pinShortcut", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z", a, b, c, d) != 0
    }
}

/// A Java string from Rust.
unsafe fn jstring(env: *mut macroquad::miniquad::native::android::ndk_sys::JNIEnv, s: &str) -> *mut std::ffi::c_void {
    let c = std::ffi::CString::new(s.replace('\0', "")).unwrap_or_default();
    (**env).NewStringUTF.unwrap()(env, c.as_ptr()) as _
}

/// Call a `String f(...)` method of the activity and take its text (freeing the local
/// references, as these run in loops on background threads).
unsafe fn string_call(env: *mut macroquad::miniquad::native::android::ndk_sys::JNIEnv, s: *mut std::ffi::c_void) -> String {
    if s.is_null() {
        return String::new();
    }
    let text = get_utf_str!(env, s).to_string();
    (**env).DeleteLocalRef.unwrap()(env, s as _);
    text
}

/// HTTPS GET through Java (there is no curl or TLS stack in the app).
pub(crate) fn http_get(url: &str) -> Result<Vec<u8>, String> {
    unsafe {
        let env = attach_jni_env();
        let u = jstring(env, url);
        let s = call_object_method!(env, ACTIVITY, "httpGet", "(Ljava/lang/String;)Ljava/lang/String;", u);
        (**env).DeleteLocalRef.unwrap()(env, u as _);
        let body = string_call(env, s);
        if body.is_empty() {
            Err("no answer from GitHub".into())
        } else {
            Ok(body.into_bytes())
        }
    }
}

/// Open a web page in the browser.
pub(crate) fn open_url(url: &str) {
    unsafe {
        let env = attach_jni_env();
        let u = jstring(env, url);
        call_void_method!(env, ACTIVITY, "openUrl", "(Ljava/lang/String;)V", u);
        (**env).DeleteLocalRef.unwrap()(env, u as _);
    }
}

/// Copy `file` into the phone's Download/KlikCore folder (the app's external folder on
/// Android 9 and older) as `name`; returns where it went.
pub(crate) fn save_to_downloads(file: &Path, name: &str, mime: &str) -> Option<String> {
    unsafe {
        let env = attach_jni_env();
        let (p, n, m) = (jstring(env, &file.to_string_lossy()), jstring(env, name), jstring(env, mime));
        let s = call_object_method!(env, ACTIVITY, "saveToDownloads", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;", p, n, m);
        for r in [p, n, m] {
            (**env).DeleteLocalRef.unwrap()(env, r as _);
        }
        let place = string_call(env, s);
        (!place.is_empty()).then_some(place)
    }
}

/// Start downloading and installing the APK at `url` (see [`update_status`]).
pub(crate) fn install_update(url: &str) {
    unsafe {
        let env = attach_jni_env();
        let u = jstring(env, url);
        call_void_method!(env, ACTIVITY, "installUpdate", "(Ljava/lang/String;)V", u);
        (**env).DeleteLocalRef.unwrap()(env, u as _);
    }
}

/// The update's progress: "", "downloading <percent>", "waiting" or "failed <why>".
pub(crate) fn update_status() -> String {
    unsafe {
        let env = attach_jni_env();
        let s = call_object_method!(env, ACTIVITY, "updateStatus", "()Ljava/lang/String;");
        string_call(env, s)
    }
}

/// Run a game inside the launcher's window until it ends (then any game it asked to start,
/// as launchers do). Returns an error message when a game can't be loaded.
pub(crate) async fn play(path: &Path) -> Result<(), String> {
    set_ui_scale(false);
    let r = play_games(path).await;
    set_ui_scale(true);
    r
}

/// The launcher draws into a smaller surface that Android scales up (readable on phones);
/// games use the full screen resolution. See `SURFACE_SCALE` in scripts/android/build.py.
fn set_ui_scale(on: bool) {
    unsafe {
        let env = attach_jni_env();
        call_void_method!(env, ACTIVITY, "setUiScale", "(Z)V", on as i32);
    }
}

async fn play_games(path: &Path) -> Result<(), String> {
    let mut next = Some(path.to_path_buf());
    let _ = render::take_next_game();
    while let Some(path) = next.take() {
        let game = ctf_loader::GameData::load(&path).map_err(|e| format!("Could not load {}: {e}", path.display()))?;
        let legacy = runtime::files::default_save_root(&game.name);
        let opts = render::ViewerOptions {
            run_events: true,
            game_path: Some(path.clone()),
            settings: render::settings::Settings::for_game(&path, Some(&legacy)),
            fullscreen: Some(true),
            ..Default::default()
        };
        render::run(game, opts).await;
        next = render::take_next_game();
    }
    Ok(())
}
