#!/usr/bin/env python3
"""Build KlikCore for Android: a signed APK in target/android/.

    python scripts/android/build.py [--release] [--abi arm64-v8a,armeabi-v7a,x86_64]

The crate's library is built as `libklikcore.so` with the NDK's clang, miniquad's Java
activity is compiled and dexed, and everything is packed into an APK with the Android
SDK's build tools (aapt2, d8, zipalign, apksigner). Python standard library only.

Tools are found through these environment variables, with common install folders as
fallback: ANDROID_HOME / ANDROID_SDK_ROOT (SDK), ANDROID_NDK_HOME / ANDROID_NDK_ROOT /
ANDROID_NDK_LATEST_HOME (NDK; default: the newest under <sdk>/ndk), JAVA_HOME (javac,
keytool), CARGO (cargo).

Signing: KLIKCORE_KEYSTORE / KLIKCORE_KEYSTORE_PASS / KLIKCORE_KEY_ALIAS if set, else the
release key in ~/.klikcore-signing (release.keystore + password.txt, alias "klikcore") if
present, else a debug keystore made in target/android/ on first use.
"""

import argparse
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = "com.takobyte.klikcore"
LIB_NAME = "klikcore"
MIN_SDK = 24
TARGET_SDK = 35
WINDOWS = os.name == "nt"

# Android ABI -> (Rust target, NDK clang prefix)
ABIS = {
    "arm64-v8a": ("aarch64-linux-android", "aarch64-linux-android"),
    "armeabi-v7a": ("armv7-linux-androideabi", "armv7a-linux-androideabi"),
    "x86_64": ("x86_64-linux-android", "x86_64-linux-android"),
    "x86": ("i686-linux-android", "i686-linux-android"),
}


def die(msg):
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


def run(cmd, **kw):
    print("+ " + " ".join(str(c) for c in cmd), flush=True)
    r = subprocess.run([str(c) for c in cmd], **kw)
    if r.returncode != 0:
        die(f"{Path(str(cmd[0])).name} failed (exit {r.returncode})")
    return r


def version_key(name):
    return [int(x) if x.isdigit() else x for x in re.split(r"[.\-]", name)]


def newest(dir_):
    subs = [p for p in dir_.iterdir() if p.is_dir()] if dir_.is_dir() else []
    return max(subs, key=lambda p: version_key(p.name), default=None)


def find_sdk():
    cands = [os.environ.get(v) for v in ("ANDROID_HOME", "ANDROID_SDK_ROOT")]
    home = Path.home()
    cands += [
        os.environ.get("LOCALAPPDATA", "") and str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk"),
        str(home / "AppData" / "Local" / "Android" / "Sdk"),
        str(home / "Android" / "Sdk"),
        str(home / "Library" / "Android" / "sdk"),
    ]
    for c in cands:
        if c and Path(c, "platforms").is_dir():
            return Path(c)
    die("Android SDK not found: set ANDROID_HOME")


