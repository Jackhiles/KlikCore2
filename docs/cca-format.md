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
| 14 | 1+1 | 3, 0x41 (flags) |
| 16 | 2+2 | hotspot x, y |
| 20 | 2+2 | action point x, y |

then the data:

- `u32` decoded size;
- a row table, 8 bytes per row: `u32` offset of the row's run list (from data + 4), `u32`
  position of the row's first opaque pixel in the decoded pixel buffer (plus a constant: the
  first row's value);
- run lists: per row, byte pairs `(transparent pixels to skip, opaque pixels to draw)` until
  the row's width is covered (the last pair draws 0);
- one run-length pixel stream for all opaque pixels: a byte below 0x80 repeats the next byte
  that many times, a byte 0x80+n copies the next n bytes, 0 ends it.

Pixels are indices into the game palette (8-bit).
