//! Line-of-sight checking, port of `p_sight.c`.
//!
//! Walks the BSP tree testing visibility of two mobjs (`P_CheckSight`), with
//! the REJECT matrix trivial rejection preserved.

use crate::doomdef::NF_SUBSECTOR;
use crate::fixed::{fixed_div, Fixed};
use crate::level::Level;
use crate::p_maputl::Divline;
use crate::p_mobj::{MobjId, Mobjs};

/// One sight pass state (strace, sightz, slopes).
struct Sight {
    strace: Divline,
    t2x: Fixed,
    t2y: Fixed,
    sightzstart: Fixed,
    topslope: Fixed,
    bottomslope: Fixed,
    validcount: i32,
}

impl Sight {
    /// `P_DivlineSide` (the sight version, which can return 2 "on the line").
    fn divline_side(&self, x: Fixed, y: Fixed, node: &DivlineLike) -> i32 {
        if node.dx == 0 {
            if x == node.x {
                return 2;
            }
            if x <= node.x {
                return (node.dy > 0) as i32;
            }
            return (node.dy < 0) as i32;
        }
        if node.dy == 0 {
            if x == node.y {
                return 2;
            }
            if y <= node.y {
                return (node.dx < 0) as i32;
            }
            return (node.dx > 0) as i32;
        }

        let dx = x - node.x;
        let dy = y - node.y;
        let left = (node.dy >> crate::fixed::FRAC_BITS) * (dx >> crate::fixed::FRAC_BITS);
        let right = (dy >> crate::fixed::FRAC_BITS) * (node.dx >> crate::fixed::FRAC_BITS);
        if right < left {
            0
        } else if left == right {
            2
        } else {
            1
        }
    }
}

/// Node-as-divline fields needed by the sight code (`node_t` shares layout).
struct DivlineLike {
    x: Fixed,
    y: Fixed,
    dx: Fixed,
    dy: Fixed,
}

/// `P_CrossSubsector`: is the trace unobstructed through this subsector?
fn cross_subsector(level: &Level, line_vc: &mut [i32], s: &mut Sight, num: usize) -> bool {
    let sub = &level.subsectors[num];
    let first = sub.firstline as usize;
    let count = sub.numlines as i32;

    for i in 0..count {
        let seg = &level.segs[first + i as usize];
        let line_idx = seg.linedef as usize;
        let line = &level.lines[line_idx];
        if line_vc[line_idx] == s.validcount {
            continue;
        }
        line_vc[line_idx] = s.validcount;

        let v1 = &level.vertexes[line.v1 as usize];
        let v2 = &level.vertexes[line.v2 as usize];
        let s1 = s.divline_side(v1.x, v1.y, &DivlineLike { x: s.strace.x, y: s.strace.y, dx: s.strace.dx, dy: s.strace.dy });
        let s2 = s.divline_side(v2.x, v2.y, &DivlineLike { x: s.strace.x, y: s.strace.y, dx: s.strace.dx, dy: s.strace.dy });
        if s1 == s2 {
            continue;
        }

        let divl = DivlineLike {
            x: v1.x,
            y: v1.y,
            dx: v2.x - v1.x,
            dy: v2.y - v1.y,
        };
        let s1 = s.divline_side(s.strace.x, s.strace.y, &divl);
        let s2 = s.divline_side(s.t2x, s.t2y, &divl);
        if s1 == s2 {
            continue;
        }

        if line.backsector.is_none() || line.flags & crate::doomdef::ML_TWOSIDED == 0 {
            return false;
        }

        let front = &level.sectors[seg.frontsector];
        let back = &level.sectors[seg.backsector.unwrap_or(seg.frontsector)];

        if front.floorheight == back.floorheight && front.ceilingheight == back.ceilingheight {
            continue;
        }

        let opentop = front.ceilingheight.min(back.ceilingheight);
        let openbottom = front.floorheight.max(back.floorheight);
        if openbottom >= opentop {
            return false;
        }

        let dl = Divline {
            x: divl.x,
            y: divl.y,
            dx: divl.dx,
            dy: divl.dy,
        };
        let frac = crate::p_maputl::intercept_vector(&s.strace, &dl);

        if front.floorheight != back.floorheight {
            let slope = fixed_div(openbottom - s.sightzstart, frac);
            if slope > s.bottomslope {
                s.bottomslope = slope;
            }
        }
        if front.ceilingheight != back.ceilingheight {
            let slope = fixed_div(opentop - s.sightzstart, frac);
            if slope < s.topslope {
                s.topslope = slope;
            }
        }
        if s.topslope <= s.bottomslope {
            return false;
        }
    }
    true
}