def find_ndk(sdk):
    for v in ("ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "ANDROID_NDK_LATEST_HOME", "ANDROID_NDK"):
        p = os.environ.get(v)
        if p and Path(p, "toolchains").is_dir():
            return Path(p)
    ndk = newest(sdk / "ndk")
    if ndk and (ndk / "toolchains").is_dir():
        return ndk
    die("Android NDK not found: set ANDROID_NDK_HOME (or install one with the SDK manager)")


def ndk_bin(ndk):
    host = {"Windows": "windows-x86_64", "Linux": "linux-x86_64", "Darwin": "darwin-x86_64"}[platform.system()]
    b = ndk / "toolchains" / "llvm" / "prebuilt" / host / "bin"
    if not b.is_dir():
        die(f"no NDK toolchain at {b}")
    return b


def exe(path):
    """An NDK / SDK tool, with Windows' .exe / .cmd / .bat suffix if there is one."""
    if WINDOWS:
        for ext in (".exe", ".cmd", ".bat"):
            if path.with_name(path.name + ext).exists():
                return path.with_name(path.name + ext)
    if path.exists():
        return path
    die(f"missing tool: {path}")


def find_java_tool(name):
    cands = []
    if os.environ.get("JAVA_HOME"):
        cands.append(Path(os.environ["JAVA_HOME"]) / "bin" / name)
    cands += [
        Path(r"C:\Program Files\Android\Android Studio\jbr\bin") / name,
        Path("/opt/android-studio/jbr/bin") / name,
    ]
    for c in cands:
        for ext in ((".exe",) if WINDOWS else ()) + ("",):
            if c.with_name(c.name + ext).is_file():
                return c.with_name(c.name + ext)
    found = shutil.which(name)
    if found:
        return Path(found)
    die(f"{name} not found: set JAVA_HOME to a JDK (17 or newer)")


def find_cargo():
    if os.environ.get("CARGO"):
        return Path(os.environ["CARGO"])
    home = Path.home() / ".cargo" / "bin" / ("cargo.exe" if WINDOWS else "cargo")
    if home.is_file():
        return home
    found = shutil.which("cargo")
    if found:
        return Path(found)
    die("cargo not found")


def crate_version():
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^\[package\][^\[]*?^version\s*=\s*"([^"]+)"', text, re.M | re.S)
    if not m:
        die("no version in Cargo.toml")
    name = m.group(1)
    nums = [int(x) for x in re.findall(r"\d+", name.split("-")[0])[:3]] + [0, 0, 0]
    return name, max(1, nums[0] * 10000 + nums[1] * 100 + nums[2])


def build_library(cargo, abi, ndk, release):
    rust_target, clang = ABIS[abi]
    b = ndk_bin(ndk)
    cc = exe(b / f"{clang}{MIN_SDK}-clang")
    cxx = exe(b / f"{clang}{MIN_SDK}-clang++")
    env = dict(os.environ)
    t = rust_target.replace("-", "_")
    env[f"CC_{t}"] = str(cc)
    env[f"CXX_{t}"] = str(cxx)
    env[f"AR_{t}"] = str(exe(b / "llvm-ar"))
    env[f"CARGO_TARGET_{t.upper()}_LINKER"] = str(cc)
    env.setdefault("ANDROID_NDK_HOME", str(ndk))
    cmd = [cargo, "rustc", "--lib", "--crate-type", "cdylib", "--target", rust_target]
    if release:
        cmd.append("--release")
    run(cmd, cwd=ROOT, env=env)
    so = ROOT / "target" / rust_target / ("release" if release else "debug") / f"lib{LIB_NAME}.so"
    if not so.is_file():
        die(f"cargo did not produce {so}")
    return so


def miniquad_java_dir(cargo):
    out = subprocess.run(
        [str(cargo), "metadata", "--format-version", "1"], cwd=ROOT, capture_output=True, text=True, encoding="utf-8"
    )
    if out.returncode != 0:
        die("cargo metadata failed:\n" + out.stderr)
    for p in json.loads(out.stdout)["packages"]:
        if p["name"] == "miniquad":
            d = Path(p["manifest_path"]).parent / "java"
            if (d / "MainActivity.java").is_file():
                return d
    die("miniquad's Java sources not found")


