"""Minimal FCM (Brother ScanNCut) writer, used to check the writer rules in FCM-FORMAT.md.

Geometry is given in millimetres, y down, origin at the top-left of the artboard. Each piece is a
list of subpaths; a subpath is (start, segments, closed) where a segment is ('L', (x, y)) or
('C', (x1, y1), (x2, y2), (x, y)).

    python3 -I tools/fcmwrite.py out.fcm      # writes the built-in test sheet
"""
import math
import struct
import sys

UNITS_PER_MM = 100  # FCM coordinates are 0.01 mm

# Path tool flags
PATH_OPEN = 0x0001
TOOL_CUT = 0x0002
TOOL_DRAW = 0x0004

# Header values copied from the known-good sample (Brother's Illustrator plug-in output)
CONTENT_ID = 400000002
GENERATOR = (b'1PLG', 0)
FILE_TYPE_CUT = 0x10
SEAM_ALLOWANCE = 2000
RESTRICTIONS = 0x0004  # PROHIBITION_OF_SEAM_ALLOWANCE_SETTING, on every normal piece
CLOSED_DIAMETER = 0x3F000000  # f32 0.5 in the rhinestone-diameter slot of closed paths; open paths write 0

# Thumbnail: 88x88 1-bit BMP, about 3.57 mm per pixel, mat origin at pixel (2, 3)
THUMB_W = THUMB_H = 88
THUMB_UNITS_PER_PX = 357
THUMB_ORIGIN_PX = (2, 3)


def write_fcm(pieces, artboard_mm, op=TOOL_CUT | TOOL_DRAW):
    cut_w = int(artboard_mm[0] * UNITS_PER_MM)
    cut_h = int(artboard_mm[1] * UNITS_PER_MM)
    encoded = [encode_piece(p, op) for p in pieces]
    out = bytearray(b'#FCM0100')
    out += u32(CONTENT_ID)
    out += var_header(thumbnail(pieces))
    out += u32(FILE_TYPE_CUT) + u32(0) + u32(cut_w) + u32(cut_h) + u32(SEAM_ALLOWANCE)
    out += piece_table(encoded)
    return bytes(out)


