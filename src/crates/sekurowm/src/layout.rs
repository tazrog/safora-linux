//! Pure tiling math. No Wayland types here so it can be unit tested.

use sekuro_common::config::TileMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn inset(self, d: i32) -> Self {
        Self::new(self.x + d, self.y + d, (self.w - 2 * d).max(1), (self.h - 2 * d).max(1))
    }
}

/// Split `total` pixels into `n` parts separated by `gap`, distributing the remainder
/// so the parts exactly fill `total`.
fn split_even(total: i32, n: usize, gap: i32) -> Vec<(i32, i32)> {
    let n_i = n as i32;
    let usable = (total - gap * (n_i - 1)).max(n_i);
    let base = usable / n_i;
    let extra = usable % n_i;
    let mut pos = 0;
    (0..n_i)
        .map(|i| {
            let len = base + i32::from(i < extra);
            let r = (pos, len);
            pos += len + gap;
            r
        })
        .collect()
}

/// Compute one tile per window inside `area` (which already excludes the outer gap).
pub fn compute(mode: TileMode, n: usize, area: Rect, gap: i32, master_ratio: f64) -> Vec<Rect> {
    match n {
        0 => return vec![],
        1 => return vec![area],
        _ => {}
    }
    match mode {
        TileMode::Dwindle | TileMode::Smart => {
            let mut out = Vec::with_capacity(n);
            let mut rest = area;
            for i in 0..n {
                if i == n - 1 {
                    out.push(rest);
                    break;
                }
                if rest.w >= rest.h {
                    let w = (rest.w - gap) / 2;
                    out.push(Rect::new(rest.x, rest.y, w, rest.h));
                    rest = Rect::new(rest.x + w + gap, rest.y, rest.w - w - gap, rest.h);
                } else {
                    let h = (rest.h - gap) / 2;
                    out.push(Rect::new(rest.x, rest.y, rest.w, h));
                    rest = Rect::new(rest.x, rest.y + h + gap, rest.w, rest.h - h - gap);
                }
            }
            out
        }
        TileMode::Master => {
            let ratio = master_ratio.clamp(0.1, 0.9);
            let mw = (((area.w - gap) as f64) * ratio) as i32;
            let mut out = vec![Rect::new(area.x, area.y, mw, area.h)];
            let sx = area.x + mw + gap;
            let sw = area.w - mw - gap;
            out.extend(split_even(area.h, n - 1, gap).into_iter().map(|(y, h)| Rect::new(sx, area.y + y, sw, h)));
            out
        }
        TileMode::Columns => {
            split_even(area.w, n, gap).into_iter().map(|(x, w)| Rect::new(area.x + x, area.y, w, area.h)).collect()
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Smart split: a binary space partition where each new window splits the *focused* tile along its
// longer side.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Two tiles next to each other (a vertical dividing line).
    SideBySide,
    /// Two tiles one above the other (a horizontal dividing line).
    Stacked,
}

/// Wider than tall → side by side; taller than wide → stacked.
pub fn smart_dir(r: Rect) -> Dir {
    if r.w >= r.h { Dir::SideBySide } else { Dir::Stacked }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(u32),
    Split { dir: Dir, ratio: f32, a: Box<Node>, b: Box<Node> },
}

fn split_rect(dir: Dir, ratio: f32, r: Rect, gap: i32) -> (Rect, Rect) {
    match dir {
        Dir::SideBySide => {
            let w = (((r.w - gap) as f32) * ratio) as i32;
            (Rect::new(r.x, r.y, w, r.h), Rect::new(r.x + w + gap, r.y, r.w - w - gap, r.h))
        }
        Dir::Stacked => {
            let h = (((r.h - gap) as f32) * ratio) as i32;
            (Rect::new(r.x, r.y, r.w, h), Rect::new(r.x, r.y + h + gap, r.w, r.h - h - gap))
        }
    }
}

impl Node {
    pub fn contains(&self, id: u32) -> bool {
        match self {
            Node::Leaf(i) => *i == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// Leaf ids in tree order (left/top first).
    pub fn ids(&self, out: &mut Vec<u32>) {
        match self {
            Node::Leaf(i) => out.push(*i),
            Node::Split { a, b, .. } => {
                a.ids(out);
                b.ids(out);
            }
        }
    }

    pub fn rects(&self, area: Rect, gap: i32, out: &mut Vec<(u32, Rect)>) {
        match self {
            Node::Leaf(i) => out.push((*i, area)),
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb) = split_rect(*dir, *ratio, area, gap);
                a.rects(ra, gap, out);
                b.rects(rb, gap, out);
            }
        }
    }

    /// Replace the leaf `target` with a split holding `target` and `new`. Returns false if `target` is absent.
    pub fn insert(&mut self, target: u32, new: u32, area: Rect, gap: i32) -> bool {
        match self {
            Node::Leaf(i) if *i == target => {
                *self = Node::Split {
                    dir: smart_dir(area),
                    ratio: 0.5,
                    a: Box::new(Node::Leaf(target)),
                    b: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb) = split_rect(*dir, *ratio, area, gap);
                a.insert(target, new, ra, gap) || b.insert(target, new, rb, gap)
            }
        }
    }

    /// Remove a leaf; its sibling takes over the parent's space. `None` if the tree becomes empty.
    pub fn remove(self, id: u32) -> Option<Node> {
        match self {
            Node::Leaf(i) => (i != id).then_some(Node::Leaf(i)),
            Node::Split { dir, ratio, a, b } => match (a.remove(id), b.remove(id)) {
                (Some(a), Some(b)) => Some(Node::Split { dir, ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    pub fn swap_leaves(&mut self, x: u32, y: u32) {
        match self {
            Node::Leaf(i) => {
                if *i == x {
                    *i = y;
                } else if *i == y {
                    *i = x;
                }
            }
            Node::Split { a, b, .. } => {
                a.swap_leaves(x, y);
                b.swap_leaves(x, y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 10, y: 20, w: 1000, h: 600 };

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn single_window_fills_area() {
        for mode in [TileMode::Dwindle, TileMode::Master, TileMode::Columns] {
            assert_eq!(compute(mode, 1, AREA, 8, 0.5), vec![AREA]);
        }
    }

    #[test]
    fn tiles_never_overlap_and_stay_inside() {
        for mode in [TileMode::Dwindle, TileMode::Master, TileMode::Columns] {
            for n in 2..=12 {
                let tiles = compute(mode, n, AREA, 8, 0.55);
                assert_eq!(tiles.len(), n);
                for (i, a) in tiles.iter().enumerate() {
                    assert!(a.w > 0 && a.h > 0, "{mode:?} n={n} empty tile {a:?}");
                    assert!(a.x >= AREA.x && a.y >= AREA.y);
                    assert!(a.x + a.w <= AREA.x + AREA.w && a.y + a.h <= AREA.y + AREA.h);
                    for b in &tiles[i + 1..] {
                        assert!(!overlaps(a, b), "{mode:?} n={n}: {a:?} overlaps {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn columns_fill_width_exactly() {
        let t = compute(TileMode::Columns, 3, AREA, 10, 0.5);
        assert_eq!(t.last().map(|r| r.x + r.w), Some(AREA.x + AREA.w));
        assert_eq!(t[1].x - (t[0].x + t[0].w), 10);
    }

    #[test]
    fn master_is_larger_than_stack() {
        let t = compute(TileMode::Master, 4, AREA, 8, 0.6);
        assert!(t[0].w > t[1].w);
        assert_eq!(t[1].w, t[3].w);
    }

    // ---- smart split

    const WIDE: Rect = Rect { x: 0, y: 0, w: 1600, h: 800 };
    const TALL: Rect = Rect { x: 0, y: 0, w: 600, h: 1400 };

    #[test]
    fn smart_direction_follows_aspect() {
        assert_eq!(smart_dir(WIDE), Dir::SideBySide);
        assert_eq!(smart_dir(TALL), Dir::Stacked);
        assert_eq!(smart_dir(Rect::new(0, 0, 500, 500)), Dir::SideBySide);
    }

    #[test]
    fn new_window_splits_the_focused_tile() {
        let mut t = Node::Leaf(1);
        assert!(t.insert(1, 2, WIDE, 10)); // wide → 1 | 2
        let mut r = vec![];
        t.rects(WIDE, 10, &mut r);
        assert_eq!(r[0].1.y, r[1].1.y);
        assert!(r[1].1.x > r[0].1.x);
        // Window 3 opens while 2 is focused: only 2's tile (800x800-ish, square → side by side) is split.
        assert!(t.insert(2, 3, r[1].1, 10));
        let mut r2 = vec![];
        t.rects(WIDE, 10, &mut r2);
        assert_eq!(r2.len(), 3);
        assert_eq!(r2[0].1, r[0].1, "window 1 is untouched");
    }

    #[test]
    fn tall_tile_stacks() {
        let mut t = Node::Leaf(1);
        t.insert(1, 2, TALL, 10);
        let mut r = vec![];
        t.rects(TALL, 10, &mut r);
        assert_eq!(r[0].1.x, r[1].1.x);
        assert!(r[1].1.y > r[0].1.y);
    }

    #[test]
    fn removing_collapses_to_sibling() {
        let mut t = Node::Leaf(1);
        t.insert(1, 2, WIDE, 10);
        t.insert(2, 3, WIDE, 10);
        let t = t.remove(2).unwrap();
        let mut ids = vec![];
        t.ids(&mut ids);
        assert_eq!(ids, vec![1, 3]);
        assert!(Node::Leaf(5).remove(5).is_none());
    }

    #[test]
    fn smart_tiles_do_not_overlap() {
        let mut t = Node::Leaf(1);
        let mut focus = 1;
        for id in 2..=9 {
            let mut r = vec![];
            t.rects(WIDE, 8, &mut r);
            let area = r.iter().find(|(i, _)| *i == focus).unwrap().1;
            assert!(t.insert(focus, id, area, 8));
            focus = id;
        }
        let mut r = vec![];
        t.rects(WIDE, 8, &mut r);
        for (i, (_, a)) in r.iter().enumerate() {
            assert!(a.w > 0 && a.h > 0);
            for (_, b) in &r[i + 1..] {
                assert!(!overlaps(a, b));
            }
        }
    }
}