SURFACE_SCALE = """    // KlikCore: pixels of the view per pixel of the surface (touch is divided by it).
    float scale = 1.0f;
    boolean uiScale = true;

    void updateScale(int w, int h) {
        float density = getResources().getDisplayMetrics().density;
        float s = 1.0f; // the launcher scales itself (src/launcher/mod.rs ui_scale)
        scale = s;
        if (s > 1.0f) {
            getHolder().setFixedSize(Math.round(w / s), Math.round(h / s));
        } else {
            getHolder().setSizeFromLayout();
        }
    }

    // Android sometimes drops the fixed surface size while the window settles (bars
    // hiding, rotation), leaving the UI drawn small in a corner: check twice a second and
    // apply it again when the surface isn't the size it should be.
    boolean healing = false;
    final Runnable heal = new Runnable() {
        @Override
        public void run() {
            if (uiScale && scale > 1.0f && getWidth() > 0) {
                int ww = Math.round(getWidth() / scale);
                int hh = Math.round(getHeight() / scale);
                android.graphics.Rect f = getHolder().getSurfaceFrame();
                if (f.width() != ww || f.height() != hh) {
                    getHolder().setSizeFromLayout();
                    getHolder().setFixedSize(ww, hh);
                }
            }
            postDelayed(this, 500);
        }
    };

    @Override
    protected void onSizeChanged(int w, int h, int oldw, int oldh) {
        super.onSizeChanged(w, h, oldw, oldh);
        updateScale(w, h);
        if (!healing) {
            healing = true;
            postDelayed(heal, 500);
        }
    }

    public void setUiScale(boolean on) {
        uiScale = on;
        if (getWidth() > 0) {
            updateScale(getWidth(), getHeight());
        }
    }

"""

ACTIVITY_SCALE = """    // KlikCore: lower-resolution (readable) launcher UI on, or full resolution for games.
    public void setUiScale(final boolean on) {
        runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    view.setUiScale(on);
                }
            });
    }

"""


# Immersive full screen: no status or navigation bar over the app.
IMMERSIVE = """    public void hideSystemBars() {
        View decorView = getWindow().getDecorView();
        if (Build.VERSION.SDK_INT >= 28) {
            getWindow().getAttributes().layoutInDisplayCutoutMode = LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
        }
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            android.view.WindowInsetsController c = decorView.getWindowInsetsController();
            if (c != null) {
                c.hide(android.view.WindowInsets.Type.systemBars());
                c.setSystemBarsBehavior(android.view.WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            decorView.setSystemUiVisibility(View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_FULLSCREEN
                | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_LAYOUT_STABLE);
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        }
    }

"""


# Home-screen shortcuts that open one game: pinned through ShortcutManager; the game's
# path rides in the intent and Rust asks for it with takeLaunchGame().
SHORTCUTS = """    String launchGame = null;
    boolean firstIntentTaken = false;

    void takeGameFrom(Intent i) {
        if (i != null) {
            String g = i.getStringExtra("klikcore_game");
            if (g != null) {
                launchGame = g;
            }
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        if (takeUpdateStatus(intent)) {
            return;
        }
        setIntent(intent);
        takeGameFrom(intent);
    }

    public void goHome() {
        moveTaskToBack(true);
    }

    public String takeLaunchGame() {
        if (!firstIntentTaken) {
            firstIntentTaken = true;
            takeGameFrom(getIntent());
        }
        String g = launchGame;
        launchGame = null;
        return g == null ? "" : g;
    }

    public boolean pinShortcut(String id, String label, String iconPath, String gamePath) {
        if (Build.VERSION.SDK_INT < 26) {
            return false;
        }
        android.content.pm.ShortcutManager sm = getSystemService(android.content.pm.ShortcutManager.class);
        if (sm == null || !sm.isRequestPinShortcutSupported()) {
            return false;
        }
        Intent intent = new Intent(this, MainActivity.class);
        intent.setAction(Intent.ACTION_VIEW);
        intent.putExtra("klikcore_game", gamePath);
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        android.content.pm.ShortcutInfo.Builder b = new android.content.pm.ShortcutInfo.Builder(this, id)
            .setShortLabel(label)
            .setLongLabel(label)
            .setIntent(intent);
        android.graphics.Bitmap bmp = iconPath.isEmpty() ? null : android.graphics.BitmapFactory.decodeFile(iconPath);
        if (bmp != null) {
            b.setIcon(android.graphics.drawable.Icon.createWithAdaptiveBitmap(padForAdaptive(bmp)));
        } else {
            b.setIcon(android.graphics.drawable.Icon.createWithResource(this, getApplicationInfo().icon));
        }
        try {
            return sm.requestPinShortcut(b.build(), null);
        } catch (Exception e) {
            Log.w("KlikCore", "pinShortcut: " + e);
            return false;
        }
    }

    // Adaptive icons are cropped to a mask: keep the game's icon inside the safe area on
    // a dark backdrop.
    android.graphics.Bitmap padForAdaptive(android.graphics.Bitmap src) {
        int size = 432;
        android.graphics.Bitmap out = android.graphics.Bitmap.createBitmap(size, size, android.graphics.Bitmap.Config.ARGB_8888);
        android.graphics.Canvas c = new android.graphics.Canvas(out);
        c.drawColor(0xff17171c);
        int inner = 264;
        int o = (size - inner) / 2;
        android.graphics.Paint p = new android.graphics.Paint(android.graphics.Paint.FILTER_BITMAP_FLAG);
        c.drawBitmap(src, null, new android.graphics.Rect(o, o, o + inner, o + inner), p);
        return out;
    }

"""


