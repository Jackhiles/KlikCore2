# The Click & Create / The Games Factory `.cca` format

Notes for the loader, worked out from game files themselves (offsets in bytes, little-endian).
A game made with Click & Create / The Games Factory ships a small runtime .exe (the
`CNCSA32` player) next to a `.cca` file that holds the whole game.

## File header

| Offset | Size | Meaning |
|---|---|---|
| 0x000 | 4 | `GAME` |
| 0x004 | 2 | version (0x0207 seen) |
| 0x006 | 80 | game name (zero-padded) |
| 0x056 | ~158 | author (zero-padded) |
| 0x0F8 | 2+2 | window width, height |
| 0x100 | 2 | flags |
| 0x108 | 4×2 | control type per player |
| 0x110 | 4×8×2 | keys per player: up, down, left, right, fire 1-4 (virtual-key codes) |
| 0x15C | 2 | frame count N |
| 0x15E | 4×(N+1) | u32 table: entry 0 unknown (0x34), entries 1..N = absolute file offset of each frame |

After the header: the application icon (32×32 4-bit + mask, then 16×16 4-bit + mask), a
preview bitmap (`u16 2, u32 size`, then a Windows DIB: BITMAPINFOHEADER + 256-entry palette +
pixels), the extension list (fixed entries naming `*.COX` files), and the game palette as a
Windows LOGPALETTE (`00 03 00 01` = version 0x300, 256 entries of R, G, B, flags).

## Frames

Each frame starts at its table offset; the u32 before it is its size.

| Offset in frame | Meaning |
|---|---|
| 0 | 2 × i16 (unknown) |
| 4 | width, height (u16) |
| 8 | name (zero-padded; "Untitled" by default) |
| … | settings, then the frame's palette (LOGPALETTE) |
| after palette | a preview thumbnail (`u16 1, u32 size`, DIB, 64×48 8-bit) |
| then | the frame's objects: fixed records (name padded, numeric fields) followed by tagged blocks, e.g. `SPRI` (active objects), `CNTR` (counters), `LIVE` (lives), `SCRE` (score), `SYST` |
| then | events (compact byte code) |

Objects are stored per frame (each frame has its own copies).

### Frame blocks

After the thumbnail, a frame is a run of blocks: `u16 kind, u32 size, payload`.

- **Kind 2, objects:** `u16 count`, then the object records.
- **Kind 3, instances:** `u16 count`, then `count` × `u32` offsets (the drawing order) into
  the records that follow; each record is 32 bytes: `u16 object (id − 1)`, `i16 x`, `i16 y`,
  `u16 type`, type-specific fields, ending in `ff ff`.

