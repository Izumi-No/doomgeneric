//! Movement/collision utilities, port of the geometry helpers of `p_maputl.c`.
//!
//! The blockmap iterators take a separate per-line `validcount` table (owned by
//! the world) instead of scribbling on the level, and call closures with a line
//! reference so callers can keep the level borrowed immutably.

use crate::fixed::{fixed_div, fixed_mul, Fixed, FRAC_BITS, FRAC_UNIT};
use crate::level::{Level, Line, SlopeType};
use alloc::vec::Vec;

// `m_bbox.h` (note: differs from `level.rs`'s `BBOX_*` ordering).
pub const BOXTOP: usize = 0;
pub const BOXBOTTOM: usize = 1;
pub const BOXLEFT: usize = 2;
pub const BOXRIGHT: usize = 3;

/// `MAPBLOCKSHIFT`.
pub const MAPBLOCKSHIFT: u32 = 23;

/// `divline_t`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Divline {
    pub x: Fixed,
    pub y: Fixed,
    pub dx: Fixed,
    pub dy: Fixed,
}

/// The window through a two-sided line (`P_LineOpening`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LineOpening {
    pub opentop: Fixed,
    pub openbottom: Fixed,
    pub openrange: Fixed,
    pub lowfloor: Fixed,
}

/// `P_AproxDistance`: an estimate of the distance (not exact).
pub fn aprox_distance(dx: Fixed, dy: Fixed) -> Fixed {
    let dx = dx.abs();
    let dy = dy.abs();
    if dx < dy {
        dx + dy - (dx >> 1)
    } else {
        dx + dy - (dy >> 1)
    }
}

/// `P_PointOnLineSide`. Returns 0 for the front side, 1 for the back side.
pub fn point_on_line_side(level: &Level, x: Fixed, y: Fixed, line: &Line) -> i32 {
    let v1 = level.vertexes[line.v1 as usize];
    if line.dx == 0 {
        if x <= v1.x {
            return (line.dy > 0) as i32;
        }
        return (line.dy < 0) as i32;
    }
    if line.dy == 0 {
        if y <= v1.y {
            return (line.dx < 0) as i32;
        }
        return (line.dx > 0) as i32;
    }

    let dx = x - v1.x;
    let dy = y - v1.y;

    let left = fixed_mul(line.dy >> FRAC_BITS, dx);
    let right = fixed_mul(dy, line.dx >> FRAC_BITS);

    if right < left {
        0 // front side
    } else {
        1 // back side
    }
}

/// `P_BoxOnLineSide`. Considers the line infinite. Returns 0/1, or -1 if the
/// box crosses the line. `tmbox` uses the `BOX*` ordering above.
pub fn box_on_line_side(level: &Level, tmbox: &[Fixed; 4], ld: &Line) -> i32 {
    let v1 = level.vertexes[ld.v1 as usize];

    let (p1, p2) = match ld.slopetype {
        SlopeType::Horizontal => {
            let mut p1 = (tmbox[BOXTOP] > v1.y) as i32;
            let mut p2 = (tmbox[BOXBOTTOM] > v1.y) as i32;
            if ld.dx < 0 {
                p1 ^= 1;
                p2 ^= 1;
            }
            (p1, p2)
        }
        SlopeType::Vertical => {
            let mut p1 = (tmbox[BOXRIGHT] < v1.x) as i32;
            let mut p2 = (tmbox[BOXLEFT] < v1.x) as i32;
            if ld.dy < 0 {
                p1 ^= 1;
                p2 ^= 1;
            }
            (p1, p2)
        }
        SlopeType::Positive => (
            point_on_line_side(level, tmbox[BOXLEFT], tmbox[BOXTOP], ld),
            point_on_line_side(level, tmbox[BOXRIGHT], tmbox[BOXBOTTOM], ld),
        ),
        SlopeType::Negative => (
            point_on_line_side(level, tmbox[BOXRIGHT], tmbox[BOXTOP], ld),
            point_on_line_side(level, tmbox[BOXLEFT], tmbox[BOXBOTTOM], ld),
        ),
    };

    if p1 == p2 {
        p1
    } else {
        -1
    }
}

