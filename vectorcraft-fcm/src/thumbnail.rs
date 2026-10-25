//! The file's preview: a 1-bit BMP of the whole mat with the cut lines on it, which the machine's
//! pattern list and CanvasWorkspace show.

use crate::fcm::Mat;
use crate::geom::{Piece, Pt, Seg, cubic_at};

const WIDTH: usize = 88;
const HEIGHT: usize = 88;
/// Mat units per pixel on a 12 × 12 in mat (the 12 × 24 in mat is drawn at half that scale, so
/// that all of it shows).
const UNITS_PER_PX: f64 = 357.0;
/// Where the mat's origin falls in the picture.
const ORIGIN_PX: Pt = Pt::new(2.0, 3.0);
/// Steps a curve is drawn in.
const CURVE_STEPS: u32 = 12;
/// Lines are drawn after clamping their ends to this many pixels around the picture, which keeps
/// art far off the mat from taking long to draw.
const MARGIN_PX: f64 = 8.0;
const DPM: i32 = 3780; // 96 dpi

/// The BMP of `pieces`, given in mat units.
pub fn render(pieces: &[Piece], mat: Mat) -> Vec<u8> {
    let scale = match mat {
        Mat::Inches12x12 => UNITS_PER_PX,
        Mat::Inches12x24 => UNITS_PER_PX * 2.0,
    };
    let mut canvas = Canvas::new();
    let px = |p: Pt| Pt::new(p.x / scale + ORIGIN_PX.x, p.y / scale + ORIGIN_PX.y);
    for sp in pieces.iter().flatten() {
        let mut cur = sp.start;
        let close = sp.closed.then_some(Seg::Line(sp.start));
        for seg in sp.segs.iter().copied().chain(close) {
            match seg {
                Seg::Line(end) => canvas.line(px(cur), px(end)),
                Seg::Cubic(c1, c2, end) => {
                    let mut prev = cur;
                    for i in 1..=CURVE_STEPS {
                        let q = cubic_at(cur, c1, c2, end, f64::from(i) / f64::from(CURVE_STEPS));
                        canvas.line(px(prev), px(q));
                        prev = q;
                    }
                }
            }
            cur = seg.end();
        }
    }
    canvas.bmp()
}

/// One bit per pixel, rows top first: 1 white, 0 ink.
struct Canvas {
    rows: Vec<Vec<u8>>,
}

impl Canvas {
    fn new() -> Canvas {
        Canvas { rows: vec![vec![0xFF; WIDTH.div_ceil(8)]; HEIGHT] }
    }

    fn line(&mut self, a: Pt, b: Pt) {
        let clamp = |v: f64, max: usize| {
            let v = if v.is_finite() { v } else { 0.0 };
            v.clamp(-MARGIN_PX, max as f64 + MARGIN_PX).round() as i32
        };
        let (mut x, mut y) = (clamp(a.x, WIDTH), clamp(a.y, HEIGHT));
        let (x1, y1) = (clamp(b.x, WIDTH), clamp(b.y, HEIGHT));
        let (dx, dy) = ((x1 - x).abs(), -(y1 - y).abs());
        let (sx, sy) = (if x < x1 { 1 } else { -1 }, if y < y1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.ink(x, y);
            if (x, y) == (x1, y1) {
                return;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn ink(&mut self, x: i32, y: i32) {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else { return };
        if x < WIDTH
            && let Some(byte) = self.rows.get_mut(y).and_then(|row| row.get_mut(x / 8))
        {
            *byte &= !(0x80u8 >> (x % 8));
        }
    }

    /// A bottom-up 1-bit BMP with a black-and-white palette, as Brother's software writes them.
    fn bmp(&self) -> Vec<u8> {
        let stride = WIDTH.div_ceil(32) * 4;
        let pixels_at = 14 + 40 + 8;
        let size = pixels_at + stride * HEIGHT;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(pixels_at as u32).to_le_bytes());
        for v in [40, WIDTH as i32, HEIGHT as i32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&1u16.to_le_bytes()); // bits per pixel
        for v in [0, 0, DPM, DPM, 2, 2] {
            // uncompressed, image size left to the reader, resolution, 2 colours, both important
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        for row in self.rows.iter().rev() {
            out.extend_from_slice(row);
            out.resize(out.len() + stride - row.len(), 0);
        }
        out
    }
}
