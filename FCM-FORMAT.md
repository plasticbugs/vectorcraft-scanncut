# Brother ScanNCut FCM format (version 0100)

What a writer needs to produce FCM cut files that Brother's ScanNCut machines and CanvasWorkspace
read. Sources:

- The public, MPL-2.0 Rust library [justjanne/fcmlib](https://github.com/justjanne/fcmlib), used
  only as a description of the format. No code was copied.
- An independent parser (`tools/fcmdump.py`) run on fcmlib's 1,127 sample files and on one
  privately owned file (not included). Every file parses with every byte accounted for.
- That file's EPS counterpart (also not included), to tie FCM coordinates to an artboard. The
  FCM's generator is `1PLG`; with the EPS made in Illustrator, it is presumably an Illustrator
  plug-in export (called "plug-in" below).

`tools/fcmwrite.py` is a small reference writer that follows the rules below. Its test sheet
(`samples/vectorcraft-test-sheet.fcm`) cuts correctly on a ScanNCut.

## Conventions

| Item         | Value                                                                    |
| ------------ | ------------------------------------------------------------------------ |
| Byte order   | little-endian everywhere                                                 |
| Length unit  | 0.01 mm (1 inch = 2540 units; 1 pt = 35.2778 units)                      |
| Axes         | x right, **y down**; origin at the top-left of the mat / cut area        |
| Point        | `i32 x, i32 y` (8 bytes)                                                 |
| Strings      | short name: 8 bytes ASCII, NUL padded; others: `u8 n` + `n` UTF-16LE units |

## File layout

```
FileHeader  CutData  PieceTable
```

### FileHeader

| Size  | Field             | Value written by Brother's software                               |
| ----- | ----------------- | ----------------------------------------------------------------- |
| 4     | variant           | `#FCM` (one sample is `#VCM`; same layout)                    |
| 4     | version           | `0100`                                                            |
| u32   | content_id        | `400000001` (web), `400000002` (desktop app, `1PLG`); scans made on the machine use `1009xxxxx` |
| u32   | var_header_len    | bytes from the next field to the end of the header                |
| 8     | short_name        | NUL-padded; empty from the desktop app                            |
| u8+…  | long_name         | UTF-16; the desktop app and plug-in write `" "`                   |
| u8+…  | author            | `" "`                                                             |
| u8+…  | copyright         | `" "`                                                             |
| u8    | thumb_block_w     | `3`                                                               |
| u8    | thumb_block_h     | `3` (one file in the corpus has `4`, with a 78×128 thumbnail)     |
| u32   | thumbnail_len     |                                                                   |
| n     | thumbnail         | a Windows BMP, see [Thumbnail](#thumbnail)                        |
| 4+u32 | generator         | `1WEB` v100/v200, `1APP` v206, `1PLG` v0; scans: `u32 device, u32 version` |
| u32?  | print_to_cut      | present only in Print-and-Cut files (`1`)                         |

### CutData

| Size | Field          | Value                                                                  |
| ---- | -------------- | ---------------------------------------------------------------------- |
| u32  | file_type      | `0x10` cut; `0x38` print-and-cut (then AlignmentData follows)          |
| u32  | mat_id         | `0` = 12×12″ mat, `1` = 12×24″ mat                                     |
| u32  | cut_width      | CanvasWorkspace: `29667` (both mats). Plug-in: artboard width          |
| u32  | cut_height     | CanvasWorkspace: `29880` (12×12″) or `60360` (12×24″). Plug-in: artboard height |
| u32  | seam_allowance | `2000` (current software) or `700` (older web exports)                 |
| …    | AlignmentData  | print-and-cut only: `u32 needed`, `u32 n`, `n × Point` (registration marks) |

All art in every sample sits inside `[0, cut_width] × [0, cut_height]`.

### PieceTable

| Size      | Field                                            |
| --------- | ------------------------------------------------ |
| u32       | n pieces                                         |
| n × u32   | byte offset of each piece, from the piece data   |
| u32       | total length of the piece data                   |
| u32       | n again                                          |
| n × u16   | piece ids: always `0, 1, 2, …`                   |
| …         | piece data: the pieces back to back, no padding  |

A **piece** is one object the machine moves and selects as a unit: a glyph, a compound path, a
group of dashes. Its paths are the subpaths.

### Piece

| Size       | Field              | Value                                                         |
| ---------- | ------------------ | ------------------------------------------------------------- |
| 8          | reserved           | zero                                                          |
| u32        | width              | bounding box of the **true curve** (with cubic extrema), in units, rounded up |
| u32        | height             | likewise                                                      |
| u32        | has_transform      | always `1`                                                    |
| 6 × f32    | transform          | `a b c d e f`; mat = (a·x + c·y + e, b·x + d·y + f). Unrotated pieces: `1 0 0 1 cx cy` with (cx, cy) the centre of the bounding box, so the piece's points are centred on 0 |
| u32        | expansion_limit    | `0`                                                           |
| u32        | reduction_limit    | `0`                                                           |
| u32        | restrictions       | `0x4` (seam-allowance setting prohibited) on normal pieces; `0x0` on seam-allowance pieces |
| u32 + 4    | label              | length `4`, then `00 00 00 00`, or `01` + 3 ASCII (`A01`, …) in Brother's pattern sets |
| u32        | n paths            |                                                               |
| n × (u32 len + Path) | paths   |                                                               |

### Path

| Size    | Field               | Value                                                           |
| ------- | ------------------- | --------------------------------------------------------------- |
| u32     | tool_len            | `4`                                                             |
| u32     | tool                | flags, see below                                                |
| u32     | n runs              | number of segment runs                                          |
| u32     | n rhinestones       | `0` except rhinestone paths                                     |
| u32     | rhinestone diameter | rhinestone paths: the diameter. Otherwise `0x3F000000` (f32 0.5) on closed paths and `0` on open ones, in every sample |
| Point   | start               | (only if n runs > 0)                                            |
| runs    | segment runs        | `u32 kind` (0 = lines, 1 = cubic Béziers), `u32 n`, then `n` endpoints, or `n × (c1, c2, end)` |
| points  | rhinestones         | `n × Point`                                                     |

Runs are **maximal**: no two adjacent runs share a kind (checked over 103,877 runs). A closed
path's last segment ends exactly on its start point; there is no "close" opcode.

### Tool flags

| Bit      | Meaning                    | Notes                                               |
| -------- | -------------------------- | --------------------------------------------------- |
| `0x0001` | open path                  |                                                     |
| `0x0002` | cut                        |                                                     |
| `0x0004` | draw                       | `0x6`/`0x7` (cut + draw) is the default for plain lines: the machine picks |
| `0x0008` | seam allowance             | fabric pattern outlines                             |
| `0x0010` | rhinestone                 |                                                     |
| `0x0020` | fill                       |                                                     |
| `0x0040` | auto align                 |                                                     |
| `0x1000` | draw only                  | with `0x4`                                          |
| `0x2000` | emboss                     |                                                     |
| `0x4000` | foil                       |                                                     |
| `0x8000` | perforating                |                                                     |

Common values: `0x6`/`0x7` (cut + draw), `0x2`/`0x3` (cut only), `0x4`/`0x5` (draw only),
`0xE`/`0xF` (with seam allowance), `0x1004`/`0x1025` (draw-only lines and fills).

## Thumbnail

A bottom-up, 1-bit `BI_RGB` BMP: 40-byte `BITMAPINFOHEADER`, `biSizeImage = 0`, 2 palette
entries `00 00 00 FF` (index 0, black ink) and `FF FF FF FF` (index 1, white). Rows are padded to
4 bytes, with padding bits zero.

| Writer               | Size    | Pixels per metre |
| -------------------- | ------- | ---------------- |
| CanvasWorkspace web  | 92×100  | 3780             |
| CanvasWorkspace app  | 88×88   | 3780, 5669 or 7559 |
| Illustrator plug-in  | 88×88   | 3780             |

The thumbnail shows the **whole mat**, not a crop of the design: about 357 units (3.57 mm) per
pixel, with the mat origin near pixel (2, 3). Lines are drawn 1 pixel wide.

## Writer checklist

1. Convert to 0.01 mm, y down, origin at the cut area's top-left.
2. One piece per top-level object (or compound path / group); one path per subpath.
3. Turn quadratic curves into cubics. Close closed subpaths with a final segment to the start.
4. Group consecutive segments of one kind into a run.
5. Piece box = the true bounding box; points are integers relative to its centre; the transform is
   `1 0 0 1 cx cy` with the centre as f32.
6. Tool: `0x6` closed, `0x7` open (or the cut-only / draw-only variants). Diameter slot `0x3F000000`
   closed, `0` open.
7. Restrictions `0x4`, label empty, ids `0..n`.
8. Draw the thumbnail at mat scale.

## Open questions (check on a machine)

- Whether the machine checks `generator` or `content_id` (scans use other values, so maybe not).
- Whether it uses `cut_width` / `cut_height` (CanvasWorkspace always writes the mat area; the
  plug-in writes the artboard).
- How it treats a design that runs past the mat area.
