#![allow(clippy::unwrap_used, clippy::expect_used)]

use base64::Engine as _;
use serde_json::{Value, json};

/// Mat units per point.
const U: f64 = 2540.0 / 72.0;

mod header {
    use super::*;

    #[test]
    fn matches_the_known_good_file() {
        let f = export_ok(&doc(vec![rect(10.0, 10.0, 72.0, 36.0)]), json!({}));
        assert_eq!(&f.raw[..8], b"#FCM0100");
        assert_eq!(f.content_id, 400_000_002);
        assert_eq!(f.names, ["".to_string(), " ".into(), " ".into(), " ".into()]);
        assert_eq!(f.thumb_blocks, (3, 3));
        assert_eq!(f.generator, (*b"1PLG", 0));
        assert_eq!((f.file_type, f.mat_id, f.cut_w, f.cut_h, f.seam), (0x10, 0, 29_667, 29_880, 2000));
        assert_eq!(f.ids, vec![0]);
    }

    #[test]
    fn thumbnail_is_an_88_pixel_1_bit_bmp_with_the_art_on_it() {
        let f = export_ok(&doc(vec![rect(0.0, 0.0, 300.0, 300.0)]), json!({}));
        let bmp = &f.thumbnail;
        assert_eq!(bmp.len(), 1118);
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32le(bmp, 2) as usize, bmp.len());
        assert_eq!((i32le(bmp, 18), i32le(bmp, 22), u16le(bmp, 28)), (88, 88, 1));
        assert_eq!(&bmp[54..62], &[0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(bmp[62..].iter().any(|&b| b != 0xFF && b != 0), "no ink drawn");
    }

    #[test]
    fn a_12_by_24_mat_has_its_own_id_and_cutting_area() {
        let f = export_ok(&doc(vec![rect(0.0, 0.0, 72.0, 72.0)]), json!({"mat": "12 x 24 in"}));
        assert_eq!((f.mat_id, f.cut_w, f.cut_h), (1, 29_667, 60_360));
    }
}

mod geometry {
    use super::*;

    #[test]
    fn a_rectangle_is_a_closed_run_of_lines_centred_on_its_piece() {
        // One inch from the artboard's corner, an inch wide and half an inch tall.
        let f = export_ok(&doc_at((100.0, 50.0), vec![rect(172.0, 122.0, 72.0, 36.0)]), json!({}));
        let p = only_piece(&f);
        assert_eq!((p.width, p.height), (2540, 1270));
        assert_eq!(p.transform[..4], [1.0, 0.0, 0.0, 1.0]);
        assert_close(p.transform[4] as f64, 3810.0);
        assert_close(p.transform[5] as f64, 3175.0);
        assert_eq!((p.restrictions, p.label), (4, [0; 4]));
        let path = &p.paths[0];
        assert_eq!((path.tool, path.diameter), (0x6, 0x3F00_0000));
        assert_eq!(path.start, [-1270, -635]);
        assert_eq!(path.runs, vec![Run::Lines(vec![[1270, -635], [1270, 635], [-1270, 635], [-1270, -635]])]);
    }

    #[test]
    fn a_circle_is_one_run_of_cubics_that_ends_on_its_start() {
        let f = export_ok(&doc(vec![circle(144.0, 144.0, 36.0)]), json!({}));
        let path = &only_piece(&f).paths[0];
        let [Run::Cubics(segs)] = &path.runs[..] else { panic!("{:?}", path.runs) };
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[3][2], path.start);
        let p = only_piece(&f);
        assert!((p.width as f64 - 72.0 * U).abs() <= 1.0, "{}", p.width);
    }

