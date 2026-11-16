# KlikCore 2

*(Formerly Rekindle. Data from earlier versions moves over automatically; paired devices keep working.)*

A clean-room, open-source runtime for games built with Clickteam Fusion 2.5 and Multimedia Fusion 1.5, written in Rust.
It works the way ScummVM or OpenMW do: this repository is only the engine. It contains
**no game data, no assets and no game-specific logic**. You supply the files of a game you own.

```
cargo run --release -- path/to/game.exe              # run the game
cargo run --release -- path/to/game.exe --dump-info  # print a summary
cargo run --release -- path/to/game.ccn --extract out/
```

## Status

| Milestone | Scope | State |
|---|---|---|
| 1 | Binary loader (`ctf_loader`), `--dump-info`, asset extraction | done |
| 2 | Macroquad window, texture upload, frame/layer/instance rendering, PageUp/PageDown frame switching | done |
| 3 | Object runtime (positions, values, animations, movements) and event interpreter | done |
| 4 | Audio playback, Counter/Score/Lives/String rendering | done |
| 5 | Extension objects (INI, Global Store X), sandboxed save files, end-of-frame events | done |
| 6 | Pixel-accurate and rotated/scaled collisions, backdrop obstacles, platform movement, frame transitions, paste into background | done |
| 7 | Perspective distortion effect (panorama, perspective, sine wave / offset) | done |
| 8 | Multimedia Fusion 1.5 games (older compression, headers, objects, events, banks) | done |
| 9 | More extensions (Date & Time, Cursor), scenario regression tests, performance, audio pan / resume, more ink effects | done |

### What the loader reads today
- `.exe` (PE overlay → optional `wwww` pack → `PAME`/`PAMU` game data) and bare `.ccn`
- Multimedia Fusion 1.5 data (runtime 0x0301): its deflate variant, smaller headers, 1.5 object / event layouts, image, sound and font banks
- ANSI (`PAME`) and Unicode (`PAMU`) builds, including the encrypted chunks used by newer runtimes (build > 284)
- Application header, app strings, extension list, global values/strings
- Object definitions (handle, type, name, ink effect) and their properties: backdrop images, Quick Backdrop shapes, the common object header and animations
- Frames: header, layers, placed instances, virtual rect, raw event block
- Image bank, decoded to RGBA (24/16/15-bit and 32-bit modes, alpha planes, RLE, LZX)
- Sound and music banks (WAV/OGG/MP3/MIDI detected by magic number) and the font bank

### What runs today
The event interpreter runs each frame's event sheet at the game's frame rate:
- **Object selection** as in Fusion: conditions narrow down which instances the actions apply to. Qualifiers, negated conditions and OR blocks are supported.
- **Conditions**: start of frame, always/never, compare two values, only-one-action-when-looping, run-once, repeat, restrict-for, chance, every X / timer comparisons, key pressed / held / any key, mouse click / in zone / over object / object clicked, player keys, overlap and collision, compare X/Y, alterable values and strings, counters and global values, flags, visibility, animation playing / finished / frame, object count, pick random, zones, playfield and window-border tests, groups and fast loops ("on loop").
- **Actions**: set position / X / Y, create and shoot objects, destroy, show / hide, force and restore animation / frame / direction / speed, start / stop, reverse / bounce, look at, alterable values, strings and flags, counters, strings, semi-transparency, alpha and RGB coefficients, scale and angle, z-order and layers, global values / strings, groups, fast loops, frame jumps (next / previous / jump / restart / end), center display, frame background colour, timer, score / lives, cursor.
- **Expressions**: arithmetic and string operators, about 60 system functions (Random, Str$, Val, Left$/Mid$/Right$, Min/Max, Sin/Cos/Atan2, Distance/Angle, LoopIndex, ...), and object, counter, timer, mouse and frame expressions.
- **Movements**: Static, Bouncing Ball, 8 Directions (player 1 keys), Mouse, Path, Platform (walking, gravity, jumping, solid obstacles, jump-through platforms, small slopes), plus animation playback with speeds, looping and "back to" frames. "Stop" in a collision event moves an object back out of the obstacle.
- **Collisions**: pixel-accurate (per-image alpha masks), with bounding boxes for "collision box" objects and scaled or rotated sprites. Backdrop obstacles and pasted backdrops answer "overlapping a backdrop".
- **Transitions**: frame fade-in / fade-out with logic paused while they play: fade, door and band wipes, centre-out and scroll covers; other effects play as fades.
- **Effects**: the Perspective object warps whatever is drawn beneath it (panorama, perspective, sine wave, sine offset) with a GPU shader. Ink effects: semi-transparency, tint / alpha coefficients, add, subtract, invert, and AND / OR / XOR approximated with multiply / screen / difference blending.