/// `P_PointOnDivlineSide`. Returns 0 or 1.
pub fn point_on_divline_side(x: Fixed, y: Fixed, line: &Divline) -> i32 {
    if line.dx == 0 {
        if x <= line.x {
            return (line.dy > 0) as i32;
        }
        return (line.dy < 0) as i32;
    }
    if line.dy == 0 {
        if y <= line.y {
            return (line.dx < 0) as i32;
        }
        return (line.dx > 0) as i32;
    }

    let dx = x - line.x;
    let dy = y - line.y;

    // Try to quickly decide by looking at sign bits.
    if (line.dy ^ line.dx ^ dx ^ dy) & i32::MIN != 0 {
        if (line.dy ^ dx) & i32::MIN != 0 {
            return 1; // left is negative
        }
        return 0;
    }

    let left = fixed_mul(line.dy >> 8, dx >> 8);
    let right = fixed_mul(dy >> 8, line.dx >> 8);

    if right < left {
        0 // front side
    } else {
        1 // back side
    }
}

/// `P_MakeDivline`.
pub fn make_divline(level: &Level, li: &Line) -> Divline {
    let v1 = level.vertexes[li.v1 as usize];
    Divline {
        x: v1.x,
        y: v1.y,
        dx: li.dx,
        dy: li.dy,
    }
}

/// `P_InterceptVector`: fractional intercept point along the first divline.
pub fn intercept_vector(v2: &Divline, v1: &Divline) -> Fixed {
    let den = fixed_mul(v1.dy >> 8, v2.dx) - fixed_mul(v1.dx >> 8, v2.dy);
    if den == 0 {
        return 0;
    }

    let num = fixed_mul((v1.x - v2.x) >> 8, v1.dy) + fixed_mul((v2.y - v1.y) >> 8, v1.dx);
    fixed_div(num, den)
}

/// `P_LineOpening`: the window through a two sided line. A single sided line
/// yields `openrange == 0`.
pub fn line_opening(level: &Level, linedef: &Line) -> LineOpening {
    let (Some(front), Some(back)) = (linedef.frontsector, linedef.backsector) else {
        return LineOpening {
            opentop: 0,
            openbottom: 0,
            openrange: 0,
            lowfloor: 0,
        };
    };
    let front = &level.sectors[front];
    let back = &level.sectors[back];

    let opentop = if front.ceilingheight < back.ceilingheight {
        front.ceilingheight
    } else {
        back.ceilingheight
    };

    let (openbottom, lowfloor) = if front.floorheight > back.floorheight {
        (front.floorheight, back.floorheight)
    } else {
        (back.floorheight, front.floorheight)
    };

    LineOpening {
        opentop,
        openbottom,
        openrange: opentop - openbottom,
        lowfloor,
    }
}

/// Blockmap dimensions `(orgx, orgy, width, height)` from a loaded level.
pub fn blockmap_dims(level: &Level) -> Option<(Fixed, Fixed, usize, usize)> {
    let d = &level.blockmap.data;
    if d.len() < 4 {
        return None;
    }
    let w = d[2] as i32;
    let h = d[3] as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    Some((
        (d[0] as i32) << FRAC_BITS,
        (d[1] as i32) << FRAC_BITS,
        w as usize,
        h as usize,
    ))
}

/// `P_BlockLinesIterator`: call `func` for every not-yet-visited line in block
/// `(x, y)`. `validcount` marks already-checked lines in `line_vc` (sized to the
/// line count). Returns `false` as soon as `func` does.
pub fn block_lines<F>(
    level: &Level,
    line_vc: &mut [i32],
    validcount: i32,
    x: i32,
    y: i32,
    mut func: F,
) -> bool
where
    F: FnMut(usize, &Line) -> bool,
{
    let Some((_, _, w, h)) = blockmap_dims(level) else {
        return true;
    };
    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
        return true;
    }

    let data = &level.blockmap.data;
    // The first four shorts are the header; cell offsets follow.
    let offset = data[4 + y as usize * w + x as usize] as usize;
    if offset >= data.len() {
        return true;
    }

    let mut i = offset;
    while i < data.len() {
        let raw = data[i];
        if raw == -1 {
            break;
        }
        let ld = raw as usize;
        if line_vc[ld] != validcount {
            line_vc[ld] = validcount;
            if !func(ld, &level.lines[ld]) {
                return false;
            }
        }
        i += 1;
    }
    true
}

// ---------------------------------------------------------------------------
// Path traversal (`P_PathTraverse` + intercepts, `p_maputl.c`)
// ---------------------------------------------------------------------------

/// One intercept along a trace (`intercept_t`).
#[derive(Clone, Copy, Debug)]
pub struct Intercept {
    pub frac: Fixed,
    pub kind: InterceptKind,
}

/// What the intercept hit (`intercept_t::d` + `isaline`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterceptKind {
    Line(usize),
    Thing(crate::p_mobj::MobjId),
}

pub const PT_ADDLINES: u32 = 1;
pub const PT_ADDTHINGS: u32 = 2;
pub const PT_EARLYOUT: u32 = 4;

