//! The document JSON VectorCraft hands an exporter (its native `.vectorcraft` format) → the
//! pieces to cut, in document points, and what was left out.
//!
//! Paths, compound paths, groups, sublayers, Intertwine groups and symbol instances are cut.
//! Hidden objects, guides, template and non-printing layers are left out, and so is what a cutter
//! can't cut or this plug-in can't see the shape of (type, images, live blends and the like), each
//! counted for a warning.

use std::collections::HashMap;

use serde_json::Value;

use crate::geom::{Affine, Piece, Pt, Seg, SubPath};

/// How deep symbols may nest in one another (deeper art is left out).
const MAX_NEST: u32 = 32;

/// How top-level objects become pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    /// Each top-level object is one piece: a group and its contents stay together on the mat.
    Together,
    /// Each path or compound path is a piece of its own.
    Separately,
}

pub struct Art {
    pub pieces: Vec<Piece>,
    pub skipped: Skipped,
}

/// Art left out of the file, by reason.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Skipped {
    pub text: usize,
    pub images: usize,
    pub live: usize,
    pub placed: usize,
    pub dimensions: usize,
    pub unknown: usize,
    pub unreadable: usize,
    pub missing_symbols: usize,
    /// Clip groups, whose clipped art is cut whole and whose clipping path is left out.
    pub clip_groups: usize,
    /// Objects cut without the live effects that change their shape.
    pub reshaped: usize,
    pub nonprinting_layers: Vec<String>,
}

impl Skipped {
    /// One line per reason, for the user.
    pub fn warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        let mut left_out = |n: usize, one: &str, many: &str, why: &str| {
            if n > 0 {
                w.push(format!("Left out {n} {}: {why}", if n == 1 { one } else { many }));
            }
        };
        left_out(self.text, "type object", "type objects", "outline type first (Type › Create Outlines).");
        left_out(self.images, "image", "images", "a cutting machine cuts paths only.");
        left_out(self.live, "blend, envelope, mesh or repeat", "blends, envelopes, meshes or repeats", "expand them first (Object › Expand…).");
        left_out(self.placed, "placed document", "placed documents", "expand or embed the art first.");
        left_out(self.dimensions, "dimension", "dimensions", "dimensions are annotations, not cut lines.");
        left_out(self.missing_symbols, "symbol instance", "symbol instances", "its symbol is missing from the document.");
        left_out(self.unknown, "object", "objects", "this plug-in doesn't know that kind of object.");
        left_out(self.unreadable, "path", "paths", "their points could not be read.");
        if self.clip_groups > 0 {
            w.push(format!(
                "{} clipping {}: the clipped art is cut whole, and the clipping paths are left out. Release them (Object › Clipping Mask › Release) or use Pathfinder to cut only what shows.",
                self.clip_groups,
                if self.clip_groups == 1 { "mask" } else { "masks" }
            ));
        }
        if self.reshaped > 0 {
            w.push(format!(
                "{} {} live effects that change the shape: they are cut without them. Apply them first (Object › Expand Appearance).",
                self.reshaped,
                if self.reshaped == 1 { "object has" } else { "objects have" }
            ));
        }
        if !self.nonprinting_layers.is_empty() {
            let names: Vec<String> = self.nonprinting_layers.iter().map(|n| format!("“{n}”")).collect();
            w.push(format!("Left out the non-printing {} {}.", if names.len() == 1 { "layer" } else { "layers" }, names.join(", ")));
        }
        w
    }
}

/// The pieces of `doc` (the `document` object of the native file), in paint order.
pub fn collect(doc: &Value, grouping: Grouping) -> Art {
    let symbols = list(&doc["symbols"]).iter().filter_map(|s| Some((s["name"].as_str()?, &s["art"]))).collect();
    let mut walker = Walker { symbols, grouping, pieces: Vec::new(), skipped: Skipped::default() };
    for layer in list(&doc["layers"]) {
        walker.layer(layer);
    }
    Art { pieces: walker.pieces, skipped: walker.skipped }
}

struct Walker<'a> {
    symbols: HashMap<&'a str, &'a Value>,
    grouping: Grouping,
    pieces: Vec<Piece>,
    skipped: Skipped,
}

impl<'a> Walker<'a> {
    fn layer(&mut self, layer: &'a Value) {
        let kind = &layer["kind"];
        if !visible(layer) || flag(kind, "template") {
            return;
        }
        if kind["printable"].as_bool() == Some(false) {
            let name = layer["name"].as_str().unwrap_or("Layer");
            self.skipped.nonprinting_layers.push(name.to_string());
            return;
        }
        for child in list(&kind["children"]) {
            if child["kind"]["type"].as_str() == Some("layer") {
                self.layer(child);
            } else {
                self.top(child);
            }
        }
    }

    /// A top-level object: one piece, or one per shape in it.
    fn top(&mut self, node: &'a Value) {
        let mut shapes = Vec::new();
        self.shapes(node, &Affine::IDENTITY, 0, &mut shapes);
        match self.grouping {
            Grouping::Together => {
                let piece: Piece = shapes.into_iter().flatten().collect();
                if !piece.is_empty() {
                    self.pieces.push(piece);
                }
            }
            Grouping::Separately => self.pieces.extend(shapes.into_iter().filter(|s| !s.is_empty())),
        }
    }