### Audio and HUD objects
- **Sound**: play sample (once, looped N times, on a channel), stop / pause / resume sample, channel or all, main / sample / channel volume, and the "sample / channel not playing" conditions. All bank formats (WAV PCM/ADPCM, OGG Vorbis, MP3) are decoded with `symphonia` and streamed: each playing sound has a decoder thread that stays a fraction of a second ahead of KlikCore's mixer (`cpal` output), so long music never sits in memory as decoded PCM. Pan, volume, resume after pause and "set position" are applied while streaming; frequency changes and the MIDI music bank are not supported yet. `--mute` turns sound off.
- **Counters, Score, Lives**: digit images, horizontal / vertical bars, image-per-value ("animation") and text display, with their min / max clamping. Score and Lives show the player's score and lives.
- **String objects**: their paragraph text, colour and alignment, with word wrap. Game data only names fonts (face, size, weight), so the runtime looks the face up among the system's installed fonts (Windows `Fonts` folders, common Linux / macOS folders) and falls back to a default font.

### Extensions and save files
Extension objects (`.mfx` files) are reimplemented natively in `src/runtime/ext/`:
- **Ini** (`kcini`): current file / group / item, set and get values and strings, delete items and groups. A file name without a folder lives in `%APPDATA%\MMFApplications\<name>`, as with the original object.
- **Date & Time** (`kcclock`): reading the local date and time.
- **Cursor** (`KcCursor`): using an Active object's image as the mouse pointer.
- **Windows controls** (`kcedit`, `kclist`, `kcbutton`, `kccombo`): edit boxes, lists, buttons and combo boxes drawn
  inside the game. Click an edit box and type; lists load / save line files through the save sandbox.
- **Platform helpers** with no visible effect on PC (window control, Discord status, Android / XNA stubs) are accepted.
- **Global Store X** (`gstorex`): the integer store, with set / add / subtract / set range / clear, compare, and save / load in its binary file format.
- **Array** (`kcarray`): numeric or text arrays in up to three dimensions (growing on write), indexes, read / write
  at X / X,Y / X,Y,Z, clear, and load / save through the save sandbox.
- **Control X** (`ctrlx`): "key pressed" by key name, for games that let players rebind keys.
- **File** (`kcfile`): file / folder exists, create a folder or an empty file (in the save sandbox), and the user's Documents folder.
- **Joypad object** (`joystick`): reported as a connected controller with nothing pressed, which keeps such games'
  keyboard controls working; gamepads still work through KlikCore's own controller support.
- **ControlIME** (Japanese text input mode) is accepted and does nothing.