pub const MAXINTERCEPTS: usize = 128 + 61;

/// `MAPBTOFRAC`.
pub const MAPBTOFRAC: u32 = FRAC_BITS - (MAPBLOCKSHIFT - FRAC_BITS);
const MAPBLOCKSIZE: Fixed = 128 << 16;

/// `P_PathTraverse`: walk the map blocks along a trace, collecting the
/// intercepts of lines/things then visiting them in order. `visit` is the
/// traverser (`PTR_*`).
#[allow(clippy::too_many_arguments)]
pub fn path_traverse<V>(
    level: &Level,
    mobjs: &crate::p_mobj::Mobjs,
    line_vc: &mut [i32],
    validcount: i32,
    x1: Fixed,
    y1: Fixed,
    x2: Fixed,
    y2: Fixed,
    flags: u32,
    mut visit: V,
) -> bool
where
    V: FnMut(&Intercept) -> bool,
{
    let mut intercepts: Vec<Intercept> = Vec::with_capacity(64);

    let (orgx, orgy, _, _) = blockmap_dims(level).unwrap_or((0, 0, 0, 0));

    let mut x1 = x1;
    let mut y1 = y1;
    if ((x1 - orgx) & (MAPBLOCKSIZE - 1)) == 0 {
        x1 += FRAC_UNIT; // don't side exactly on a line
    }
    if ((y1 - orgy) & (MAPBLOCKSIZE - 1)) == 0 {
        y1 += FRAC_UNIT;
    }

    let trace = Divline {
        x: x1,
        y: y1,
        dx: x2 - x1,
        dy: y2 - y1,
    };

    let x1b = x1 - orgx;
    let y1b = y1 - orgy;
    let xt1 = x1b >> MAPBLOCKSHIFT;
    let yt1 = y1b >> MAPBLOCKSHIFT;

    let x2b = x2 - orgx;
    let y2b = y2 - orgy;
    let xt2 = x2b >> MAPBLOCKSHIFT;
    let yt2 = y2b >> MAPBLOCKSHIFT;

    let (mapxstep, ystep, partial_x) = if xt2 > xt1 {
        (
            1,
            fixed_div(y2b - y1b, (x2b - x1b).abs()),
            FRAC_UNIT - ((x1b >> MAPBTOFRAC) & (FRAC_UNIT - 1)),
        )
    } else if xt2 < xt1 {
        (
            -1,
            fixed_div(y2b - y1b, (x2b - x1b).abs()),
            (x1b >> MAPBTOFRAC) & (FRAC_UNIT - 1),
        )
    } else {
        (0, 256 * FRAC_UNIT, FRAC_UNIT)
    };
    let mut yintercept = (y1b >> MAPBTOFRAC).wrapping_add(fixed_mul(partial_x, ystep));

    let (mapystep, xstep, partial_y) = if yt2 > yt1 {
        (
            1,
            fixed_div(x2b - x1b, (y2b - y1b).abs()),
            FRAC_UNIT - ((y1b >> MAPBTOFRAC) & (FRAC_UNIT - 1)),
        )
    } else if yt2 < yt1 {
        (
            -1,
            fixed_div(x2b - x1b, (y2b - y1b).abs()),
            (y1b >> MAPBTOFRAC) & (FRAC_UNIT - 1),
        )
    } else {
        (0, 256 * FRAC_UNIT, FRAC_UNIT)
    };
    let mut xintercept = (x1b >> MAPBTOFRAC).wrapping_add(fixed_mul(partial_y, xstep));

    let earlyout = flags & PT_EARLYOUT != 0;

    let mut mapx = xt1;
    let mut mapy = yt1;

    let mut buf: Vec<crate::p_mobj::MobjId> = Vec::new();

    for _count in 0..64 {
        if flags & PT_ADDLINES != 0 {
            let mut ok = true;
            if !block_lines(level, line_vc, validcount, mapx, mapy, |li, _line| {
                ok = add_line_intercept(level, &trace, earlyout, li, &mut intercepts);
                ok
            }) {
                return false;
            }
            if !ok {
                return false;
            }
        }

        if flags & PT_ADDTHINGS != 0 {
            let (_, _, bw, bh) = blockmap_dims(level).unwrap_or((0, 0, 0, 0));
            if mapx >= 0 && mapy >= 0 && (mapx as usize) < bw && (mapy as usize) < bh {
                let block = mapy as usize * bw + mapx as usize;
                buf.clear();
                mobjs.collect_block(block, &mut buf);
                for id in buf.iter().copied() {
                    if let Some(m) = mobjs.get(id) {
                        add_thing_intercept(&trace, m.x, m.y, m.radius, id, &mut intercepts);
                    }
                }
            }
        }

        if mapx == xt2 && mapy == yt2 {
            break;
        }
        if (yintercept >> FRAC_BITS) == mapy {
            yintercept += ystep;
            mapx += mapxstep;
        } else if (xintercept >> FRAC_BITS) == mapx {
            xintercept += xstep;
            mapy += mapystep;
        }
    }

    traverse_intercepts(&mut intercepts, FRAC_UNIT, &mut visit)
}

