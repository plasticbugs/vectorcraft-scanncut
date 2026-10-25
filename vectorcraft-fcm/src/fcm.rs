//! Brother ScanNCut FCM cut files (format `0100`): the encoder.
//!
//! The layout and every constant written here are described in `FCM-FORMAT.md`. Coordinates are
//! 0.01 mm, y down, from the top-left corner of the mat's cutting area. The header values are those
//! of a file the machine is known to cut.

use crate::geom::{Piece, Pt, Seg, SubPath, bounds};
use crate::thumbnail;

/// Mat units (0.01 mm) per inch.
pub const UNITS_PER_INCH: f64 = 2540.0;

/// Path tool flags.
pub const PATH_OPEN: u32 = 0x0001;
pub const TOOL_CUT: u32 = 0x0002;
pub const TOOL_DRAW: u32 = 0x0004;

const CONTENT_ID: u32 = 400_000_002;
const GENERATOR: &[u8; 4] = b"1PLG";
const GENERATOR_VERSION: u32 = 0;
const FILE_TYPE_CUT: u32 = 0x10;
const SEAM_ALLOWANCE: u32 = 2000;
/// Seam allowance setting prohibited: what every plain cut piece carries.
const RESTRICTIONS: u32 = 0x0004;
/// The rhinestone-diameter slot of a closed path (f32 0.5); open paths write 0.
const CLOSED_DIAMETER: u32 = 0x3F00_0000;
/// Thumbnail block size, as Brother's software writes it for an 88 × 88 thumbnail.
const THUMB_BLOCKS: [u8; 2] = [3, 3];
const RUN_LINES: u32 = 0;
const RUN_CUBICS: u32 = 1;
/// Farthest a coordinate may be from the mat's origin: 100 m, far past any mat, and well inside
/// an `i32`.
const MAX_COORD: f64 = 10_000_000.0;
/// How far past a whole unit a piece's size may be before it rounds up (0.00001 mm).
const SIZE_TOLERANCE: f64 = 1e-3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mat {
    Inches12x12,
    Inches12x24,
}

impl Mat {
    fn id(self) -> u32 {
        match self {
            Mat::Inches12x12 => 0,
            Mat::Inches12x24 => 1,
        }
    }

    /// The cutting area in mat units, as Brother's software writes it.
    pub fn cut_area(self) -> (u32, u32) {
        match self {
            Mat::Inches12x12 => (29_667, 29_880),
            Mat::Inches12x24 => (29_667, 60_360),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mat::Inches12x12 => "12 x 12 in",
            Mat::Inches12x24 => "12 x 24 in",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub mat: Mat,
    /// [`TOOL_CUT`], [`TOOL_DRAW`] or both (the machine then cuts or draws, as chosen on it).
    pub tools: u32,
}

/// The FCM file of `pieces`, in mat units. Subpaths without segments and pieces without
/// subpaths are left out; it is an error when nothing is left.
pub fn encode(pieces: &[Piece], opts: &Options) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();
    for piece in pieces {
        if let Some(bytes) = encode_piece(piece, opts.tools)? {
            encoded.push(bytes);
        }
    }
    if encoded.is_empty() {
        return Err("Nothing to cut.".into());
    }
    let count = u16::try_from(encoded.len()).map_err(|_| format!("Too many pieces for one FCM file: {} (at most 65,535).", encoded.len()))?;

    let mut out = Vec::new();
    out.extend_from_slice(b"#FCM0100");
    put_u32(&mut out, CONTENT_ID);
    let header = variable_header(&thumbnail::render(pieces, opts.mat))?;
    put_u32(&mut out, len32(&header)?);
    out.extend_from_slice(&header);

    let (cut_w, cut_h) = opts.mat.cut_area();
    for v in [FILE_TYPE_CUT, opts.mat.id(), cut_w, cut_h, SEAM_ALLOWANCE] {
        put_u32(&mut out, v);
    }

    put_u32(&mut out, u32::from(count));
    let mut offset = 0u32;
    for piece in &encoded {
        put_u32(&mut out, offset);
        offset = offset.checked_add(len32(piece)?).ok_or("The file would be over 4 GB.")?;
    }
    put_u32(&mut out, offset);
    put_u32(&mut out, u32::from(count));
    for id in 0..count {
        out.extend_from_slice(&id.to_le_bytes());
    }
    for piece in &encoded {
        out.extend_from_slice(piece);
    }
    Ok(out)
}

/// Names, thumbnail and generator: the part of the header that starts with its own length.
fn variable_header(bmp: &[u8]) -> Result<Vec<u8>, String> {
    let mut v = vec![0u8; 8]; // short name: empty, NUL padded
    for _ in 0..3 {
        // long name, author, copyright: one space each, as UTF-16 with a count of code units
        v.push(1);
        v.extend_from_slice(&u16::from(b' ').to_le_bytes());
    }
    v.extend_from_slice(&THUMB_BLOCKS);
    put_u32(&mut v, len32(bmp)?);
    v.extend_from_slice(bmp);
    v.extend_from_slice(GENERATOR);
    put_u32(&mut v, GENERATOR_VERSION);
    Ok(v)
}

/// One piece: its points relative to the centre of its exact bounds, which its transform moves
/// back into place. `None` when it has nothing to cut.
fn encode_piece(piece: &Piece, tools: u32) -> Result<Option<Vec<u8>>, String> {
    let paths: Vec<&SubPath> = piece.iter().filter(|sp| !sp.segs.is_empty()).collect();
    let Some([x0, y0, x1, y1]) = bounds(paths.iter().copied()) else { return Ok(None) };
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) {
        return Err("A path lies too far from the artboard to be cut (over 100 m away), or has invalid coordinates.".into());
    }
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    // Within ±MAX_COORD, so the casts can't saturate.
    let local = |p: Pt| [(p.x - cx).round() as i32, (p.y - cy).round() as i32];

