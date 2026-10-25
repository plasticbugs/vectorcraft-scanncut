//! ScanNCut FCM: a VectorCraft exporter plug-in (plug-in ABI v1, `docs/plugins.md` in VectorCraft)
//! that writes the document's paths as a Brother ScanNCut cut file.
//!
//! The chosen artboard's top-left corner is the top-left corner of the mat's cutting area; 1 pt of
//! art is 1 pt on the mat. Each top-level object is a piece the machine moves as one (or each path
//! is, as the user picks), and every path is cut, drawn, or left for the machine to do either.

pub mod document;
pub mod fcm;
pub mod geom;
mod thumbnail;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::document::Grouping;
use crate::fcm::{Mat, Options, TOOL_CUT, TOOL_DRAW, UNITS_PER_INCH};
use crate::geom::{Affine, Piece, bounds};

const MANIFEST: &str = r#"{
  "id": "scanncut-fcm",
  "name": "ScanNCut FCM",
  "version": "0.1.0",
  "kind": "exporter",
  "author": "Scott Moschella",
  "description": "Brother ScanNCut cut files (.fcm) of the document's paths.",
  "extensions": ["fcm"],
  "params": {
    "mat": {"type": "choice", "options": ["12 x 12 in", "12 x 24 in"], "default": "12 x 12 in", "label": "Mat",
            "help": "The mat the design is cut on. The artboard's top-left corner is the top-left corner of the mat's cutting area."},
    "lines": {"type": "choice", "options": ["Cut or draw", "Cut only", "Draw only"], "default": "Cut or draw", "label": "Lines",
              "help": "Cut or draw: choose on the machine whether to cut the lines or draw them with a pen. Cut only and Draw only mark every line for one of them."},
    "pieces": {"type": "choice", "options": ["Groups stay together", "Every path separately"], "default": "Groups stay together", "label": "Pieces",
               "help": "What the machine moves and selects as one piece: each top-level object (a group with everything in it), or each path and compound path."},
    "artboard": {"type": "int", "min": 1, "max": 1000, "default": 1, "label": "Artboard",
                 "help": "The artboard whose top-left corner is the top-left corner of the mat's cutting area. Art anywhere in the document is exported."}
  }
}"#;

/// Native file versions this plug-in was written against.
const KNOWN_VERSION: u64 = 4;
/// Mat units (0.01 mm) per point.
const UNITS_PER_PT: f64 = UNITS_PER_INCH / 72.0;

#[unsafe(no_mangle)]
pub extern "C" fn vc_abi_version() -> u32 {
    1
}

/// `len << 32 | ptr` of the UTF-8 manifest JSON.
#[unsafe(no_mangle)]
pub extern "C" fn vc_manifest() -> u64 {
    pack(MANIFEST.as_bytes())
}

/// A block of `size` bytes the host writes the input and the parameters into. Never freed: every
/// run gets a fresh instance.
#[unsafe(no_mangle)]
pub extern "C" fn vc_alloc(size: u32) -> u32 {
    let mut block = Vec::<u8>::with_capacity(size as usize);
    let ptr = block.as_mut_ptr();
    std::mem::forget(block);
    ptr as u32
}

/// Runs the export: `input` holds the document as native file JSON, `params` the parameters.
/// Returns the output JSON as `len << 32 | ptr`, or a negative error code.
///
/// # Safety
/// The host guarantees both blocks came from `vc_alloc` and hold `*_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vc_run(input: *const u8, input_len: u32, params: *const u8, params_len: u32) -> i64 {
    // SAFETY: see the function's contract.
    let (input, params) = unsafe { (std::slice::from_raw_parts(input, input_len as usize), std::slice::from_raw_parts(params, params_len as usize)) };
    let Ok(bytes) = serde_json::to_vec(&export(input, params)) else { return -2 };
    pack(bytes.leak()) as i64
}