/// `PIT_AddThingIntercepts`: push an intercept if the thing's cross-section
/// intersects the trace.
#[allow(clippy::too_many_arguments)]
pub fn add_thing_intercept(
    trace: &Divline,
    x: Fixed,
    y: Fixed,
    radius: Fixed,
    id: crate::p_mobj::MobjId,
    intercepts: &mut Vec<Intercept>,
) {
    let tracepositive = (trace.dx ^ trace.dy) > 0;
    let (x1, y1, x2, y2) = if tracepositive {
        (x - radius, y + radius, x + radius, y - radius)
    } else {
        (x - radius, y - radius, x + radius, y + radius)
    };
    let s1 = point_on_divline_side(x1, y1, trace);
    let s2 = point_on_divline_side(x2, y2, trace);
    if s1 == s2 {
        return;
    }
    let dl = Divline {
        x: x1,
        y: y1,
        dx: x2 - x1,
        dy: y2 - y1,
    };
    let frac = intercept_vector(trace, &dl);
    if frac < 0 {
        return;
    }
    intercepts.push(Intercept {
        frac,
        kind: InterceptKind::Thing(id),
    });
}

/// `P_TraverseIntercepts`.
pub fn traverse_intercepts<V>(
    intercepts: &mut [Intercept],
    maxfrac: Fixed,
    mut visit: V,
) -> bool
where
    V: FnMut(&Intercept) -> bool,
{
    let mut count = intercepts.len();
    while count > 0 {
        count -= 1;
        let mut dist = i32::MAX;
        let mut best: Option<usize> = None;
        for (i, scan) in intercepts.iter().enumerate() {
            if scan.frac < dist {
                dist = scan.frac;
                best = Some(i);
            }
        }
        let Some(bi) = best else { return true };
        if dist > maxfrac {
            return true;
        }
        if !visit(&intercepts[bi]) {
            return false;
        }
        intercepts[bi].frac = i32::MAX;
    }
    true
}