    #[test]
    fn an_open_path_alternates_runs_and_is_marked_open() {
        let path = node_path(
            vec![json!({"p": [0, 0]}), json!({"p": [72, 0], "out": [100, 0]}), json!({"p": [144, 72], "in": [144, 40]}), json!({"p": [144, 144]})],
            false,
        );
        let f = export_ok(&doc(vec![path]), json!({}));
        let path = &only_piece(&f).paths[0];
        assert_eq!((path.tool, path.diameter), (0x7, 0));
        let kinds: Vec<&str> = path.runs.iter().map(Run::kind).collect();
        assert_eq!(kinds, ["lines", "cubics", "lines"]);
    }

    #[test]
    fn a_symbol_instance_is_cut_where_its_transform_puts_it() {
        let mut d = doc(vec![json!({"id": 9, "kind": {"type": "symbolinstance", "symbol": "Box", "xf": [2, 0, 0, 2, 72, 0]}})]);
        d["document"]["symbols"] = json!([{"name": "Box", "art": rect(0.0, 0.0, 36.0, 36.0)}]);
        let f = export_ok(&d, json!({}));
        let p = only_piece(&f);
        assert_eq!((p.width, p.height), (2540, 2540));
        assert_close(p.transform[4] as f64, 2540.0 + 1270.0);
    }
}

mod pieces {
    use super::*;

    #[test]
    fn a_group_is_one_piece_unless_paths_are_asked_for_separately() {
        let d = doc(vec![group(vec![rect(0.0, 0.0, 36.0, 36.0), rect(72.0, 0.0, 36.0, 36.0)], false)]);
        let together = export_ok(&d, json!({}));
        assert_eq!(together.pieces.len(), 1);
        assert_eq!(together.pieces[0].paths.len(), 2);
        let apart = export_ok(&d, json!({"pieces": "Every path separately"}));
        assert_eq!(apart.pieces.len(), 2);
        assert_eq!(apart.ids, vec![0, 1]);
    }

    #[test]
    fn a_compound_path_stays_one_piece_either_way() {
        let washer = json!({"id": 5, "kind": {"type": "compound", "rule": "evenodd", "children": [circle(100.0, 100.0, 50.0), circle(100.0, 100.0, 20.0)]}});
        for pieces in ["Groups stay together", "Every path separately"] {
            let f = export_ok(&doc(vec![washer.clone()]), json!({"pieces": pieces}));
            assert_eq!(f.pieces.len(), 1, "{pieces}");
            assert_eq!(f.pieces[0].paths.len(), 2, "{pieces}");
        }
    }

    #[test]
    fn sublayers_split_into_pieces_like_their_layer() {
        let sub = json!({"id": 50, "name": "Sub", "kind": {"type": "layer", "color": {"Preset": 1}, "template": false, "printable": true,
            "children": [rect(0.0, 0.0, 10.0, 10.0), rect(20.0, 0.0, 10.0, 10.0)]}});
        assert_eq!(export_ok(&doc(vec![sub]), json!({})).pieces.len(), 2);
    }
}

mod options {
    use super::*;

    #[test]
    fn lines_choose_the_tools() {
        let d = doc(vec![rect(0.0, 0.0, 36.0, 36.0), node_path(vec![json!({"p": [0, 50]}), json!({"p": [50, 50]})], false)]);
        for (lines, closed, open) in [("Cut or draw", 0x6, 0x7), ("Cut only", 0x2, 0x3), ("Draw only", 0x4, 0x5)] {
            let f = export_ok(&d, json!({"lines": lines}));
            assert_eq!((f.pieces[0].paths[0].tool, f.pieces[1].paths[0].tool), (closed, open), "{lines}");
        }
    }

    #[test]
    fn the_chosen_artboard_is_the_origin() {
        let mut d = doc(vec![rect(1000.0, 0.0, 72.0, 72.0)]);
        d["document"]["artboards"] = json!([artboard(0.0, 0.0), artboard(1000.0, 0.0)]);
        let f = export_ok(&d, json!({"artboard": 2}));
        let p = only_piece(&f);
        assert_close(p.transform[4] as f64, 1270.0);
        let out = vectorcraft_fcm::export(d.to_string().as_bytes(), json!({"artboard": 3}).to_string().as_bytes());
        assert_eq!(out["error"], "There is no artboard 3: the document has 2.");
    }
}