    /// Each path and compound path in `node`, in paint order, as its subpaths mapped by `xf`.
    fn shapes(&mut self, node: &'a Value, xf: &Affine, depth: u32, out: &mut Vec<Piece>) {
        if !visible(node) || depth > MAX_NEST {
            return;
        }
        if reshaped(node) {
            self.skipped.reshaped += 1;
        }
        let kind = &node["kind"];
        match kind["type"].as_str().unwrap_or_default() {
            "path" => {
                // Guides don't print, and a clipping path doesn't show (its group is counted).
                if flag(kind, "guide") || flag(kind, "clipping") {
                    return;
                }
                match subpaths(&kind["path"], xf) {
                    Some(sp) => out.push(sp),
                    None => self.skipped.unreadable += 1,
                }
            }
            "compound" => {
                let mut shape = Vec::new();
                for member in list(&kind["children"]).iter().filter(|m| visible(m)) {
                    match subpaths(&member["kind"]["path"], xf) {
                        Some(sp) => shape.extend(sp),
                        None => self.skipped.unreadable += 1,
                    }
                }
                out.push(shape);
            }
            "group" | "intertwine" | "layer" => {
                if node.get("dimension").is_some_and(|d| !d.is_null()) {
                    self.skipped.dimensions += 1;
                    return;
                }
                if flag(kind, "clip") {
                    self.skipped.clip_groups += 1;
                }
                for child in list(&kind["children"]) {
                    self.shapes(child, xf, depth + 1, out);
                }
            }
            "symbolinstance" => {
                let art = kind["symbol"].as_str().and_then(|name| self.symbols.get(name).copied());
                match (art, affine(&kind["xf"])) {
                    (Some(art), Some(inst)) => self.shapes(art, &inst.then(xf), depth + 1, out),
                    (None, _) => self.skipped.missing_symbols += 1,
                    (_, None) => self.skipped.unreadable += 1,
                }
            }
            "text" => self.skipped.text += 1,
            "image" => self.skipped.images += 1,
            "blend" | "envelope" | "mesh" | "repeat" => self.skipped.live += 1,
            "placeddocument" => self.skipped.placed += 1,
            _ => self.skipped.unknown += 1,
        }
    }
}

/// The subpaths of a native path (`{"subpaths": [{"anchors": [{p, in?, out?}], "closed"}]}`)
/// mapped by `xf`; subpaths of fewer than two anchors have nothing to cut and are dropped.
/// `None` when a point can't be read.
fn subpaths(path: &Value, xf: &Affine) -> Option<Vec<SubPath>> {
    let mut out = Vec::new();
    for sp in path["subpaths"].as_array()? {
        let anchors = sp["anchors"].as_array()?.iter().map(anchor).collect::<Option<Vec<Anchor>>>()?;
        let (Some(first), Some(last)) = (anchors.first(), anchors.last()) else { continue };
        if anchors.len() < 2 {
            continue;
        }
        let closed = sp["closed"].as_bool().unwrap_or(false);
        let mut segs: Vec<Seg> = anchors.windows(2).filter_map(|w| Some(segment(w.first()?, w.get(1)?))).collect();
        if closed {
            segs.push(segment(last, first));
        }
        out.push(SubPath { start: first.p, segs, closed }.map(xf));
    }
    Some(out)
}

struct Anchor {
    p: Pt,
    handle_in: Option<Pt>,
    handle_out: Option<Pt>,
}

/// `{"p": [x, y], "in"?: [x, y], "out"?: [x, y]}` (or the `{x, y}` points of version 1 files).
fn anchor(a: &Value) -> Option<Anchor> {
    let handle = |key: &str| match a.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(v) => point(v).map(Some),
    };
    Some(Anchor { p: point(&a["p"])?, handle_in: handle("in")?, handle_out: handle("out")? })
}

/// From `a` to `b`: a line unless either has a handle on that side.
fn segment(a: &Anchor, b: &Anchor) -> Seg {
    if a.handle_out.is_none() && b.handle_in.is_none() {
        Seg::Line(b.p)
    } else {
        Seg::Cubic(a.handle_out.unwrap_or(a.p), b.handle_in.unwrap_or(b.p), b.p)
    }
}

fn point(v: &Value) -> Option<Pt> {
    let (x, y) = match v {
        Value::Array(xy) => (xy.first()?.as_f64()?, xy.get(1)?.as_f64()?),
        Value::Object(_) => (v["x"].as_f64()?, v["y"].as_f64()?),
        _ => return None,
    };
    let p = Pt::new(x, y);
    p.is_finite().then_some(p)
}

/// `[a, b, c, d, e, f]`.
fn affine(v: &Value) -> Option<Affine> {
    let xs = v.as_array()?;
    let mut m = [0.0; 6];
    for (slot, x) in m.iter_mut().zip(xs) {
        *slot = x.as_f64().filter(|x| x.is_finite())?;
    }
    (xs.len() == 6).then_some(Affine(m))
}

/// Whether `node` has a visible live effect that changes its geometry (distortions, warps, path
/// effects, 3D, plug-in effects); looks such as shadows and glows don't change what is cut.
fn reshaped(node: &Value) -> bool {
    list(&node["appearance"]["effects"]).iter().any(|e| {
        let id = e["id"].as_str().unwrap_or_default();
        e["visible"].as_bool() != Some(false)
            && (["distort.", "warp.", "path.", "threeD.", "plugin."].iter().any(|p| id.starts_with(p))
                || matches!(id, "stylize.roundCorners" | "stylize.scribble"))
    })
}

fn visible(node: &Value) -> bool {
    node["visible"].as_bool() != Some(false)
}

fn flag(kind: &Value, key: &str) -> bool {
    kind[key].as_bool() == Some(true)
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}