/// The exporter's output for the native file JSON `input` and the parameters JSON `params`:
/// `{"dataBase64", "warnings"}`, or `{"error"}` to decline with a message.
pub fn export(input: &[u8], params: &[u8]) -> Value {
    match write_fcm(input, params) {
        Ok((file, warnings)) => json!({"dataBase64": base64::engine::general_purpose::STANDARD.encode(file), "warnings": warnings}),
        Err(e) => json!({"error": e}),
    }
}

fn write_fcm(input: &[u8], params: &[u8]) -> Result<(Vec<u8>, Vec<String>), String> {
    let file: Value = serde_json::from_slice(input).map_err(|e| format!("The document could not be read: {e}."))?;
    let params: Value = serde_json::from_slice(params).map_err(|e| format!("The export options could not be read: {e}."))?;
    let doc = &file["document"];
    if !doc.is_object() {
        return Err("The document could not be read: it has no `document`.".into());
    }
    let mut warnings = Vec::new();
    if file["version"].as_u64().is_some_and(|v| v > KNOWN_VERSION) {
        warnings.push("This document was saved in a newer format than this plug-in knows; check the file before cutting.".to_string());
    }

    let mat = match params["mat"].as_str() {
        Some("12 x 24 in") => Mat::Inches12x24,
        _ => Mat::Inches12x12,
    };
    let tools = match params["lines"].as_str() {
        Some("Cut only") => TOOL_CUT,
        Some("Draw only") => TOOL_DRAW,
        _ => TOOL_CUT | TOOL_DRAW,
    };
    let grouping = match params["pieces"].as_str() {
        Some("Every path separately") => Grouping::Separately,
        _ => Grouping::Together,
    };

    let to_mat = mat_transform(doc, &params["artboard"])?;
    let art = document::collect(doc, grouping);
    warnings.extend(art.skipped.warnings());
    let pieces: Vec<Piece> = art.pieces.iter().map(|piece| piece.iter().map(|sp| sp.map(&to_mat)).collect()).collect();
    if pieces.iter().all(|p| p.iter().all(|sp| sp.segs.is_empty())) {
        let mut why = "Nothing to cut: the document has no visible paths.".to_string();
        if art.skipped.text > 0 {
            why.push_str(" Outline type first (Type › Create Outlines).");
        }
        return Err(why);
    }

    let (cut_w, cut_h) = mat.cut_area();
    let off_mat = pieces
        .iter()
        .filter_map(bounds)
        .filter(|[x0, y0, x1, y1]| *x0 < 0.0 || *y0 < 0.0 || *x1 > f64::from(cut_w) || *y1 > f64::from(cut_h))
        .count();
    if off_mat > 0 {
        warnings.push(format!(
            "{off_mat} {} past the {} mat's cutting area ({:.1} × {:.1} mm from the artboard's top-left corner).",
            if off_mat == 1 { "piece reaches" } else { "pieces reach" },
            mat.label(),
            f64::from(cut_w) / 100.0,
            f64::from(cut_h) / 100.0,
        ));
    }

    Ok((fcm::encode(&pieces, &Options { mat, tools })?, warnings))
}

/// Document points → mat units, with the chosen artboard's top-left corner at the origin.
fn mat_transform(doc: &Value, artboard: &Value) -> Result<Affine, String> {
    let artboards = doc["artboards"].as_array().map_or(&[][..], Vec::as_slice);
    let n = artboard.as_f64().filter(|n| n.is_finite() && *n >= 1.0).unwrap_or(1.0) as usize;
    let (x0, y0) = match artboards.get(n - 1) {
        Some(a) => (a["rect"]["x0"].as_f64().unwrap_or(0.0), a["rect"]["y0"].as_f64().unwrap_or(0.0)),
        None if artboards.is_empty() => (0.0, 0.0),
        None => return Err(format!("There is no artboard {n}: the document has {}.", artboards.len())),
    };
    Ok(Affine([UNITS_PER_PT, 0.0, 0.0, UNITS_PER_PT, -x0 * UNITS_PER_PT, -y0 * UNITS_PER_PT]))
}

fn pack(bytes: &[u8]) -> u64 {
    ((bytes.len() as u64) << 32) | bytes.as_ptr() as u64
}