mod left_out {
    use super::*;

    #[test]
    fn hidden_guides_and_clipping_paths_are_not_cut() {
        let mut hidden = rect(0.0, 0.0, 10.0, 10.0);
        hidden["visible"] = json!(false);
        let mut guide = rect(0.0, 0.0, 10.0, 10.0);
        guide["kind"]["guide"] = json!(true);
        let mut mask = rect(0.0, 0.0, 50.0, 50.0);
        mask["kind"]["clipping"] = json!(true);
        let clip = group(vec![mask, rect(10.0, 10.0, 100.0, 100.0)], true);
        let (f, warnings) = export_with_warnings(&doc(vec![hidden, guide, clip]), json!({}));
        assert_eq!(f.pieces.len(), 1);
        assert_eq!(f.pieces[0].paths.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("1 clipping mask: the clipped art is cut whole"), "{warnings:?}");
    }

    #[test]
    fn type_images_and_non_printing_layers_are_reported() {
        let mut d = doc(vec![rect(0.0, 0.0, 10.0, 10.0), json!({"id": 30, "kind": {"type": "text"}}), json!({"id": 31, "kind": {"type": "image"}})]);
        let notes = json!({"id": 40, "name": "Notes", "kind": {"type": "layer", "color": {"Preset": 2}, "template": false, "printable": false, "children": [rect(0.0, 0.0, 5.0, 5.0)]}});
        d["document"]["layers"].as_array_mut().unwrap().push(notes);
        let (f, warnings) = export_with_warnings(&d, json!({}));
        assert_eq!(f.pieces.len(), 1);
        assert_eq!(
            warnings,
            [
                "Left out 1 type object: outline type first (Type › Create Outlines).",
                "Left out 1 image: a cutting machine cuts paths only.",
                "Left out the non-printing layer “Notes”.",
            ]
        );
    }

    #[test]
    fn shape_changing_effects_are_reported_and_looks_are_not() {
        let mut roughened = rect(0.0, 0.0, 10.0, 10.0);
        roughened["appearance"] = json!({"effects": [{"id": "distort.roughen", "params": {}, "visible": true}]});
        let mut glowing = rect(20.0, 0.0, 10.0, 10.0);
        glowing["appearance"] = json!({"effects": [{"id": "stylize.outerGlow", "params": {}, "visible": true}]});
        let (_, warnings) = export_with_warnings(&doc(vec![roughened, glowing]), json!({}));
        assert_eq!(warnings, ["1 object has live effects that change the shape: they are cut without them. Apply them first (Object › Expand Appearance)."]);
    }

    #[test]
    fn art_past_the_mat_is_reported() {
        let (_, warnings) = export_with_warnings(&doc(vec![rect(0.0, 0.0, 72.0, 72.0), rect(800.0, 0.0, 72.0, 72.0)]), json!({}));
        assert_eq!(warnings, ["1 piece reaches past the 12 x 12 in mat's cutting area (296.7 × 298.8 mm from the artboard's top-left corner)."]);
    }

    #[test]
    fn nothing_to_cut_declines_with_a_reason() {
        let out = vectorcraft_fcm::export(doc(vec![json!({"id": 3, "kind": {"type": "text"}})]).to_string().as_bytes(), b"{}");
        assert_eq!(out["error"], "Nothing to cut: the document has no visible paths. Outline type first (Type › Create Outlines).");
        let out = vectorcraft_fcm::export(b"not json", b"{}");
        assert!(out["error"].as_str().unwrap().starts_with("The document could not be read"));
    }
}

// ---- documents ----

fn doc(children: Vec<Value>) -> Value {
    doc_at((0.0, 0.0), children)
}