- **Kind 0x8004, events:** `"Fra\0"`, `u16` (0x0217 seen), `u32 size`, then event groups in
  the Multimedia Fusion 1.5 layout (KlikCore's `old` event format): `i16 size (negative), u8
  conditions, u8 actions, u16 flags`, 14-byte group header, then ACEs `u16 size, i8 object
  type, i8 number, u16 object, u16 object list, u8 flags, u8 flags2, u8 parameter count, u8
  default type` (+ `u16 identifier` for conditions), then parameters `u16 size, u16 code, data`.
  Condition and action numbers match Multimedia Fusion's (e.g. keyboard −6 / −1 "key pressed",
  game −3 / 4 "end the application").

### Object records

| Offset from the name | Meaning |
|---|---|
| 0 | name (zero-padded, 40 bytes) |
| 40 | 0 |
| 34 | width, height (u16; quick backdrops and text) |
| 42 | type in the low byte: 0 quick backdrop, 1 backdrop, 2 Active, 3 string, 5 score, 6 lives, 7 counter, 32+ extension; the high bits are flags (0x100, 0x400 seen on all kinds; 0x800 = hidden at start). 0xFFF marks records with another layout |
| 44 | instance count |
| 46 | object id (from 1; instances use id − 1) |
| 64 | u32 = the file offset of name + 68 (a reliable signature for finding records) |
| 68 | u16 tag (0x8002 when the block is an image) |
| 70 | u32 size of the properties block that follows at 74 |

Active objects' properties: a few fixed fields, then a movement block `SPRI` (`u32 size`,
movement type: 0 static, 4 ball, …, then its settings, e.g. the speed), then the animations:
  Ball settings are i16s from the movement's offset 10: speed, randomizer, angles, security, deceleration (deceleration matched against a game's Fusion rebuild).

- `u16 size, u16 ?, i16 offsets[16]` (from the start of this header; unused ones negative);
- each animation: `i16 direction offsets[32]` (from the animation's start; unused ones negative);
- each direction: `u8 min speed, u8 max speed, u16 repeat, u16 loop-to frame, u16 frame count,
  u16 image numbers[count]` (image numbers are indices into the image bank).

Then the alterable value names (24 bytes each) and an editor icon (`u16 0x8002, u32 size`, an
image in the codec below). A quick backdrop's block is its fill tile image.

### Text, counters, score and lives

- **String (`TEXT`):** `u16 size, u16 ?, u16 count, u16 offsets[count]` (from the size
  field), then per paragraph `u16 size, u16 font (0xFFFF = as before), u8 r, g, b, ?, u16
  flags` (alignment as in Fusion: 1 centred, 2 right) and the zero-ended text. Later
  paragraphs' colour field often reads `00 00 80 00`: the first paragraph's colour.
- **Counter (`CNTR`):** `u16 size`, then initial, minimum, maximum (i32), then a display block.
- **Score (`SCRE`), lives (`LIVE`):** a display block.
- **Display block:** `u16 size, u16 type, u16 ?`, then for type 1 (digit images: 0-9 and
  '-') and 4 (an icon per life) `u16 count, u16 images[count]`; for 2 / 3 (vertical /
  horizontal bar) `u16 width, u16 ?, r, g, b, ?, u16 height`; 0 = hidden.

## Image bank

Right after the last frame (its offset + its size + 6 bytes): `u32 count`, then `count` pairs
of `u32 offset, u32 size`, offsets counted from the count field.

Each image: a 24-byte header

| Offset | Size | Meaning |
|---|---|---|
| 0 | 2 | checksum |
| 2 | 2 | format (7 or 8 seen) |
| 4 | 2 | 0 |
| 6 | 4 | data size (item size − 24) |
| 10 | 2+2 | width, height |
| 14 | 1+1 | colour mode (3 = palette, 6 = 15-bit colour), flags (0x40 = per-row transparent runs; low bits 1 / 2 = 1 / 2 bytes per pixel, not always set) |
| 16 | 2+2 | hotspot x, y |
| 20 | 2+2 | action point x, y |

then the data:

- `u32` decoded size;
- a row table, 8 bytes per row: `u32` offset of the row's run list (from data + 4), `u32`
  position of the row's first opaque pixel in the decoded pixel buffer (plus a constant: the
  first row's value);
- run lists: per row, byte pairs `(transparent pixels to skip, opaque pixels to draw)` until
  the row's width is covered (the last pair draws 0);
- one run-length pixel stream for all opaque pixels: a byte below 0x80 repeats the next pixel
  that many times, a byte 0x80+n copies the next n pixels, 0 ends it.

Pixels are indices into the game palette (1 byte) or, in colour mode 6, 15-bit colours
(2 bytes, `0RRRRRGGGGGBBBBB`). Embedded images (`u16 0x8002`, size at +12, width / height at
+16 / +18, colour mode and flags at +20 / +21, data at +30) are one stream over the whole
picture, each row padded to an even number of bytes.

## Fonts, music and sounds

After the image bank come three more banks with the same table (`u32 count`, then `count` x
`u32 offset, u32 size` from the count field): fonts (104-byte Windows LOGFONT entries), music
and sounds. A music or sound entry is a 32-byte header (`u32 ?, u16 ?, u32 data size`, the
name in up to 18 bytes, `u32 ?`) and its data: a standard MIDI file for music; for a sound a
16-byte WAVEFORMAT (PCM, e.g. mono 8-bit 11025 / 22050 Hz) followed by the samples. Events
number sounds and music from 0 in bank order.