# Self-update: Rust finds a newer release (httpGet: Android has no curl, and Rust has no
# TLS here); installUpdate downloads the APK into a PackageInstaller session, and Android
# asks the player to confirm. The result comes back to the activity as an intent.
UPDATER = """    static final String UPDATE_ACTION = "com.takobyte.klikcore.UPDATE_STATUS";
    volatile String updateState = "";

    // GET over HTTPS; "" on any failure.
    public String httpGet(String url) {
        try {
            java.net.HttpURLConnection c = (java.net.HttpURLConnection) new java.net.URL(url).openConnection();
            c.setRequestProperty("User-Agent", "KlikCore-updater");
            c.setRequestProperty("Accept", "application/vnd.github+json");
            c.setConnectTimeout(15000);
            c.setReadTimeout(30000);
            if (c.getResponseCode() != 200) {
                return "";
            }
            java.io.InputStream in = c.getInputStream();
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            byte[] buf = new byte[16384];
            int n;
            while ((n = in.read(buf)) > 0) {
                out.write(buf, 0, n);
            }
            in.close();
            return out.toString("UTF-8");
        } catch (Exception e) {
            Log.w("KlikCore", "httpGet: " + e);
            return "";
        }
    }

    // "", "downloading <percent>", "waiting" (Android is asking the player) or "failed <why>".
    public String updateStatus() {
        return updateState;
    }

    public void installUpdate(final String url) {
        updateState = "downloading 0";
        new Thread(new Runnable() {
                @Override
                public void run() {
                    try {
                        java.net.HttpURLConnection c = (java.net.HttpURLConnection) new java.net.URL(url).openConnection();
                        c.setRequestProperty("User-Agent", "KlikCore-updater");
                        c.setConnectTimeout(15000);
                        c.setReadTimeout(60000);
                        if (c.getResponseCode() != 200) {
                            updateState = "failed the download answered " + c.getResponseCode();
                            return;
                        }
                        long total = c.getContentLengthLong();
                        android.content.pm.PackageInstaller pi = getPackageManager().getPackageInstaller();
                        android.content.pm.PackageInstaller.SessionParams params =
                            new android.content.pm.PackageInstaller.SessionParams(android.content.pm.PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                        if (total > 0) {
                            params.setSize(total);
                        }
                        int id = pi.createSession(params);
                        android.content.pm.PackageInstaller.Session s = pi.openSession(id);
                        java.io.InputStream in = c.getInputStream();
                        java.io.OutputStream o = s.openWrite("klikcore.apk", 0, total > 0 ? total : -1);
                        byte[] buf = new byte[65536];
                        long done = 0;
                        int n;
                        while ((n = in.read(buf)) > 0) {
                            o.write(buf, 0, n);
                            done += n;
                            if (total > 0) {
                                updateState = "downloading " + (done * 100 / total);
                            }
                        }
                        in.close();
                        s.fsync(o);
                        o.close();
                        Intent i = new Intent(MainActivity.this, MainActivity.class);
                        i.setAction(UPDATE_ACTION);
                        int flags = android.app.PendingIntent.FLAG_UPDATE_CURRENT
                            | (Build.VERSION.SDK_INT >= 31 ? android.app.PendingIntent.FLAG_MUTABLE : 0);
                        android.app.PendingIntent pending = android.app.PendingIntent.getActivity(MainActivity.this, id, i, flags);
                        updateState = "waiting";
                        s.commit(pending.getIntentSender());
                        s.close();
                    } catch (Exception e) {
                        Log.w("KlikCore", "installUpdate: " + e);
                        updateState = "failed " + e.getMessage();
                    }
                }
            }).start();
    }

    // The installer's answer: show its confirmation, or note why it failed.
    boolean takeUpdateStatus(Intent intent) {
        if (intent == null || !UPDATE_ACTION.equals(intent.getAction())) {
            return false;
        }
        int status = intent.getIntExtra(android.content.pm.PackageInstaller.EXTRA_STATUS, -999);
        if (status == android.content.pm.PackageInstaller.STATUS_PENDING_USER_ACTION) {
            Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
            if (confirm != null) {
                try {
                    startActivity(confirm);
                } catch (Exception e) {
                    updateState = "failed " + e.getMessage();
                }
            }
        } else if (status == android.content.pm.PackageInstaller.STATUS_SUCCESS) {
            updateState = "";
        } else if (status == android.content.pm.PackageInstaller.STATUS_FAILURE_CONFLICT
                || status == android.content.pm.PackageInstaller.STATUS_FAILURE_INCOMPATIBLE) {
            updateState = "failed key";
        } else if (status == android.content.pm.PackageInstaller.STATUS_FAILURE_ABORTED) {
            updateState = "failed cancelled";
        } else {
            String m = intent.getStringExtra(android.content.pm.PackageInstaller.EXTRA_STATUS_MESSAGE);
            updateState = "failed " + (m == null ? ("status " + status) : m);
        }
        return true;
    }

"""