    // Rounded up, as Brother's software does, but not past a whole number by float noise alone.
    let size = |v: f64| (v - SIZE_TOLERANCE).ceil().max(0.0) as u32;
    let mut out = vec![0u8; 8];
    put_u32(&mut out, size(x1 - x0));
    put_u32(&mut out, size(y1 - y0));
    put_u32(&mut out, 1); // has a transform
    for v in [1.0, 0.0, 0.0, 1.0, cx as f32, cy as f32] {
        out.extend_from_slice(&f32::to_le_bytes(v));
    }
    for v in [0, 0, RESTRICTIONS, 4, 0] {
        // expansion and reduction limits, restrictions, an empty 4-byte label
        put_u32(&mut out, v);
    }
    put_u32(&mut out, len32(&paths)?);
    for sp in paths {
        let path = encode_path(sp, tools, &local)?;
        put_u32(&mut out, len32(&path)?);
        out.extend_from_slice(&path);
    }
    Ok(Some(out))
}

fn encode_path(sp: &SubPath, tools: u32, local: &impl Fn(Pt) -> [i32; 2]) -> Result<Vec<u8>, String> {
    let mut segs = sp.segs.clone();
    if sp.closed && segs.last().map(|s| local(s.end())) != Some(local(sp.start)) {
        // A closed path ends exactly on its start: there is no "close" in the format.
        segs.push(Seg::Line(sp.start));
    }
    // Runs of one kind of segment, as long as they go.
    let mut runs: Vec<(u32, Vec<Seg>)> = Vec::new();
    for seg in segs {
        let kind = match seg {
            Seg::Line(_) => RUN_LINES,
            Seg::Cubic(..) => RUN_CUBICS,
        };
        match runs.last_mut() {
            Some((k, run)) if *k == kind => run.push(seg),
            _ => runs.push((kind, vec![seg])),
        }
    }

    let mut out = Vec::new();
    put_u32(&mut out, 4);
    put_u32(&mut out, tools | if sp.closed { 0 } else { PATH_OPEN });
    put_u32(&mut out, len32(&runs)?);
    put_u32(&mut out, 0); // no rhinestones
    put_u32(&mut out, if sp.closed { CLOSED_DIAMETER } else { 0 });
    put_point(&mut out, local(sp.start));
    for (kind, run) in &runs {
        put_u32(&mut out, *kind);
        put_u32(&mut out, len32(run)?);
        for seg in run {
            match *seg {
                Seg::Line(p) => put_point(&mut out, local(p)),
                Seg::Cubic(c1, c2, p) => {
                    for q in [c1, c2, p] {
                        put_point(&mut out, local(q));
                    }
                }
            }
        }
    }
    Ok(out)
}

fn len32<T>(items: &[T]) -> Result<u32, String> {
    u32::try_from(items.len()).map_err(|_| "Too much to fit in an FCM file.".to_string())
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_point(out: &mut Vec<u8>, [x, y]: [i32; 2]) {
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
}
