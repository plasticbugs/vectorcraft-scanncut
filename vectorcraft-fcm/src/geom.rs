//! The geometry the exporter works in: subpaths of lines and cubic Béziers, the affine maps
//! between document, symbol and mat coordinates, and exact bounds.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Pt {
        Pt { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// A segment from the previous segment's end (or the subpath's start).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Line(Pt),
    /// Two control points, then the end.
    Cubic(Pt, Pt, Pt),
}

impl Seg {
    pub fn end(self) -> Pt {
        match self {
            Seg::Line(p) | Seg::Cubic(_, _, p) => p,
        }
    }

    fn map(self, xf: &Affine) -> Seg {
        match self {
            Seg::Line(p) => Seg::Line(xf.apply(p)),
            Seg::Cubic(c1, c2, p) => Seg::Cubic(xf.apply(c1), xf.apply(c2), xf.apply(p)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubPath {
    pub start: Pt,
    pub segs: Vec<Seg>,
    /// Closed subpaths end on their start point.
    pub closed: bool,
}

impl SubPath {
    pub fn map(&self, xf: &Affine) -> SubPath {
        SubPath { start: xf.apply(self.start), segs: self.segs.iter().map(|s| s.map(xf)).collect(), closed: self.closed }
    }
}

/// What the machine moves and selects as one unit: a shape with its holes, a group, a word.
pub type Piece = Vec<SubPath>;

/// `x' = a·x + c·y + e`, `y' = b·x + d·y + f`: `[a, b, c, d, e, f]`, the order VectorCraft
/// stores affines in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn apply(&self, p: Pt) -> Pt {
        let [a, b, c, d, e, f] = self.0;
        Pt::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    /// This map followed by `outer`.
    pub fn then(&self, outer: &Affine) -> Affine {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = outer.0;
        Affine([
            a2 * a1 + c2 * b1,
            b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,
            b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,
            b2 * e1 + d2 * f1 + f2,
        ])
    }
}

/// `[x0, y0, x1, y1]`: the exact bounds of `paths`, through their anchors and the extrema of their
/// curves (not their control points). `None` without any point.
pub fn bounds<'a>(paths: impl IntoIterator<Item = &'a SubPath>) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    let mut add = |p: Pt| {
        b = Some(match b {
            None => [p.x, p.y, p.x, p.y],
            Some([x0, y0, x1, y1]) => [x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)],
        });
    };
    for sp in paths {
        let mut cur = sp.start;
        add(cur);
        for seg in &sp.segs {
            if let Seg::Cubic(c1, c2, end) = *seg {
                cubic_extrema(cur, c1, c2, end, &mut add);
            }
            cur = seg.end();
            add(cur);
        }
    }
    b
}

/// The point of the cubic `p0 c1 c2 p3` at `t`.
pub fn cubic_at(p0: Pt, c1: Pt, c2: Pt, p3: Pt, t: f64) -> Pt {
    let m = 1.0 - t;
    let (w0, w1, w2, w3) = (m * m * m, 3.0 * m * m * t, 3.0 * m * t * t, t * t * t);
    Pt::new(w0 * p0.x + w1 * c1.x + w2 * c2.x + w3 * p3.x, w0 * p0.y + w1 * c1.y + w2 * c2.y + w3 * p3.y)
}

/// Each point strictly inside the cubic where x or y turns.
fn cubic_extrema(p0: Pt, c1: Pt, c2: Pt, p3: Pt, add: &mut impl FnMut(Pt)) {
    for axis in [|p: Pt| p.x, |p: Pt| p.y] {
        let (q0, q1, q2, q3) = (axis(p0), axis(c1), axis(c2), axis(p3));
        // The derivative's roots: a·t² + b·t + c = 0.
        let a = -q0 + 3.0 * q1 - 3.0 * q2 + q3;
        let b = 2.0 * (q0 - 2.0 * q1 + q2);
        let c = q1 - q0;
        let mut roots = [None, None];
        if a.abs() < 1e-12 {
            if b.abs() > 1e-12 {
                roots[0] = Some(-c / b);
            }
        } else {
            let disc = b * b - 4.0 * a * c;
            if disc >= 0.0 {
                let r = disc.sqrt();
                roots = [Some((-b + r) / (2.0 * a)), Some((-b - r) / (2.0 * a))];
            }
        }
        for t in roots.into_iter().flatten() {
            if t > 0.0 && t < 1.0 {
                add(cubic_at(p0, c1, c2, p3, t));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_follow_the_curve_not_its_handles() {
        // A bump whose handles reach y = -40 but whose curve peaks at y = -30.
        let sp = SubPath { start: Pt::new(0.0, 0.0), segs: vec![Seg::Cubic(Pt::new(0.0, -40.0), Pt::new(100.0, -40.0), Pt::new(100.0, 0.0))], closed: false };
        let [x0, y0, x1, y1] = bounds([&sp]).unwrap();
        assert_eq!((x0, x1, y1), (0.0, 100.0, 0.0));
        assert!((y0 + 30.0).abs() < 1e-9, "{y0}");
    }

    #[test]
    fn then_applies_the_inner_map_first() {
        let scale = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        let shift = Affine([1.0, 0.0, 0.0, 1.0, 10.0, 0.0]);
        assert_eq!(scale.then(&shift).apply(Pt::new(1.0, 1.0)), Pt::new(12.0, 2.0));
        assert_eq!(shift.then(&scale).apply(Pt::new(1.0, 1.0)), Pt::new(22.0, 2.0));
    }
}