def write_java(src_dir, out_dir):
    """miniquad's activity with our package and library name; `//%` plugin hooks removed."""
    if out_dir.exists():
        shutil.rmtree(out_dir)
    activity = (src_dir / "MainActivity.java").read_text(encoding="utf-8")
    activity = activity.replace("TARGET_PACKAGE_NAME", PACKAGE).replace('"LIBRARY_NAME"', f'"{LIB_NAME}"')
    activity = "".join(l for l in activity.splitlines(True) if not l.strip().startswith("//%"))
    # Leave the volume keys to the system (miniquad's view takes every key).
    hook = "public boolean onKey(View v, int keyCode, KeyEvent event) {"
    if hook not in activity:
        die("miniquad's MainActivity.java changed: onKey not found")
    activity = activity.replace(
        hook,
        hook
        + "\n        if (keyCode == KeyEvent.KEYCODE_VOLUME_UP || keyCode == KeyEvent.KEYCODE_VOLUME_DOWN"
        + "\n                || keyCode == KeyEvent.KEYCODE_VOLUME_MUTE) {\n            return false;\n        }",
    )
    # The launcher renders at a lower resolution that the system scales up, so its UI is
    # readable on phones; games get the full resolution (Rust calls setUiScale).
    activity, n = re.subn(r"event\.get([XY])\((\w+)\);", lambda m: f"event.get{m[1]}({m[2]}) / scale;", activity)
    anchors = ["    public Surface getNativeSurface() {", "    public void showKeyboard(final boolean show) {"]
    if n != 12 or any(a not in activity for a in anchors):
        die("miniquad's MainActivity.java changed: touch handling / anchors not found")
    activity = activity.replace(anchors[0], SURFACE_SCALE + anchors[0])
    activity = activity.replace(anchors[1], ACTIVITY_SCALE + anchors[1])
    # Hide the status and navigation bars (miniquad only lets the app draw under them on
    # Android 11+), again whenever the app comes back; a swipe shows them for a moment.
    fit = "getWindow().setDecorFitsSystemWindows(false);"
    if fit not in activity or "    protected void onResume() {\n        super.onResume();" not in activity:
        die("miniquad's MainActivity.java changed: fullscreen / onResume not found")
    activity = activity.replace(fit, fit + "\n                            hideSystemBars();")
    activity = activity.replace(
        "    protected void onResume() {\n        super.onResume();",
        "    protected void onResume() {\n        super.onResume();\n        hideSystemBars();",
    )
    activity = activity.replace(anchors[1], IMMERSIVE + SHORTCUTS + UPDATER + anchors[1])
    # Back always goes to KlikCore (as the key it is), never to Android's default (which
    # put the app in the background mid-game when the view didn't have focus). The
    # launcher leaves the app itself (goHome) when Back is pressed on the library.
    back = "        super.onBackPressed();"
    if back not in activity:
        die("miniquad's MainActivity.java changed: onBackPressed not found")
    activity = activity.replace(
        back, "        QuadNative.surfaceOnKeyDown(KeyEvent.KEYCODE_BACK);\n        QuadNative.surfaceOnKeyUp(KeyEvent.KEYCODE_BACK);"
    )
    files = [
        (out_dir / Path(*PACKAGE.split(".")) / "MainActivity.java", activity),
        (out_dir / "quad_native" / "QuadNative.java", (src_dir / "QuadNative.java").read_text(encoding="utf-8")),
    ]
    for path, text in files:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    return [p for p, _ in files]