fn doc_at(origin: (f64, f64), children: Vec<Value>) -> Value {
    json!({
        "format": "vectorcraft", "version": 4, "generator": "test",
        "document": {
            "version": 1, "title": "t", "units": "Points",
            "artboards": [artboard(origin.0, origin.1)],
            "layers": [{"id": 1, "name": "Layer 1", "kind": {"type": "layer", "color": {"Preset": 0}, "template": false, "printable": true, "children": children}}],
            "symbols": []
        }
    })
}

fn artboard(x: f64, y: f64) -> Value {
    json!({"id": 1, "name": "Artboard", "rect": {"x0": x, "y0": y, "x1": x + 612.0, "y1": y + 792.0}})
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Value {
    node_path(vec![json!({"p": [x, y]}), json!({"p": [x + w, y]}), json!({"p": [x + w, y + h]}), json!({"p": [x, y + h]})], true)
}

fn circle(cx: f64, cy: f64, r: f64) -> Value {
    let k = 0.552_284_75 * r;
    node_path(
        vec![
            json!({"p": [cx + r, cy], "in": [cx + r, cy - k], "out": [cx + r, cy + k]}),
            json!({"p": [cx, cy + r], "in": [cx + k, cy + r], "out": [cx - k, cy + r]}),
            json!({"p": [cx - r, cy], "in": [cx - r, cy + k], "out": [cx - r, cy - k]}),
            json!({"p": [cx, cy - r], "in": [cx - k, cy - r], "out": [cx + k, cy - r]}),
        ],
        true,
    )
}

fn node_path(anchors: Vec<Value>, closed: bool) -> Value {
    json!({"id": 2, "kind": {"type": "path", "path": {"subpaths": [{"anchors": anchors, "closed": closed}]}}})
}

fn group(children: Vec<Value>, clip: bool) -> Value {
    json!({"id": 3, "kind": {"type": "group", "children": children, "clip": clip}})
}

// ---- running the export ----

fn export_ok(d: &Value, params: Value) -> Fcm {
    export_with_warnings(d, params).0
}

fn export_with_warnings(d: &Value, params: Value) -> (Fcm, Vec<String>) {
    let out = vectorcraft_fcm::export(d.to_string().as_bytes(), params.to_string().as_bytes());
    assert!(out.get("error").is_none(), "{out}");
    let bytes = base64::engine::general_purpose::STANDARD.decode(out["dataBase64"].as_str().unwrap()).unwrap();
    let warnings = out["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    (Fcm::read(bytes), warnings)
}

fn only_piece(f: &Fcm) -> &Piece {
    assert_eq!(f.pieces.len(), 1);
    &f.pieces[0]
}

fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() < 0.01, "{a} != {b}");
}

// ---- an FCM reader, written from FCM-FORMAT.md independently of the encoder ----

struct Fcm {
    raw: Vec<u8>,
    content_id: u32,
    names: [String; 4],
    thumb_blocks: (u8, u8),
    thumbnail: Vec<u8>,
    generator: ([u8; 4], u32),
    file_type: u32,
    mat_id: u32,
    cut_w: u32,
    cut_h: u32,
    seam: u32,
    ids: Vec<u16>,
    pieces: Vec<Piece>,
}

struct Piece {
    width: u32,
    height: u32,
    transform: [f32; 6],
    restrictions: u32,
    label: [u8; 4],
    paths: Vec<Path>,
}

struct Path {
    tool: u32,
    diameter: u32,
    start: [i32; 2],
    runs: Vec<Run>,
}

#[derive(Debug, PartialEq)]
enum Run {
    Lines(Vec<[i32; 2]>),
    Cubics(Vec<[[i32; 2]; 3]>),
}

impl Run {
    fn kind(&self) -> &'static str {
        match self {
            Run::Lines(_) => "lines",
            Run::Cubics(_) => "cubics",
        }
    }
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        s
    }
    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn pt(&mut self) -> [i32; 2] {
        [self.i32(), self.i32()]
    }
    fn utf16(&mut self) -> String {
        let n = self.u8() as usize;
        String::from_utf16(&(0..n).map(|_| self.u16()).collect::<Vec<_>>()).unwrap()
    }
}

