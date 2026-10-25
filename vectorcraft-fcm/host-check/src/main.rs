//! `host-check PLUGIN.wasm DOC.vectorcraft OUT.fcm [PARAMS-JSON]`
//!
//! Loads the plug-in with VectorCraft's own host (manifest checks, sandbox, budgets), runs it as an
//! export of the document, prints its warnings and writes the file it returns.

use std::error::Error;
use std::fs;

use base64::Engine as _;
use serde_json::{Value, json};
use vectorcraft_plugins::export::{self, Target};
use vectorcraft_plugins::{Limits, Plugin};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, wasm, doc, out, rest @ ..] = args.as_slice() else {
        return Err("usage: host-check PLUGIN.wasm DOC.vectorcraft OUT.fcm [PARAMS-JSON]".into());
    };
    let plugin = Plugin::load(&fs::read(wasm)?, Limits::default())?;
    let m = plugin.manifest();
    println!("plug-in {} {} ({:?}), {} bytes", m.id, m.version, m.kind, plugin.size());
    let params: Value = match rest.first() {
        Some(p) => serde_json::from_str(p)?,
        None => json!({}),
    };
    let name = std::path::Path::new(doc).file_stem().and_then(|s| s.to_str()).unwrap_or("document");
    let file = fs::read(doc)?;
    let exported = export::run(&plugin, json_part(&file), &params, Target { extension: "fcm", name })?;
    for w in &exported.warnings {
        println!("warning: {w}");
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(&exported.data_base64)?;
    fs::write(out, &bytes)?;
    println!("wrote {out} ({} bytes)", bytes.len());
    Ok(())
}

/// The JSON of a saved file without the data a version 4 file may keep after it, as the app
/// hands it to an exporter (`vectorcraft_format::json_part`).
fn json_part(file: &[u8]) -> &[u8] {
    const BLOB_MAGIC: &[u8] = b"\n\0VCBLOBS\0";
    file.windows(BLOB_MAGIC.len()).position(|w| w == BLOB_MAGIC).map_or(file, |at| &file[..at])
}