Games write their files through a sandbox. Writes go to `%APPDATA%\KlikCore\<game>\` (override with `--save-dir`), split into `app\` for paths under the game folder and `appdata\` for AppData paths. Reads check the sandbox first and then the real location, so saves from the original game are picked up but never modified. On Windows that includes the VirtualStore (`%LOCALAPPDATA%\VirtualStore`), where old games' saves to protected places such as `C:\` really end up.

To move progress between KlikCore and the original game, open a game's **Settings** in the launcher and use the **Saves** row: *Use original game's saves* replaces KlikCore's copies with the original game's, *Copy to original game* does the reverse (each needs a second click). Whatever gets replaced or removed is first copied to `backups\<time>\` in the game's KlikCore folder. From the command line: `klikcore <GAME> --saves status|import|export`. The save manager knows a game's files from its KlikCore folder and from the original files the game has read (listed in `original-files.txt`).

Anything not implemented yet is counted rather than crashing: see the HUD or `--headless`. Extension objects (`ext:...`) are the most common gap.

### Getting started
Run `build.cmd` (Windows) or `./build.sh` (Linux / Steam Deck). It builds KlikCore and puts **`KlikCore.exe`**
(or `KlikCore`) in this folder: double-click it to open the launcher. Re-running the script while KlikCore is open is
fine. (`cargo build --release` alone leaves the program in `target/release/`.)

### Launcher
Run `klikcore` without a game to open the launcher: a grid of game covers with the selected game's details beside it.
Covers and banners come from Steam's library artwork when the game is a Steam game, otherwise they are generated from
the game's own images (cached in `%APPDATA%\KlikCore\art\`; *Refresh artwork* in a game's menu makes them again).
Type to search; arrow keys / D-pad move, Enter / A plays, Y opens settings. **+ Add** adds a game file, a folder of
games (also `klikcore --add FILE_OR_FOLDER`) or Steam games; the **...** menu has Steam games, the play log and the data
folder. A game's own **...** menu has Add to Steam, Desktop shortcut (also `klikcore GAME --shortcut`), Mods folder,
Open game folder and Remove from library (the files are untouched). Double-click a cover to play. When a game runs another program (launchers that start the real game), a Clickteam game opens in a new KlikCore window; on Windows other programs start as they would with the original runtime. Only one copy
of a game runs at a time, and while a game started from the launcher is running, the launcher ignores the controller.
`KLIKCORE_LAUNCHER_SHOT=file.png` saves the launcher window once artwork has loaded and exits (for testing; `KLIKCORE_LAUNCHER_PAGE=settings|steam|log` opens a page first).

**Steam** (shown when Steam is installed; also `klikcore GAME --add-to-steam`) adds the game to Steam as a non-Steam
shortcut that starts it with KlikCore, with the game's icon. If the game is or was a Steam game (installed through
Steam, or copied out of a Steam library into a folder named after it), its store artwork (capsule, header, hero and
logo) is copied from Steam's cache; other games get artwork made from the game itself (its most detailed
full-screen picture with its icon and title). The shortcut is named "<title> (KlikCore)" and tagged for a "KlikCore"
collection. Steam only reads its shortcuts when it starts and may overwrite the file while
open, so a running Steam is closed and started again around the change (in Steam's Gaming Mode, where that isn't
possible, the shortcut appears after the next restart). The original file is kept as `shortcuts.vdf.klikcore-backup`. Release builds of the launcher also add **KlikCore itself** to
Steam when it isn't there yet: quietly when Steam is closed, otherwise the status bar offers it (Add / Not now /
Don't ask again, saved as `add_self_to_steam=never` in `launcher.ini`). Its shortcut gets KlikCore's own artwork (portrait
and wide capsules, a honeycomb hero background and the logo); an older shortcut (made under the Rekindle name) is
renamed when Steam is closed, and its artwork is replaced whenever KlikCore's changes.

**Steam games** finds Clickteam Fusion games installed through Steam (all Steam libraries; on Linux also Flatpak
Steam) and adds them, either linked where Steam installed them or copied into a storage folder you choose (so they
keep working after uninstalling from Steam). Copies from Steam are marked "(Steam)" in the library. From the command
line, `klikcore --find-steam` lists them.

### Devices and save sync
KlikCore on several devices on the same network (PCs, Steam Deck, phone) can pair and keep their saves in sync. Open
**... > Devices and sync** in the launcher: other devices running KlikCore appear under *On this network*; press
**Pair** and check that both screens show the same 6-digit code. Paired devices then sync saves automatically when the
launcher opens and when you close a game: files only one device has are copied, the newer version of a changed file wins,
deletions are followed, and anything replaced or deleted is backed up to `<game>/backups/sync-<time>/` first. A game
that is running on either device is skipped. **Games** on a paired device lists its library and copies a game here
(into `<data dir>/games/`), which is how games get onto a phone.

Broadcasts don't cross between subnets (for example a PC on `192.168.1.x` and a phone on `192.168.2.x`): use **Add by address** on the Devices page with the other device's address (each device shows its own there); paired devices then find each other directly. Devices find each other with UDP broadcasts on port 48631 and talk over TCP port 48632 (allow KlikCore through the
firewall when asked). Pairing is an X25519 key exchange confirmed by comparing the code; after that every connection is
authenticated and encrypted (ChaCha20-Poly1305) with the paired key. Only paired devices can read saves or game files.
Without the launcher: `klikcore --sync list`, `--sync pair ID`, `--sync sync`, `--sync games ID`, `--sync fetch ID` (with `KLIKCORE_FETCH=<number>`), `--sync serve`.

Play time is logged per session in `%APPDATA%\KlikCore\playtime.tsv`; the launcher shows each game's total and last
play, and **Play log** lists every session. **Open folder** opens the KlikCore folder with saves, the play log and each
game's `klikcore.log`. A game's own "end application" action closes its window.

### Updates
Release builds of the launcher check GitHub for a newer release when they start and offer an **Update to vX** button
(download, swap in, restart). On Android the button downloads the new APK and Android asks to confirm the install
(the first time also to allow installs from KlikCore); the app then restarts as the new version. Android releases are
signed with one permanent key, so updates install over the app with games and saves kept. Copies run from a Cargo
`target` folder never update themselves; set
`KLIKCORE_NO_UPDATE_CHECK=1` to turn the check off. Releases are built by GitHub Actions when a `v*` tag is pushed
(see `.github/workflows/release.yml`).

### Linux and Steam Deck
Download `KlikCore-x86_64.AppImage` (with icon and app-menu entry; on the Steam Deck, Gear Lever adds it to the menu
in one click) or the plain `klikcore-linux-x86_64` from the releases page, make it executable (`chmod +x ...`) and
run it; on the Deck, do that in desktop mode and add it to Steam as a non-Steam game to use it in game mode. Steam
Input's gamepad template works with KlikCore's controller support (see Controllers). Add games with "Add folder..." (uses the
desktop's own picker, kdialog or zenity) or `klikcore-linux-x86_64 --add ~/Games`.

### Android
An APK for 64-bit ARM phones and tablets (`klikcore-android-arm64.apk`) is attached to each release. To build it
yourself you need the Android SDK (build-tools and a platform), the NDK, a JDK (17 or newer) and
`rustup target add aarch64-linux-android`; then run

    python scripts/android/build.py            # target/android/klikcore-debug.apk
    python scripts/android/build.py --release  # target/android/klikcore-release.apk

The script finds the tools through `ANDROID_HOME`, `ANDROID_NDK_HOME` and `JAVA_HOME` (or their usual install
folders), builds the crate's library for Android, and packs it with miniquad's Java activity into a signed APK (a
debug key is made in `target/android/` on first use; `--abi arm64-v8a,armeabi-v7a,x86_64` builds more ABIs). Install
it with `adb install target/android/klikcore-debug.apk`.

There is no file picker on Android: on start the launcher adds the Fusion games it finds in
`Android/data/com.takobyte.klikcore/files/games/` (reachable from a PC over USB, or
`adb push MyGame /sdcard/Android/data/com.takobyte.klikcore/files/games/`) and in the app's private
`files/KlikCore/games/` folder. Saves, settings and the library live in the app's private `files/KlikCore/` folder.
Games run inside the app; touch acts as the mouse, Back is Escape, and Back twice within a second returns to the
launcher. Games played with the keyboard get on-screen controls: the keys a game tests are found in its events and
movements, arrow keys (or W A S D) become a D-pad at the bottom left and up to six other keys, most used first, become
buttons at the bottom right (several can be held at once; sliding between buttons works). Each frame is classed as
played with the controls or point-and-click: frames where the player steers a movement or holds direction keys get
the controls, otherwise the frame's key tests are weighed against its pointer use (clicks, mouse-over, reading the
pointer position, mouse-following objects, buttons and lists), ignoring keys and pointer use shared by most frames
(global shortcuts, cursor objects). Point-and-click frames hide the controls and taps are the mouse.
`KLIKCORE_TOUCH_CONTROLS=debug` prints each frame's verdict. The small button at the top right hides
or shows them (remembered per game, `touch_controls=auto|on|off` in the game's settings file). On a PC,
`KLIKCORE_TOUCH_CONTROLS=1` shows them too (the mouse acts as a finger). Sound uses Android's audio (Oboe). Not available on Android: controllers, file dialogs, desktop
shortcuts, Steam, and games that start Windows programs.

### Controllers
Games that only support keyboard and mouse can be played with a gamepad (Xbox / PlayStation controllers, Steam Deck).
On by default; per game in the launcher's **Settings** (on/off, left stick keys: automatic / arrow keys / W A S D,
cursor speed).

The layout comes from the game, the same way as the Android touch controls: KlikCore reads the game's events for the
keys it uses. The D-pad and left stick press its direction keys (arrows, W A S D or the player's own keys) and A, B,
X, Y, RB, LB press its other keys, most used first (Enter, if the game uses it, on A). Each frame is also classed as
played with keys or with the pointer: in pointer frames (menus, point-and-click screens) A clicks and the left stick
moves the cursor. A legend along the bottom shows the buttons whenever the layout changes. Games without keys get:

| Control | Becomes |
|---|---|
| Left stick / D-pad | cursor / arrow keys |
| Right stick | an on-screen mouse cursor (moving the real mouse takes over again) |
| RT (RB), LT (LB) | left click, right click |
| A, Start, B, X, Y | click, Enter, Space, Ctrl, Shift |
| Back / Select | Escape |

**Steam Input**: Steam (Steam Deck, or any controller when a game starts from Steam) gives games a virtual Xbox
controller. On Windows it does that through XInput, so KlikCore reads XInput first and only falls back to other
controllers (PlayStation and others without Steam) when XInput has none; on Linux Steam's virtual controller is
preferred over the real one. Steam's default gamepad layout works as is, and a layout changed in Steam applies too.

Remap buttons in the game's settings file (`%APPDATA%\KlikCore\settings\<game>.ini`) with lines like `pad_south=32`
(button: `south east west north start select lb rb lt rt l3 r3 up down left right`; value: a Windows key code,
`click`, `rclick` or `none`). The launcher itself works with a controller too: up / down to pick a game, A to play,
Y for its settings, B to close a panel.

### Mods (replacement packs)
Images and sounds can be replaced without touching the game. Each game has a mods folder,
`%APPDATA%\KlikCore\mods\<game file name>\` (the launcher's **Mods** button creates and opens it, with a how-to):

```text
mods\MyGame\imagesS.png              replaces image 123
mods\MyGame\HD pack\imagesS.png      ...or inside any pack folder (packs apply in alphabetical order, later wins)
mods\MyGame\sounds%.wav               replaces sound 45 (.ogg / .mp3 too)
```

Find the numbers with `klikcore GAME --extract DIR` (exported files such as `123_name.png` can be dropped back in as
they are). Replacement images are drawn at the original size, so higher-resolution images just look sharper, and
collisions still use the original images, so gameplay is unchanged.

### Display and errors
- **Fullscreen**: games that ask for fullscreen at start get it; `--fullscreen` / `--windowed` override that, and
  Alt+Enter or F11 toggles while playing. Fullscreen is borderless (a window covering the screen, no display mode
  change, so switching away is instant); on Windows it covers the monitor the window is on, and leaving it puts the
  window back where it was. Your last choice is remembered per game in `klikcore.ini` in the save folder.
- **Per-game settings** (launcher: **Settings** on a game; stored in `%APPDATA%\KlikCore\settings\<game>.ini`):
  fullscreen (game default / on / off), window size (auto, 1x-3x), scaling (fit keeps the shape, integer keeps pixels
  crisp, stretch fills the window), upscaling filter (sharp pixels, smooth, enhanced bicubic, HD: bicubic with halo-free edge sharpening for drawn art, or pixel art: Scale2x-style diagonal smoothing), and whether the debug
  HUD shows at start. In game: F9 cycles the filter, F10 the scaling, F11 / Alt+Enter toggles fullscreen.
- **Errors**: if something inside the game logic fails, the tick is skipped, a red notice appears and the details go to
  `klikcore.log` in the save folder; the game pauses after repeated failures. Unsupported features hit during play are
  logged there too, which makes the file useful for bug reports.

### Viewer
Many games cover a frame with a fade or transition object that events remove. When paused (or with `--static`), you can toggle layers to see underneath.

| Key | Action |
|---|---|
| F1 | overlay: off, compact (fps, frame, state) or full (game, performance with a frame-time graph, view, layers, unsupported features, keys) |
| F2 | pause / resume the game (paused: arrows / drag scroll freely) |
| F3 | single tick while paused |
| F4 | inspector: name, position, size, animation, values, strings and flags of the object under the mouse |
| F5 | restart frame |
| F6 / F7 / F8 | bounding boxes / show hidden objects / fit whole frame |
| F9 / F10 | upscaling filter / scaling mode |
| F11 or Alt+Enter | fullscreen / window |
| Ctrl+PgUp / Ctrl+PgDn | previous / next frame |
| Ctrl+1 ... Ctrl+0 | toggle layers 1-10 |
| Ctrl+F12 | screenshot (`%APPDATA%\KlikCore\screenshots\`) |
| Ctrl+- / Ctrl+= | game speed x0.25 ... x4 |
| Ctrl+M | mute / unmute |
| Ctrl+Q | quit (all other keys go to the game) |

### Regression scenarios
Game files never go in this repository, so test scripts live next to your games. A scenario
(`*.scn`) names a game, presses keys at given ticks, runs, and checks the result:

```text
game ../games/My Game.exe
press 50 13            # Enter at tick 50
click 400 320 240      # left-click at window position (320, 240) at tick 400 (also `mouse`, `rclick`, `hold`)
run 900
expect_frame title     # by name or index; also expect_visited, expect_save, expect_no_unimplemented
expect "door" value 0 = 1   # object state: counter | value N | x | y | visible
dump door              # debug: print objects whose name contains "door"
sweep 200              # run every frame for 200 ticks
```

Run them with `klikcore --scenario FILE_OR_DIR`, or as part of `cargo test` by setting
`KLIKCORE_SCENARIOS` to the folder (without it the game tests are skipped). See `src/runtime/scenario.rs`.

Debug environment variables: `KLIKCORE_PROFILE=1` makes `--headless` print the slowest events;
`KLIKCORE_NO_EFFECTS=1` disables distortion effects for comparison. `KLIKCORE_TRACE=N` prints each condition result of event N every tick.
`KLIKCORE_HUD=compact|full|inspect` shows the overlay (also in screenshots). `KLIKCORE_WALK_FRAMES=MS` shows every frame for MS milliseconds and exits (a memory benchmark; not logged as play time).
`KLIKCORE_SETTINGS="filter=hd;window_scale=2"` overrides player settings for one run (also screenshots). With `--dump-info --verbose`, `KLIKCORE_ALL_PARAS=1` lists every paragraph of text objects and `KLIKCORE_EXT_DATA=1` prints each extension object's stored settings in hex and `KLIKCORE_ANIMS=1` lists each object's animations; with `--dump-events`, `KLIKCORE_FULL_PARAMS=1` prints parameters in full.
`KLIKCORE_TEXTURE_MB=N` changes the texture cache budget (default 192 MB): textures are kept in a least-recently-used
cache, and whatever the current frame draws is never dropped.

## CLI

```
klikcore <GAME>                         run the game
          [--frame N] [--mute] [--save-dir DIR] [--static] [--fit] [--show-hidden] [--hide-layer N]...
          [--screenshot out.png [--ticks N]]   render (after N ticks) to a PNG and exit