/// `PIT_AddLineIntercepts`: if the block's line crosses the trace, push an
/// intercept. `earlyout` stops the walk on any solid (one-sided) line.
pub fn add_line_intercept(
    level: &Level,
    trace: &Divline,
    earlyout: bool,
    li: usize,
    intercepts: &mut Vec<Intercept>,
) -> bool {
    let line = &level.lines[li];
    let v1 = level.vertexes[line.v1 as usize];
    let v2 = level.vertexes[line.v2 as usize];

    let (s1, s2) = if trace.dx > FRAC_UNIT * 16
        || trace.dy > FRAC_UNIT * 16
        || trace.dx < -FRAC_UNIT * 16
        || trace.dy < -FRAC_UNIT * 16
    {
        (
            point_on_divline_side(v1.x, v1.y, trace),
            point_on_divline_side(v2.x, v2.y, trace),
        )
    } else {
        (
            point_on_line_side(level, trace.x, trace.y, line),
            point_on_line_side(level, trace.x + trace.dx, trace.y + trace.dy, line),
        )
    };
    if s1 == s2 {
        return true;
    }

    let dl = make_divline(level, line);
    let frac = intercept_vector(trace, &dl);
    if frac < 0 {
        return true;
    }

    if earlyout && frac < FRAC_UNIT && line.backsector.is_none() {
        return false;
    }

    intercepts.push(Intercept {
        frac,
        kind: InterceptKind::Line(li),
    });
    true
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Level, Line, Sector, SlopeType, Vertex};
    use alloc::vec;
    use alloc::vec::Vec;

    fn vertex(x: i32, y: i32) -> Vertex {
        Vertex { x, y }
    }

    fn level_with(vertexes: Vec<Vertex>, line: Line) -> Level {
        let mut level = Level {
            vertexes,
            sectors: vec![
                Sector {
                    floorheight: 0,
                    ceilingheight: 128 << 16,
                    ..Sector::default()
                },
                Sector {
                    floorheight: 16 << 16,
                    ceilingheight: 64 << 16,
                    ..Sector::default()
                },
            ],
            lines: vec![line],
            ..Level::default()
        };
        level.lines[0].slopetype = SlopeType::from_vectors(level.lines[0].dx, level.lines[0].dy);
        level
    }

    #[test]
    fn point_side_horizontal_and_vertical() {
        let line = Line {
            v1: 0,
            v2: 1,
            dx: 64 << 16,
            dy: 0,
            ..Line::default()
        };
        let level = level_with(vec![vertex(0, 0), vertex(64 << 16, 0)], line);
        // Line runs +x. Points below (y<0) are on the front (0), above on back.
        assert_eq!(point_on_line_side(&level, 32 << 16, -(1 << 16), &level.lines[0]), 0);
        assert_eq!(point_on_line_side(&level, 32 << 16, 1 << 16, &level.lines[0]), 1);

        let vline = Line {
            v1: 0,
            v2: 1,
            dx: 0,
            dy: 64 << 16,
            slopetype: SlopeType::Vertical,
            ..Line::default()
        };
        let level = level_with(vec![vertex(0, 0), vertex(0, 64 << 16)], vline);
        // Line runs +y; points to its left (x<0) are front (1), right are back.
        assert_eq!(point_on_line_side(&level, -(1 << 16), 32 << 16, &level.lines[0]), 1);
        assert_eq!(point_on_line_side(&level, 1 << 16, 32 << 16, &level.lines[0]), 0);
    }

    #[test]
    fn box_on_line_side_detects_crossing() {
        let line = Line {
            v1: 0,
            v2: 1,
            dx: 64 << 16,
            dy: 0,
            slopetype: SlopeType::Horizontal,
            ..Line::default()
        };
        let level = level_with(vec![vertex(0, 0), vertex(64 << 16, 0)], line);
        // A box straddling y=0 crosses the line.
        let crossing = [1 << 16, -(1 << 16), -(1 << 16), 1 << 16]; // top,bottom,left,right
        assert_eq!(box_on_line_side(&level, &crossing, &level.lines[0]), -1);
        let above = [4 << 16, 2 << 16, -(1 << 16), 1 << 16];
        assert_eq!(box_on_line_side(&level, &above, &level.lines[0]), 1);
    }

    #[test]
    fn intercept_vector_parallel_is_zero() {
        let a = Divline {
            x: 0,
            y: 0,
            dx: 1 << 16,
            dy: 1 << 16,
        };
        let b = Divline {
            x: 0,
            y: 5 << 16,
            dx: 1 << 16,
            dy: 1 << 16,
        };
        assert_eq!(intercept_vector(&a, &b), 0);
    }

    #[test]
    fn line_opening_single_and_double_sided() {
        // Single sided.
        let mut line = Line {
            frontsector: Some(0),
            ..Line::default()
        };
        let level = level_with(vec![vertex(0, 0), vertex(8 << 16, 0)], line);
        assert_eq!(line_opening(&level, &level.lines[0]).openrange, 0);

        // Double sided: min ceiling - max floor.
        line.backsector = Some(1);
        let level = level_with(vec![vertex(0, 0), vertex(8 << 16, 0)], line);
        let open = line_opening(&level, &level.lines[0]);
        assert_eq!(open.opentop, 64 << 16);
        assert_eq!(open.openbottom, 16 << 16);
        assert_eq!(open.openrange, 48 << 16);
        assert_eq!(open.lowfloor, 0);
    }

    #[test]
    fn block_lines_iterates_each_line_once() {
        // 1x1 blockmap containing lines 0 and 1; a second block repeats line 0.
        let mut level = Level {
            lines: vec![Line::default(), Line::default()],
            ..Level::default()
        };
        // header: org(0,0), 2x1 cells; cell0 list offset 8, cell1 offset 11.
        // data: [0,0,2,1, 8,11, -1, -1, 0,1,-1, 0,-1]
        level.blockmap.data = vec![0, 0, 2, 1, 8, 11, -1, -1, 0, 1, -1, 0, -1];
        let mut vc = vec![0; 2];

        let mut seen = alloc::vec::Vec::new();
        let ok = block_lines(&level, &mut vc, 1, 0, 0, |_i, l| {
            seen.push(l.dx);
            true
        });
        assert!(ok);
        assert_eq!(seen.len(), 2);

        // Visiting block 1 within the same pass must skip line 0 (already seen).
        let mut seen2 = alloc::vec::Vec::new();
        block_lines(&level, &mut vc, 1, 1, 0, |_i, l| {
            seen2.push(l.dx);
            true
        });
        assert!(seen2.is_empty());
    }
}