def platform_jar(sdk):
    plats = [p for p in (sdk / "platforms").iterdir() if (p / "android.jar").is_file()]
    if not plats:
        die("no Android platform (android.jar) installed in the SDK")
    best = max(plats, key=lambda p: version_key(p.name.replace("android-", "")))
    return best / "android.jar"


def ensure_keystore(keytool, out):
    if os.environ.get("KLIKCORE_KEYSTORE"):
        return (
            Path(os.environ["KLIKCORE_KEYSTORE"]),
            os.environ.get("KLIKCORE_KEYSTORE_PASS") or "",
            os.environ.get("KLIKCORE_KEY_ALIAS") or "klikcore",
        )
    # The release key, if this machine has it (kept outside the repo), so local builds
    # update the same app as the published ones.
    home = Path(os.environ.get("USERPROFILE") or os.environ.get("HOME") or "~").expanduser() / ".klikcore-signing"
    if (home / "release.keystore").is_file() and (home / "password.txt").is_file():
        return home / "release.keystore", (home / "password.txt").read_text().strip(), "klikcore"
    ks = out / "debug.keystore"
    if not ks.is_file():
        run([
            keytool, "-genkeypair", "-keystore", ks, "-storepass", "android", "-keypass", "android",
            "-alias", "androiddebugkey", "-keyalg", "RSA", "-keysize", "2048", "-validity", "10000",
            "-dname", "CN=Android Debug,O=Android,C=US", "-noprompt",
        ])
    return ks, "android", "androiddebugkey"


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--release", action="store_true", help="optimised build (klikcore-release.apk)")
    ap.add_argument("--abi", default="arm64-v8a", help="comma-separated: " + ", ".join(ABIS))
    ap.add_argument("--keep-debug-info", action="store_true", help="don't strip debug info from the library")
    args = ap.parse_args()
    abis = [a.strip() for a in args.abi.split(",") if a.strip()]
    for a in abis:
        if a not in ABIS:
            die(f"unknown ABI {a}")

    sdk = find_sdk()
    ndk = find_ndk(sdk)
    build_tools = newest(sdk / "build-tools")
    if not build_tools:
        die("no SDK build-tools installed")
    jar = platform_jar(sdk)
    cargo = find_cargo()
    javac, keytool = find_java_tool("javac"), find_java_tool("keytool")
    print(f"SDK {sdk}\nNDK {ndk}\nbuild-tools {build_tools.name}, {jar.parent.name}", flush=True)

    out = ROOT / "target" / "android"
    out.mkdir(parents=True, exist_ok=True)
    version_name, version_code = crate_version()

    # 1. Native libraries.
    libs = {}
    for abi in abis:
        so = build_library(cargo, abi, ndk, args.release)
        dst = out / "lib" / abi / f"lib{LIB_NAME}.so"
        dst.parent.mkdir(parents=True, exist_ok=True)
        if args.keep_debug_info:
            shutil.copyfile(so, dst)
        else:
            run([exe(ndk_bin(ndk) / "llvm-strip"), "--strip-debug", "-o", dst, so])
        libs[abi] = dst

    # 2. Java: miniquad's activity -> classes.dex.
    java_src = write_java(miniquad_java_dir(cargo), out / "java")
    classes = out / "classes"
    if classes.exists():
        shutil.rmtree(classes)
    classes.mkdir()
    run([javac, "--release", "11", "-nowarn", "-Xlint:-options", "-encoding", "UTF-8",
         "-classpath", jar, "-d", classes] + java_src)
    dex = out / "dex"
    if dex.exists():
        shutil.rmtree(dex)
    dex.mkdir()
    class_files = sorted(classes.rglob("*.class"))
    run([exe(build_tools / "d8"), "--min-api", MIN_SDK, "--lib", jar, "--output", dex]
        + (["--release"] if args.release else ["--debug"]) + class_files)

    # 3. Resources and manifest -> base APK.
    manifest = (ROOT / "android" / "AndroidManifest.xml").read_text(encoding="utf-8")
    manifest = (manifest.replace("@PACKAGE@", PACKAGE).replace("@VERSION_CODE@", str(version_code))
                .replace("@VERSION_NAME@", version_name))
    (out / "AndroidManifest.xml").write_text(manifest, encoding="utf-8")
    flat = out / "res.zip"
    run([exe(build_tools / "aapt2"), "compile", "--dir", ROOT / "android" / "res", "-o", flat])
    base = out / "base.apk"
    link = [exe(build_tools / "aapt2"), "link", "-o", base, "-I", jar, "--manifest", out / "AndroidManifest.xml",
            "--min-sdk-version", MIN_SDK, "--target-sdk-version", TARGET_SDK,
            "--version-code", version_code, "--version-name", version_name, flat]
    if not args.release:
        link.append("--debug-mode")
    run(link)

    # 4. Add the dex and the native libraries (stored, so they load straight from the APK).
    unaligned = out / "unaligned.apk"
    shutil.copyfile(base, unaligned)
    with zipfile.ZipFile(unaligned, "a") as z:
        z.write(dex / "classes.dex", "classes.dex", compress_type=zipfile.ZIP_DEFLATED)
        for abi, so in libs.items():
            z.write(so, f"lib/{abi}/{so.name}", compress_type=zipfile.ZIP_STORED)

    # 5. Align (16 KB pages for the libraries) and sign.
    name = "klikcore-release.apk" if args.release else "klikcore-debug.apk"
    apk = out / name
    run([exe(build_tools / "zipalign"), "-f", "-P", "16", "4", unaligned, apk])
    ks, ks_pass, alias = ensure_keystore(keytool, out)
    # The password goes through the environment so the echoed command doesn't show it.
    env = dict(os.environ, KLIKCORE_SIGN_PASS=ks_pass)
    run([exe(build_tools / "apksigner"), "sign", "--ks", ks, "--ks-pass", "env:KLIKCORE_SIGN_PASS",
         "--ks-key-alias", alias, "--key-pass", "env:KLIKCORE_SIGN_PASS", apk], env=env)
    run([exe(build_tools / "apksigner"), "verify", apk])
    for f in (base, unaligned):
        f.unlink(missing_ok=True)
    print(f"\n{apk} ({apk.stat().st_size / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