klikcore <GAME> --headless N [--press TICK:VK]...   simulate N ticks without a window
klikcore <GAME> --sweep N               run every frame N ticks and list unsupported features
klikcore <GAME> --saves status         list save files (import / export copy them; backups kept)
klikcore <GAME> --probe                 try every condition / action / expression of every frame once
                                        and list the unsupported ones with the frames that use them
klikcore <GAME> --dump-events N         print frame N's event sheet
klikcore --scenario FILE_OR_DIR         run regression scenarios
klikcore <GAME> --screenshot DIR --script FILE   play a scripted input file, saving a PNG at each `shot TICK name.png`
klikcore <GAME> --fullscreen / --windowed       start fullscreen or in a window
klikcore <GAME> --dump-info [-v]
klikcore <GAME> --extract DIR
```
- `--headless` prints frame changes and the unimplemented features that were hit. `--press 50:13` presses Enter (VK 13) at tick 50.
- `--dump-info`: summary of the application, assets, objects and frames. Add `-v` to also list every object, layer, chunk and warning.
- `--extract DIR`: writes `DIR/images/<handle>.png` and `DIR/sounds/<handle>_<name>.<ext>`.

## Layout

```
src/ctf_loader/   format parser (library, no rendering deps)
  exe.rs          finding game data inside an .exe / .ccn
  chunk.rs        chunk lists, zlib, decryption
  old_inflate.rs  Multimedia Fusion 1.5 decompressor
  header.rs       application header
  objects.rs      object definitions, ink effects, extension list
  properties.rs   per-type object properties (backdrops, shapes, ObjectCommon, animations)
  frame.rs        frames, layers, instances
  images.rs       image bank + pixel decoding
  sounds.rs       sound / music banks
  fonts.rs        font bank
  events.rs       event sheets: groups, conditions, actions, parameters, expression tokens