def var_header(bmp):
    v = bytearray(8)  # short name: 8 bytes, NUL padded
    for s in (' ', ' ', ' '):  # long name, author, copyright: u8 count + UTF-16LE
        units = s.encode('utf-16-le')
        v += bytes([len(units) // 2]) + units
    v += bytes([3, 3])  # thumbnail block size
    v += u32(len(bmp)) + bmp
    v += GENERATOR[0] + u32(GENERATOR[1])
    return u32(len(v)) + v


def piece_table(encoded):
    out = bytearray(u32(len(encoded)))
    off = 0
    for e in encoded:
        out += u32(off)
        off += len(e)
    out += u32(off) + u32(len(encoded))
    for i in range(len(encoded)):
        out += struct.pack('<H', i)
    for e in encoded:
        out += e
    return out


def encode_piece(subpaths, op):
    pts = [to_units(p) for p in bbox_points(subpaths)]
    x0, x1 = min(p[0] for p in pts), max(p[0] for p in pts)
    y0, y1 = min(p[1] for p in pts), max(p[1] for p in pts)
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    local = lambda p: (round(p[0] * UNITS_PER_MM - cx), round(p[1] * UNITS_PER_MM - cy))
    out = bytearray(8)
    out += u32(math.ceil(x1 - x0)) + u32(math.ceil(y1 - y0))
    out += u32(1) + struct.pack('<6f', 1, 0, 0, 1, cx, cy)
    out += u32(0) + u32(0) + u32(RESTRICTIONS)
    out += u32(4) + u32(0)  # no label
    out += u32(len(subpaths))
    for start, segs, closed in subpaths:
        segs = list(segs)
        if closed and segs and segs[-1][-1] != start:
            segs.append(('L', start))  # closed paths end exactly on their start point
        tool = op | (0 if closed else PATH_OPEN)
        runs = []
        for s in segs:  # consecutive segments of one kind share a run
            if runs and runs[-1][0] == s[0]:
                runs[-1][1].append(s)
            else:
                runs.append((s[0], [s]))
        p = bytearray(u32(4) + u32(tool) + u32(len(runs)) + u32(0))
        p += u32(CLOSED_DIAMETER if closed else 0)
        p += pt(local(start))
        for kind, run in runs:
            p += u32(0 if kind == 'L' else 1) + u32(len(run))
            for s in run:
                for q in s[1:]:
                    p += pt(local(q))
        out += u32(len(p)) + p
    return bytes(out)


def bbox_points(subpaths):
    """Every point the piece's true bounding box passes through: anchors plus cubic extrema."""
    for start, segs, _ in subpaths:
        cur = start
        yield cur
        for s in segs:
            if s[0] == 'C':
                yield from cubic_extrema(cur, s[1], s[2], s[3])
            cur = s[-1]
            yield cur


def cubic_extrema(p0, p1, p2, p3):
    for i in (0, 1):
        a = -p0[i] + 3 * p1[i] - 3 * p2[i] + p3[i]
        b = 2 * (p0[i] - 2 * p1[i] + p2[i])
        c = p1[i] - p0[i]
        if abs(a) < 1e-12:
            ts = [-c / b] if abs(b) > 1e-12 else []
        else:
            d = b * b - 4 * a * c
            ts = [(-b + s * math.sqrt(d)) / (2 * a) for s in (1, -1)] if d >= 0 else []
        for t in ts:
            if 0 < t < 1:
                yield cubic_at(p0, p1, p2, p3, t)


def cubic_at(p0, p1, p2, p3, t):
    m = 1 - t
    return tuple(m**3 * p0[i] + 3 * m * m * t * p1[i] + 3 * m * t * t * p2[i] + t**3 * p3[i] for i in (0, 1))


def thumbnail(pieces):
    stride = ((THUMB_W + 31) // 32) * 4
    data_bytes = (THUMB_W + 7) // 8
    rows = [bytearray(b'\xff' * data_bytes + b'\0' * (stride - data_bytes)) for _ in range(THUMB_H)]

    def plot(x, y):
        if 0 <= x < THUMB_W and 0 <= y < THUMB_H:
            rows[y][x >> 3] &= ~(0x80 >> (x & 7))

    def px(p):
        return (round(p[0] * UNITS_PER_MM / THUMB_UNITS_PER_PX + THUMB_ORIGIN_PX[0]),
                round(p[1] * UNITS_PER_MM / THUMB_UNITS_PER_PX + THUMB_ORIGIN_PX[1]))

    for piece in pieces:
        for start, segs, closed in piece:
            cur = start
            for s in list(segs) + ([('L', start)] if closed else []):
                pts = [s[1]] if s[0] == 'L' else [cubic_at(cur, s[1], s[2], s[3], k / 12) for k in range(1, 13)]
                for q in pts:
                    line(px(cur), px(q), plot)
                    cur = q
    body = b''.join(bytes(r) for r in reversed(rows))  # bottom-up
    header = struct.pack('<2sIHHI', b'BM', 62 + len(body), 0, 0, 62)
    info = struct.pack('<IiiHHIIiiII', 40, THUMB_W, THUMB_H, 1, 1, 0, 0, 3780, 3780, 2, 2)
    palette = b'\x00\x00\x00\xff\xff\xff\xff\xff'
    return header + info + palette + body


def line(a, b, plot):
    (x0, y0), (x1, y1) = a, b
    dx, dy = abs(x1 - x0), -abs(y1 - y0)
    sx, sy = (1 if x0 < x1 else -1), (1 if y0 < y1 else -1)
    err = dx + dy
    while True:
        plot(x0, y0)
        if (x0, y0) == (x1, y1):
            return
        e2 = 2 * err
        if e2 >= dy:
            err += dy
            x0 += sx
        if e2 <= dx:
            err += dx
            y0 += sy


def to_units(p):
    return (p[0] * UNITS_PER_MM, p[1] * UNITS_PER_MM)


def u32(v):
    return struct.pack('<I', v)


def pt(p):
    return struct.pack('<ii', *p)


def circle(cx, cy, r):
    k = 0.5522847498 * r
    return ((cx + r, cy), [
        ('C', (cx + r, cy + k), (cx + k, cy + r), (cx, cy + r)),
        ('C', (cx - k, cy + r), (cx - r, cy + k), (cx - r, cy)),
        ('C', (cx - r, cy - k), (cx - k, cy - r), (cx, cy - r)),
        ('C', (cx + k, cy - r), (cx + r, cy - k), (cx + r, cy)),
    ], True)


def test_sheet():
    """Six shapes on a 150 x 150 mm artboard, one per writer rule worth checking on the machine."""
    rect = [((10, 10), [('L', (50, 10)), ('L', (50, 40)), ('L', (10, 40))], True)]
    disc = [circle(80, 25, 15)]
    ring = [circle(125, 25, 15), circle(125, 25, 7)]  # one piece, two subpaths: a washer
    d_shape = [((10, 60), [('L', (30, 60)), ('C', (52, 60), (52, 100), (30, 100)), ('L', (10, 100))], True)]
    zigzag = [((65, 100), [('L', (75, 60)), ('L', (85, 100)), ('L', (95, 60)), ('L', (105, 100))], False)]
    s_curve = [((115, 100), [('C', (150, 100), (110, 60), (140, 60))], False)]
    return [rect, disc, ring, d_shape, zigzag, s_curve]


if __name__ == '__main__':
    with open(sys.argv[1], 'wb') as fh:
        fh.write(write_fcm(test_sheet(), (150, 150)))
