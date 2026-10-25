# ScanNCut FCM plug-in: developer notes

A VectorCraft exporter plug-in (WebAssembly plug-in ABI v1) that writes Brother ScanNCut cut files.
Installing and using it is covered in the [main README](../README.md); the file format is described in
[`FCM-FORMAT.md`](../FCM-FORMAT.md).

## Layout

| File               | What it does                                                                  |
| ------------------ | ----------------------------------------------------------------------------- |
| `src/lib.rs`       | Manifest, ABI exports (`vc_manifest`, `vc_alloc`, `vc_run`), options, warnings |
| `src/document.rs`  | VectorCraft's native document JSON → the pieces to cut, and what was skipped  |
| `src/fcm.rs`       | The FCM encoder                                                               |
| `src/thumbnail.rs` | The file's 88 × 88 preview of the mat                                         |
| `src/geom.rs`      | Subpaths, affine maps, exact curve bounds                                     |
| `tests/export.rs`  | Exports test documents and reads the bytes back with an independent reader     |
| `host-check/`      | Runs the built plug-in through VectorCraft's own plug-in host                 |

## Build and test

```sh
rustup target add wasm32-unknown-unknown
./build.sh            # dist/scanncut-fcm.wasm
cargo test
```

`host-check/` loads the built plug-in with VectorCraft's own host, the same manifest checks,
sandbox and budgets the app uses, and runs it as Export As does. It needs a VectorCraft checkout at
`../../vectorcraft`:

```sh
cd host-check
cargo run --release -- ../dist/scanncut-fcm.wasm DOC.vectorcraft OUT.fcm '{"mat": "12 x 24 in"}'
```

`../tools/fcmdump.py FILE.fcm OUTDIR` dumps any FCM file and draws it as SVG.

## Behaviour

- **Placement:** the chosen artboard's top-left corner is the top-left corner of the mat's cutting
  area, at 1:1. The file records the mat's cutting area, as CanvasWorkspace does (29667 × 29880 or
  29667 × 60360, in 0.01 mm).
- **Pieces:** each top-level object is a piece, or each path and compound path with *Every path
  separately*. Sublayers split into pieces like layers.
- **What's cut:** paths, compound paths, groups, Intertwine groups and symbol instances (placed by
  their transform), whatever their paint.
- **What's skipped, each with a warning:** type, images, blends, envelopes, meshes, repeats,
  placed documents, dimensions, and clipping paths (the clipped art is cut whole). Hidden objects,
  guides and template layers are skipped silently. Non-printing layers are skipped with a warning.
- **Live effects:** shape-changing ones (`distort.*`, `warp.*`, `path.*`, `threeD.*`, plug-in
  effects, Round Corners, Scribble) are reported, since the plug-in gets the paths without them.