/// `P_CrossBSPNode`.
fn cross_bsp_node(level: &Level, line_vc: &mut [i32], s: &mut Sight, bspnum: u32) -> bool {
    if bspnum & NF_SUBSECTOR as u32 != 0 {
        if bspnum == u32::MAX {
            return cross_subsector(level, line_vc, s, 0);
        }
        return cross_subsector(level, line_vc, s, (bspnum & !(NF_SUBSECTOR as u32)) as usize);
    }

    let bsp = &level.nodes[bspnum as usize];
    // node_t shares the leading layout with divline_t
    let nodelike = DivlineLike {
        x: bsp.x,
        y: bsp.y,
        dx: bsp.dx,
        dy: bsp.dy,
    };

    let mut side = s.divline_side(s.strace.x, s.strace.y, &nodelike);
    if side == 2 {
        side = 0;
    }

    if !cross_bsp_node(level, line_vc, s, bsp.children[side as usize] as u32) {
        return false;
    }

    if side == s.divline_side(s.t2x, s.t2y, &nodelike) {
        return true;
    }

    cross_bsp_node(level, line_vc, s, bsp.children[(side as usize) ^ 1] as u32)
}

/// `P_CheckSight`: is a straight line between `t1` and `t2` unobstructed?
/// Uses the REJECT matrix for trivial rejection.
pub fn check_sight(
    level: &Level,
    mobjs: &Mobjs,
    line_vc: &mut [i32],
    mut validcount: i32,
    t1: MobjId,
    t2: MobjId,
) -> bool {
    let Some(a) = mobjs.get(t1) else { return false };
    let Some(b) = mobjs.get(t2) else { return false };

    // REJECT-based trivial rejection.
    let s1 = a.sector;
    let s2 = b.sector;
    let numsectors = level.sectors.len();
    let pnum = s1 * numsectors + s2;
    let bytenum = pnum >> 3;
    let bitnum = 1u32 << (pnum & 7);
    if !level.reject.is_empty() && (level.reject[bytenum % level.reject.len()] as u32 & bitnum) != 0
    {
        return false;
    }

    // Unobstructed LOS is possible; trace the BSP.
    let mut s = Sight {
        strace: Divline {
            x: a.x,
            y: a.y,
            dx: b.x - a.x,
            dy: b.y - a.y,
        },
        t2x: b.x,
        t2y: b.y,
        sightzstart: a.z + a.height - (a.height >> 2),
        topslope: (b.z + b.height) - (a.z + a.height - (a.height >> 2)),
        bottomslope: b.z - (a.z + a.height - (a.height >> 2)),
        validcount,
    };
    s.topslope = (b.z + b.height) - s.sightzstart;
    s.bottomslope = b.z - s.sightzstart;

    validcount += 1;
    s.validcount = validcount;

    if level.nodes.is_empty() {
        return true;
    }
    let root = (level.nodes.len() - 1) as u32;
    cross_bsp_node(level, line_vc, &mut s, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::Level;
    use crate::p_mobj::{Mobj, Mobjs};
    use crate::info::MobjType;

    #[test]
    fn empty_level_always_sees() {
        let level = Level::default();
        let mut mobjs = Mobjs::new(1);
        let a = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 0));
        let b = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 64 << 16, 0, 0));
        let mut vc = alloc::vec![0; 8];
        assert!(check_sight(&level, &mobjs, &mut vc, 1, a, b));
    }
}