src/runtime/      object runtime + event interpreter (no rendering deps)
  mod.rs          frame lifecycle, tick loop, draw list
  events.rs       conditions / actions with object selection
  expr.rs         expression evaluator
  instance.rs     instance state, animation, movements
  value.rs        Fusion values (int / float / string)
  audio.rs        logical sample / channel playback state
  files.rs        sandboxed save-file access
  save_sync.rs    save manager: import / export between KlikCore and the original game
  ext/            native extension implementations (ini.rs, gsx.rs, clock.rs)
  scenario.rs     scripted regression scenarios
src/audio.rs      decoding any bank sound, streamed block by block (symphonia), and resampling
src/render/       Macroquad window: input mapping, drawing, debug HUD
  text.rs         system font lookup, word wrap, alignment
  audio_out.rs    the mixer: a streaming decoder thread per playing sound, cpal output
src/main.rs       CLI;  src/dump.rs, src/extract.rs
docs/FORMAT.md    format notes (what we know and how it was verified)
reference/python/ original exploratory Python parsers (structural reference only)
```

## Clean-room and legal notes
- Never commit game files or anything extracted from them. `.gitignore` excludes `*.exe`, `*.ccn`, audio files and the local `Tools/` folder.
- Format knowledge comes from inspecting user-supplied files and from publicly documented community research. No Clickteam code is used.

## Assets and licences
KlikCore's own assets live in `assets/`: the icon (`assets/icon/`, drawn by `make_icon.py`, which writes the SVG design,
the PNG and the Windows `.ico`) and the launcher's UI font, [Inter](https://github.com/rsms/inter), under the SIL Open
Font License 1.1 (`assets/fonts/Inter-LICENSE.txt`). No game files or other copyrighted content are included.