impl Fcm {
    fn read(raw: Vec<u8>) -> Fcm {
        let mut c = Cursor { b: &raw, at: 8 };
        let content_id = c.u32();
        let header_len = c.u32() as usize;
        let header_end = c.at + header_len;
        let short = String::from_utf8(c.take(8).iter().copied().take_while(|&b| b != 0).collect()).unwrap();
        let names = [short, c.utf16(), c.utf16(), c.utf16()];
        let thumb_blocks = (c.u8(), c.u8());
        let n = c.u32() as usize;
        let thumbnail = c.take(n).to_vec();
        let generator = (c.take(4).try_into().unwrap(), c.u32());
        assert_eq!(c.at, header_end, "header length");
        let (file_type, mat_id, cut_w, cut_h, seam) = (c.u32(), c.u32(), c.u32(), c.u32(), c.u32());
        let count = c.u32() as usize;
        let offsets: Vec<usize> = (0..count).map(|_| c.u32() as usize).collect();
        let total = c.u32() as usize;
        assert_eq!(c.u32() as usize, count);
        let ids = (0..count).map(|_| c.u16()).collect();
        let base = c.at;
        assert_eq!(base + total, raw.len(), "piece data runs to the end of the file");
        let pieces = offsets
            .iter()
            .enumerate()
            .map(|(i, &off)| {
                let end = offsets.get(i + 1).copied().unwrap_or(total);
                let mut c = Cursor { b: &raw, at: base + off };
                let piece = read_piece(&mut c);
                assert_eq!(c.at, base + end, "piece {i} length");
                piece
            })
            .collect();
        Fcm { content_id, names, thumb_blocks, thumbnail, generator, file_type, mat_id, cut_w, cut_h, seam, ids, pieces, raw }
    }
}

fn read_piece(c: &mut Cursor) -> Piece {
    assert_eq!(c.take(8), &[0; 8]);
    let (width, height) = (c.u32(), c.u32());
    assert_eq!(c.u32(), 1, "has a transform");
    let transform = [c.f32(), c.f32(), c.f32(), c.f32(), c.f32(), c.f32()];
    assert_eq!((c.u32(), c.u32()), (0, 0), "limits");
    let restrictions = c.u32();
    assert_eq!(c.u32(), 4, "label length");
    let label = c.take(4).try_into().unwrap();
    let paths = (0..c.u32())
        .map(|_| {
            let len = c.u32() as usize;
            let end = c.at + len;
            let path = read_path(c);
            assert_eq!(c.at, end, "path length");
            path
        })
        .collect();
    Piece { width, height, transform, restrictions, label, paths }
}

fn read_path(c: &mut Cursor) -> Path {
    assert_eq!(c.u32(), 4, "tool length");
    let tool = c.u32();
    let run_count = c.u32();
    assert_eq!(c.u32(), 0, "rhinestones");
    let diameter = c.u32();
    let start = c.pt();
    let runs: Vec<Run> = (0..run_count)
        .map(|_| match (c.u32(), c.u32()) {
            (0, n) => Run::Lines((0..n).map(|_| c.pt()).collect()),
            (1, n) => Run::Cubics((0..n).map(|_| [c.pt(), c.pt(), c.pt()]).collect()),
            (k, _) => panic!("run kind {k}"),
        })
        .collect();
    for pair in runs.windows(2) {
        assert_ne!(pair[0].kind(), pair[1].kind(), "adjacent runs share a kind");
    }
    if tool & 1 == 0 {
        let end = match runs.last().unwrap() {
            Run::Lines(p) => *p.last().unwrap(),
            Run::Cubics(s) => s.last().unwrap()[2],
        };
        assert_eq!(end, start, "a closed path ends on its start");
    }
    Path { tool, diameter, start, runs }
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn i32le(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
