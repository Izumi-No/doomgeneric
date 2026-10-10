//! Movement clipping, a partial port of `p_map.c`.
//!
//! Implemented: `PIT_CheckLine`, `P_CheckPosition` and `P_TryMove` (geometry and
//! blocking). The sound/pickup/damage/special-line side effects of the original
//! are deferred to the gameplay layer and are marked below.

use alloc::vec::Vec;

use crate::doomdef::{ML_BLOCKING, ML_BLOCKMONSTERS, ML_TWOSIDED};
use crate::fixed::{fixed_div, fixed_mul, Fixed, FRAC_BITS, FRAC_UNIT};
use crate::level::{Level, Line, BBOX_BOTTOM, BBOX_LEFT, BBOX_RIGHT, BBOX_TOP};
use crate::p_maputl::{
    block_lines, blockmap_dims, box_on_line_side, line_opening, path_traverse, Intercept,
    InterceptKind, PT_ADDLINES, PT_ADDTHINGS, PT_EARLYOUT, BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP,
};
use crate::p_mobj::{
    Mobj, MobjId, Mobjs, MF_DROPPED, MF_DROPOFF, MF_FLOAT, MF_MISSILE, MF_NOCLIP, MF_SHOOTABLE,
    MF_SOLID, MF_SPECIAL, MF_TELEPORT,
};
use crate::p_world::World;
use crate::r_main::Render;

/// `MAXRADIUS`.
pub const MAXRADIUS: Fixed = 32 << 16;
/// `MAPBLOCKSHIFT`.
pub const MAPBLOCKSHIFT: u32 = 23;
/// Maximum step up / drop off (`24*FRACUNIT`).
const MAX_STEP: Fixed = 24 << 16;

/// The transient state threaded through the `PIT_*` callbacks (`P_CheckPosition`
/// in vanilla uses file-scope globals for these).
pub struct MoveCheck {
    pub x: Fixed,
    pub y: Fixed,
    pub radius: Fixed,
    pub flags: u32,
    pub is_player: bool,
    pub id: MobjId,

    pub bbox: [Fixed; 4],

    pub floorz: Fixed,
    pub ceilingz: Fixed,
    pub dropoffz: Fixed,

    pub ceilingline: Option<usize>,
    pub spechit: Vec<usize>,
}

/// `PIT_CheckLine`: adjusts the floor/ceiling window for a crossed line, or
/// rejects the move for a blocking one sided line.
fn check_line(check: &mut MoveCheck, level: &Level, index: usize, line: &Line) -> bool {
    if check.bbox[BOXRIGHT] <= line.bbox[BBOX_LEFT]
        || check.bbox[BOXLEFT] >= line.bbox[BBOX_RIGHT]
        || check.bbox[BOXTOP] <= line.bbox[BBOX_BOTTOM]
        || check.bbox[BOXBOTTOM] >= line.bbox[BBOX_TOP]
    {
        return true;
    }

    if box_on_line_side(level, &check.bbox, line) != -1 {
        return true;
    }

    // One sided line.
    if line.backsector.is_none() {
        return false;
    }

    if check.flags & MF_MISSILE == 0 {
        if line.flags & ML_BLOCKING != 0 {
            return false;
        }
        if !check.is_player && line.flags & ML_BLOCKMONSTERS != 0 {
            return false;
        }
    }

    let open = line_opening(level, line);

    if open.opentop < check.ceilingz {
        check.ceilingz = open.opentop;
        check.ceilingline = Some(index);
    }

    if open.openbottom > check.floorz {
        check.floorz = open.openbottom;
    }

    if open.lowfloor < check.dropoffz {
        check.dropoffz = open.lowfloor;
    }

    if line.special != 0 {
        check.spechit.push(index);
    }

    true
}

/// `PIT_CheckThing`: rejects the move when it overlaps a solid thing.
///
/// Deferred: the vanilla skull-slam, missile-damage and special-pickup branches
/// (they belong to the gameplay layer).
fn check_thing(check: &MoveCheck, id: MobjId, thing: &Mobj) -> bool {
    if thing.flags & (MF_SOLID | MF_SPECIAL | MF_SHOOTABLE) == 0 {
        return true;
    }

    let blockdist = thing.radius + check.radius;
    if (thing.x - check.x).abs() >= blockdist || (thing.y - check.y).abs() >= blockdist {
        return true; // didn't hit it
    }

    if id == check.id {
        return true; // don't clip against self
    }

    // Deferred: MF_SKULLFLY / MF_MISSILE / MF_SPECIAL handling.
    thing.flags & MF_SOLID == 0
}

/// `P_CheckPosition`: is there room for `id` at `(x, y)`? On success returns the
/// resulting floor/ceiling window and any crossed special lines.
#[allow(clippy::too_many_arguments)]
pub fn check_position(
    level: &Level,
    render: &Render,
    mobjs: &Mobjs,
    line_vc: &mut [i32],
    validcount: i32,
    id: MobjId,
    x: Fixed,
    y: Fixed,
) -> Option<MoveCheck> {
    let (radius, flags, is_player) = {
        let m = mobjs.get(id)?;
        (m.radius, m.flags, m.player.is_some())
    };

    let mut check = MoveCheck {
        x,
        y,
        radius,
        flags,
        is_player,
        id,
        bbox: [0; 4],
        floorz: 0,
        ceilingz: 0,
        dropoffz: 0,
        ceilingline: None,
        spechit: Vec::new(),
    };

    check.bbox[BOXTOP] = y + radius;
    check.bbox[BOXBOTTOM] = y - radius;
    check.bbox[BOXRIGHT] = x + radius;
    check.bbox[BOXLEFT] = x - radius;

    let subsector = render.point_in_subsector(level, x, y);
    let sector = &level.sectors[level.subsectors[subsector].sector];
    check.floorz = sector.floorheight;
    check.dropoffz = sector.floorheight;
    check.ceilingz = sector.ceilingheight;

    if flags & MF_NOCLIP != 0 {
        return Some(check);
    }

    let Some((orgx, orgy, w, h)) = blockmap_dims(level) else {
        return Some(check);
    };

    // Check things first (bounding box extended by MAXRADIUS).
    let xl = (check.bbox[BOXLEFT] - orgx - MAXRADIUS) >> MAPBLOCKSHIFT;
    let xh = (check.bbox[BOXRIGHT] - orgx + MAXRADIUS) >> MAPBLOCKSHIFT;
    let yl = (check.bbox[BOXBOTTOM] - orgy - MAXRADIUS) >> MAPBLOCKSHIFT;
    let yh = (check.bbox[BOXTOP] - orgy + MAXRADIUS) >> MAPBLOCKSHIFT;

    let mut buf: Vec<MobjId> = Vec::new();
    for bx in xl..=xh {
        for by in yl..=yh {
            let Some(block) = block_at(orgx, orgy, w, h, bx, by) else {
                continue;
            };
            buf.clear();
            mobjs.collect_block(block, &mut buf);
            for other in buf.iter().copied() {
                let Some(thing) = mobjs.get(other) else {
                    continue;
                };
                if !check_thing(&check, other, thing) {
                    return None;
                }
            }
        }
    }

    // Then lines.
    let xl = (check.bbox[BOXLEFT] - orgx) >> MAPBLOCKSHIFT;
    let xh = (check.bbox[BOXRIGHT] - orgx) >> MAPBLOCKSHIFT;
    let yl = (check.bbox[BOXBOTTOM] - orgy) >> MAPBLOCKSHIFT;
    let yh = (check.bbox[BOXTOP] - orgy) >> MAPBLOCKSHIFT;

    for bx in xl..=xh {
        for by in yl..=yh {
            if !block_lines(level, line_vc, validcount, bx, by, |idx, line| {
                check_line(&mut check, level, idx, line)
            }) {
                return None;
            }
        }
    }

    Some(check)
}

fn block_at(orgx: Fixed, orgy: Fixed, w: usize, h: usize, bx: i32, by: i32) -> Option<usize> {
    let _ = (orgx, orgy);
    if bx < 0 || by < 0 || bx as usize >= w || by as usize >= h {
        return None;
    }
    Some(by as usize * w + bx as usize)
}

/// `P_TryMove`: attempt to move `id` to `(x, y)`, linking it into its new
/// position on success.
///
/// Deferred: crossing special lines (`P_CrossSpecialLine`).
#[allow(clippy::too_many_arguments)]
pub fn try_move(
    level: &Level,
    render: &Render,
    mobjs: &mut Mobjs,
    line_vc: &mut [i32],
    validcount: i32,
    id: MobjId,
    x: Fixed,
    y: Fixed,
) -> bool {
    let (flags, height, z, radius) = {
        let Some(m) = mobjs.get(id) else {
            return false;
        };
        (m.flags, m.height, m.z, m.radius)
    };

    let Some(check) = check_position(level, render, mobjs, line_vc, validcount, id, x, y) else {
        return false;
    };

    if flags & MF_NOCLIP == 0 {
        if check.ceilingz - check.floorz < radius {
            return false; // doesn't fit
        }
        if flags & MF_TELEPORT == 0 && check.ceilingz - z < height {
            return false; // mobj must lower itself to fit
        }
        if flags & MF_TELEPORT == 0 && check.floorz - z > MAX_STEP {
            return false; // too big a step up
        }
        if flags & (MF_DROPOFF | MF_FLOAT) == 0 && check.floorz - check.dropoffz > MAX_STEP {
            return false; // don't stand over a dropoff
        }
    }

    // The move is ok: relink.
    mobjs.unset_thing_position(id);
    if let Some(m) = mobjs.get_mut(id) {
        m.floorz = check.floorz;
        m.ceilingz = check.ceilingz;
        m.x = x;
        m.y = y;
    }
    let subsector = render.point_in_subsector(level, x, y);
    let sector = level.subsectors[subsector].sector;
    mobjs.set_thing_position(id, level, render, sector);

    // Deferred: crossing special lines (P_CrossSpecialLine).
    true
}


/// `P_ThingHeightClip`: adjust `id`'s floor/ceiling and possibly its `z`.
pub fn thing_height_clip(
    level: &Level,
    render: &Render,
    mobjs: &mut Mobjs,
    line_vc: &mut [i32],
    validcount: i32,
    id: MobjId,
) -> bool {
    let (x, y) = {
        let Some(m) = mobjs.get(id) else {
            return true;
        };
        (m.x, m.y)
    };
    let onfloor = mobjs
        .get(id)
        .map(|m| m.z == m.floorz)
        .unwrap_or(false);

    let Some(check) = check_position(level, render, mobjs, line_vc, validcount, id, x, y) else {
        return false;
    };

    if let Some(m) = mobjs.get_mut(id) {
        m.floorz = check.floorz;
        m.ceilingz = check.ceilingz;
        if onfloor {
            m.z = check.floorz;
        } else if m.z + m.height > m.ceilingz {
            m.z = m.ceilingz - m.height;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Slide move (`P_SlideMove` + `P_HitSlideLine`, `p_map.c`)
// ---------------------------------------------------------------------------

/// `P_HitSlideLine`: clip `tmxmove`/`tmymove`-style momentum to slide along
/// the slideline. Returns the new `(momx, momy)`.
pub fn hit_slide_line(x: Fixed, y: Fixed, momx: Fixed, momy: Fixed, line: &Line, level: &Level) -> (Fixed, Fixed) {
    use crate::level::SlopeType;
    use crate::r_main::point_to_angle_from;
    use crate::tables::{finecosine_index, finesine_index, ANGLETOFINESHIFT, ANG180};

    if line.slopetype == SlopeType::Horizontal {
        return (momx, 0);
    }
    if line.slopetype == SlopeType::Vertical {
        return (0, momy);
    }

    let side = crate::p_maputl::point_on_line_side(level, x, y, line);
    let mut lineangle = point_to_angle_from(0, 0, line.dx, line.dy);
    if side == 1 {
        lineangle = lineangle.wrapping_add(ANG180);
    }
    let moveangle = point_to_angle_from(0, 0, momx, momy);
    let mut deltaangle = moveangle.wrapping_sub(lineangle);
    if deltaangle > ANG180 {
        deltaangle = deltaangle.wrapping_add(ANG180);
    }

    let movelen = crate::p_maputl::aprox_distance(momx, momy);
    let newlen = fixed_mul(movelen, finecosine_index(deltaangle >> ANGLETOFINESHIFT));
    (
        fixed_mul(newlen, finecosine_index(lineangle >> ANGLETOFINESHIFT)),
        fixed_mul(newlen, finesine_index(lineangle >> ANGLETOFINESHIFT)),
    )
}

/// `P_SlideMove`: try to slide `id` along walls. `hitcount` bounds the loop.
#[allow(clippy::too_many_arguments)]
pub fn slide_move(
    level: &Level,
    render: &Render,
    mobjs: &mut Mobjs,
    line_vc: &mut [i32],
    validcount: i32,
    id: MobjId,
) {
    let Some(m0) = mobjs.get(id) else { return };
    let radius = m0.radius;
    let mut mx = m0.x;
    let mut my = m0.y;

    for _hitcount in 0..3 {
        let Some(m) = mobjs.get(id) else { return };
        let momx = m.momx;
        let momy = m.momy;

        let (leadx, leady, trailx, traily);
        if momx > 0 {
            leadx = mx + radius;
            trailx = mx - radius;
        } else {
            leadx = mx - radius;
            trailx = mx + radius;
        }
        if momy > 0 {
            leady = my + radius;
            traily = my - radius;
        } else {
            leady = my - radius;
            traily = my + radius;
        }

        let mut best_frac = FRAC_UNIT + 1;
        let mut best_line: Option<usize> = None;
        // Trace along the three leading corners.
        for (ox, oy) in [(leadx, leady), (trailx, leady), (leadx, traily)] {
            let _ = path_traverse(
                level,
                mobjs,
                line_vc,
                validcount,
                ox,
                oy,
                ox + momx,
                oy + momy,
                PT_ADDLINES | PT_EARLYOUT,
                |in_: &Intercept| {
                    if let InterceptKind::Line(li) = in_.kind {
                        if in_.frac < best_frac {
                            best_frac = in_.frac;
                            best_line = Some(li);
                        }
                    }
                    true
                },
            );
        }

        if best_frac == FRAC_UNIT + 1 {
            // Hit the middle: stair-step.
            if !try_move(level, render, mobjs, line_vc, validcount, id, mx, my + momy) {
                try_move(level, render, mobjs, line_vc, validcount, id, mx + momx, my);
            }
            return;
        }

        // Fudge: don't hit exactly.
        best_frac -= 0x800;
        if best_frac > 0 {
            let newx = fixed_mul(momx, best_frac);
            let newy = fixed_mul(momy, best_frac);
            if !try_move(level, render, mobjs, line_vc, validcount, id, mx + newx, my + newy) {
                // Stair-step then return.
                if !try_move(level, render, mobjs, line_vc, validcount, id, mx, my + momy) {
                    try_move(level, render, mobjs, line_vc, validcount, id, mx + momx, my);
                }
                return;
            }
            mx = mobjs.get(id).map(|m| m.x).unwrap_or(0);
            my = mobjs.get(id).map(|m| m.y).unwrap_or(0);
        }

        // Continue along the wall: clip the remaining moves.
        let mut frac = FRAC_UNIT - (best_frac + 0x800);
        if frac > FRAC_UNIT {
            frac = FRAC_UNIT;
        }
        if frac <= 0 {
            return;
        }
        let mut momx2 = fixed_mul(momx, frac);
        let mut momy2 = fixed_mul(momy, frac);

        if let Some(line_idx) = best_line {
            let ld = level.lines[line_idx];
            let (nx, ny) = hit_slide_line(mx, my, momx2, momy2, &ld, level);
            momx2 = nx;
            momy2 = ny;
        }

        if let Some(mo) = mobjs.get_mut(id) {
            mo.momx = momx2;
            mo.momy = momy2;
        }

        if try_move(level, render, mobjs, line_vc, validcount, id, mx + momx2, my + momy2) {
            return;
        }
        // retry loop continues
    }
}

// ---------------------------------------------------------------------------
// Shoot traces (`P_AimLineAttack` / `P_LineAttack`, `p_map.c`)
// ---------------------------------------------------------------------------

/// The result of a shoot/aim trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineAttackHit {
    /// Nothing hit.
    Nothing,
    /// A thing was hit (id + slope at hit time).
    Thing(MobjId, Fixed),
    /// A line blocked the shot (line index).
    Line(usize),
}

/// `P_AimLineAttack`: find the nearest shootable thing along `angle` within
/// `distance`, returning its aim slope and the thing (`linetarget`).
#[allow(clippy::too_many_arguments)]
pub fn aim_line_attack(
    level: &Level,
    _render: &Render,
    mobjs: &Mobjs,
    line_vc: &mut [i32],
    validcount: i32,
    shooter: MobjId,
    angle: u32,
    distance: Fixed,
) -> (Fixed, Option<MobjId>) {
    let mut top_slope: Fixed = 100 * FRAC_UNIT / 160;
    let mut bottom_slope: Fixed = -100 * FRAC_UNIT / 160;
    let mut aim_slope: Fixed = 0;
    let mut target: Option<MobjId> = None;
    let (sx, sy, sz) = {
        let Some(m) = mobjs.get(shooter) else { return (0, None) };
        (m.x, m.y, m.z + (m.height >> 1) + 8 * FRAC_UNIT)
    };

    let ang = (angle >> crate::tables::ANGLETOFINESHIFT) & (crate::tables::FINEANGLES as u32 - 1);
    let ex = sx + ((distance >> FRAC_BITS) * crate::tables::finecosine_index(ang));
    let ey = sy + ((distance >> FRAC_BITS) * crate::tables::finesine_index(ang));

    let mut visit = |in_: &Intercept| -> bool {
        // Match vanilla PTR_AimTraverse's thing branch: stop at a hit.
        if let InterceptKind::Line(li) = in_.kind {
            let line = &level.lines[li];
            if line.flags & ML_TWOSIDED == 0 {
                return false;
            }
            let open = line_opening(level, line);
            if open.openbottom >= open.opentop {
                return false;
            }
            let dist = fixed_mul(distance, in_.frac);
            let slope_f = fixed_div(open.openbottom - sz, dist);
            let slope_t = fixed_div(open.opentop - sz, dist);
            if slope_f > bottom_slope {
                bottom_slope = slope_f;
            }
            if slope_t < top_slope {
                top_slope = slope_t;
            }
            if top_slope <= bottom_slope {
                return false;
            }
            return true;
        }
        if let InterceptKind::Thing(t) = in_.kind {
            let Some(tm) = mobjs.get(t) else { return true };
            if t == shooter || tm.flags & MF_SHOOTABLE == 0 {
                return true;
            }
            let dist = fixed_mul(distance, in_.frac);
            let thingtop = fixed_div(tm.z + tm.height - sz, dist);
            let thingbot = fixed_div(tm.z - sz, dist);
            if thingtop < bottom_slope {
                return true; // shot over the thing
            }
            if thingbot > top_slope {
                return true; // shot under the thing
            }
            let clipped_top = if thingtop > top_slope { top_slope } else { thingtop };
            let clipped_bot = if thingbot < bottom_slope { bottom_slope } else { thingbot };
            aim_slope = (clipped_top + clipped_bot) / 2;
            target = Some(t);
            return false;
        }
        true
    };

    let _ = path_traverse(
        level,
        mobjs,
        line_vc,
        validcount,
        sx,
        sy,
        ex,
        ey,
        PT_ADDLINES | PT_ADDTHINGS,
        &mut visit,
    );

    if target.is_some() {
        (aim_slope, target)
    } else {
        (0, None)
    }
}

/// `P_LineAttack` (shoot without damage application: the caller applies the
/// damage via `p_inter`). Returns the intercept kind that stopped the trace.
#[allow(clippy::too_many_arguments)]
pub fn line_attack_flat(
    level: &Level,
    _render: &Render,
    mobjs: &Mobjs,
    line_vc: &mut [i32],
    validcount: i32,
    shooter: MobjId,
    angle: u32,
    distance: Fixed,
    slope: Fixed,
) -> LineAttackHit {
    let (sx, sy, sz) = {
        let Some(m) = mobjs.get(shooter) else { return LineAttackHit::Nothing };
        (m.x, m.y, m.z + (m.height >> 1) + 8 * FRAC_UNIT)
    };

    let ang = (angle >> crate::tables::ANGLETOFINESHIFT) & (crate::tables::FINEANGLES as u32 - 1);
    let ex = sx + ((distance >> FRAC_BITS) * crate::tables::finecosine_index(ang));
    let ey = sy + ((distance >> FRAC_BITS) * crate::tables::finesine_index(ang));

    let mut hit: Option<LineAttackHit> = None;
    let _ = path_traverse(
        level,
        mobjs,
        line_vc,
        validcount,
        sx,
        sy,
        ex,
        ey,
        PT_ADDLINES | PT_ADDTHINGS,
        |in_: &Intercept| {
            match in_.kind {
                InterceptKind::Line(li) => {
                    let line = &level.lines[li];
                    if line.flags & ML_TWOSIDED == 0 {
                        hit = Some(LineAttackHit::Line(li));
                        return false;
                    }
                    let open = line_opening(level, line);
                    let dist = fixed_mul(distance, in_.frac);
                    let slope_f = fixed_div(open.openbottom - sz, dist);
                    if slope_f > slope {
                        hit = Some(LineAttackHit::Line(li));
                        return false;
                    }
                    let slope_t = fixed_div(open.opentop - sz, dist);
                    if slope_t < slope {
                        hit = Some(LineAttackHit::Line(li));
                        return false;
                    }
                    true
                }
                InterceptKind::Thing(t) => {
                    if t == shooter {
                        return true;
                    }
                    let Some(tm) = mobjs.get(t) else { return true };
                    if tm.flags & MF_SHOOTABLE == 0 {
                        return true;
                    }
                    let dist = fixed_mul(distance, in_.frac);
                    let thingtop = fixed_div(tm.z + tm.height - sz, dist);
                    if thingtop < slope {
                        return true;
                    }
                    let thingbot = fixed_div(tm.z - sz, dist);
                    if thingbot > slope {
                        return true;
                    }
                    hit = Some(LineAttackHit::Thing(t, slope));
                    false
                }
            }
        },
    );

    hit.unwrap_or(LineAttackHit::Nothing)
}

// ---------------------------------------------------------------------------
// Use (`P_UseLines` + `PTR_UseTraverse`, `p_map.c`) and player missiles
// (`P_SpawnPlayerMissile`, `p_mobj.c`)
// ---------------------------------------------------------------------------

/// `USERANGE`.
pub const USERANGE: Fixed = 64 * FRAC_UNIT;

impl World {
    /// `P_LineAttack` with the full `PTR_ShootTraverse` side effects:
    /// blood/puff spawning, damage and `P_ShootSpecialLine`.
    pub fn line_attack(
        &mut self,
        shooter: MobjId,
        angle: u32,
        distance: Fixed,
        slope: Fixed,
        damage: i32,
    ) -> LineAttackHit {
        let Some(m) = self.mobjs.get(shooter) else {
            return LineAttackHit::Nothing;
        };
        let (sx, sy, sz) = (m.x, m.y, m.z + (m.height >> 1) + 8 * FRAC_UNIT);

        let ang = (angle >> crate::tables::ANGLETOFINESHIFT) & (crate::tables::FINEANGLES as u32 - 1);
        let ex = sx + ((distance >> FRAC_BITS) * crate::tables::finecosine_index(ang));
        let ey = sy + ((distance >> FRAC_BITS) * crate::tables::finesine_index(ang));

        let mut hit: Option<(LineAttackHit, Fixed)> = None;
        let vid = self.bump_validcount();
        {
            let _ = path_traverse(
                &self.level,
                &self.mobjs,
                &mut self.line_vc,
                vid,
                sx,
                sy,
                ex,
                ey,
                PT_ADDLINES | PT_ADDTHINGS,
                |in_: &Intercept| {
                    match in_.kind {
                        InterceptKind::Line(li) => {
                            let line = &self.level.lines[li];
                            if line.flags & ML_TWOSIDED == 0 {
                                hit = Some((LineAttackHit::Line(li), in_.frac));
                                return false;
                            }
                            let open = line_opening(&self.level, line);
                            let dist = fixed_mul(distance, in_.frac);
                            let slope_f = fixed_div(open.openbottom - sz, dist);
                            if slope_f > slope {
                                hit = Some((LineAttackHit::Line(li), in_.frac));
                                return false;
                            }
                            let slope_t = fixed_div(open.opentop - sz, dist);
                            if slope_t < slope {
                                hit = Some((LineAttackHit::Line(li), in_.frac));
                                return false;
                            }
                            true
                        }
                        InterceptKind::Thing(t) => {
                            if t == shooter {
                                return true;
                            }
                            let Some(tm) = self.mobjs.get(t) else { return true };
                            if tm.flags & MF_SHOOTABLE == 0 {
                                return true;
                            }
                            let dist = fixed_mul(distance, in_.frac);
                            let thingtop = fixed_div(tm.z + tm.height - sz, dist);
                            if thingtop < slope {
                                return true;
                            }
                            let thingbot = fixed_div(tm.z - sz, dist);
                            if thingbot > slope {
                                return true;
                            }
                            hit = Some((LineAttackHit::Thing(t, slope), in_.frac));
                            false
                        }
                    }
                },
            );
        }
        let Some((h, frac)) = hit else {
            return LineAttackHit::Nothing;
        };
        match h {
            LineAttackHit::Line(li) => {
                // Impact specials.
                if self.level.lines[li].special != 0 {
                    crate::p_spec::shoot_special_line(self, li, shooter);
                }
                // Don't spawn a puff against the sky.
                let side_idx = crate::p_maputl::point_on_line_side(&self.level, sx, sy, &self.level.lines[li]);
                let sector = if side_idx == 0 {
                    self.level.lines[li].frontsector
                } else {
                    self.level.lines[li].backsector
                };
                let sky = sector
                    .map(|s| self.level.sectors[s].ceilingpic)
                    .map(|pic| pic == crate::wad::LumpName::from_bytes(b"F_SKY1"))
                    .unwrap_or(false);
                if !sky {
                    let px = sx + fixed_mul(ex - sx, frac);
                    let py = sy + fixed_mul(ey - sy, frac);
                    let pz = sz + fixed_mul(slope, fixed_mul(distance, frac));
                    let puff = crate::p_mobj::spawn_mobj(
                        &mut self.mobjs,
                        &mut self.thinkers,
                        &self.level,
                        &self.render,
                        crate::info::MobjType::MT_PUFF,
                        px,
                        py,
                        pz,
                    );
                    if let Some(m) = self.mobjs.get_mut(puff) {
                        m.momz = FRAC_UNIT;
                        let mut tics = m.tics - (self.rng.p.next() as i32 & 3);
                        if tics < 1 {
                            tics = 1;
                        }
                        m.tics = tics;
                    }
                }
            }
            LineAttackHit::Thing(t, _) => {
                // Blood or puff, then damage.
                let (tx, ty, tz, tnoblood) = {
                    let Some(tm) = self.mobjs.get(t) else {
                        return h;
                    };
                    (
                        tm.x,
                        tm.y,
                        tm.z + (tm.height >> 1),
                        tm.flags & crate::p_mobj::MF_NOBLOOD != 0,
                    )
                };
                let blood = if tnoblood {
                    crate::info::MobjType::MT_PUFF
                } else {
                    crate::info::MobjType::MT_BLOOD
                };
                let b = crate::p_mobj::spawn_mobj(
                    &mut self.mobjs,
                    &mut self.thinkers,
                    &self.level,
                    &self.render,
                    blood,
                    tx,
                    ty,
                    tz,
                );
                if blood == crate::info::MobjType::MT_PUFF {
                    if let Some(m) = self.mobjs.get_mut(b) {
                        m.momz = FRAC_UNIT;
                        let mut tics = m.tics - (self.rng.p.next() as i32 & 3);
                        if tics < 1 {
                            tics = 1;
                        }
                        m.tics = tics;
                    }
                }
                self.damage_mobj(t, Some(shooter), Some(shooter), damage);
            }
            LineAttackHit::Nothing => {}
        }
        h
    }

    /// `P_UseLines`: look for special lines in front of the player to
    /// activate (`PTR_UseTraverse` inline). Returns whether anything ran.
    pub fn use_lines(&mut self, pnum: usize) -> bool {
        let Some(mo) = self.players[pnum].mo else {
            return false;
        };
        let Some(m) = self.mobjs.get(mo) else {
            return false;
        };
        let (x1, y1, angle) = (m.x, m.y, m.angle);
        let an = (angle >> crate::tables::ANGLETOFINESHIFT) & (crate::tables::FINEANGLES as u32 - 1);
        let x2 = x1 + (USERANGE >> FRAC_BITS) * crate::tables::finecosine_index(an);
        let y2 = y1 + (USERANGE >> FRAC_BITS) * crate::tables::finesine_index(an);

        let mut used = false;
        let (px, py) = (x1, y1);
        {
            let vid = self.bump_validcount();
            let hit_line: Option<Option<(usize, usize)>> = {
                let mut found: Option<Option<(usize, usize)>> = Some(None);
                let _ = path_traverse(
                    &self.level,
                    &self.mobjs,
                    &mut self.line_vc,
                    vid,
                    px,
                    py,
                    x2,
                    y2,
                    PT_ADDLINES,
                    |in_: &Intercept| {
                        let InterceptKind::Line(li) = in_.kind else {
                            return true;
                        };
                        let line = &self.level.lines[li];
                        if line.special == 0 {
                            let open = line_opening(&self.level, line);
                            if open.openbottom >= open.opentop {
                                // can't use through a wall ("noway" is
                                // played after the traversal).
                                found = None;
                                return false;
                            }
                            // not a special line, but keep checking
                            return true;
                        }
                        let (tx, ty) = (self.mobjs.get(mo).map(|m| m.x).unwrap_or(x1),
                                       self.mobjs.get(mo).map(|m| m.y).unwrap_or(y1));
                        let side = crate::p_maputl::point_on_line_side(
                            &self.level,
                            tx,
                            ty,
                            line,
                        );
                        found = Some(Some((li, side as usize)));
                        false
                    },
                );
                found
            };
            // `None` outer = blocked by a closed wall; `Some(None)` = the
            // path is clear but no special line was crossed.
            let blocked = hit_line.is_none();
            let hit_line = hit_line.flatten();
            if blocked {
                self.play_sfx(crate::sounds::Sfx::sfx_noway);
            }
            if let Some((li, side)) = hit_line {
                if crate::p_switch::use_special_line(self, mo, li, side) {
                    used = true;
                }
                // can't use more than one special line in a row
            }
            used
        }
    }

    /// `P_SpawnPlayerMissile`: spawn a missile aimed at what the player is
    /// looking at (with the vanilla 1<<26 up/down aiming search).
    pub fn spawn_player_missile(&mut self, source: MobjId, ty: crate::info::MobjType) {
        let (sx, sy, an0) = {
            let Some(m) = self.mobjs.get(source) else { return };
            (m.x, m.y, m.angle)
        };

        const RANGE: Fixed = 16 * 64 * FRAC_UNIT;
        let mut an = an0;
        let vid = self.bump_validcount();
        let (mut slope, mut linetarget) = aim_line_attack(
            &self.level,
            &self.render,
            &self.mobjs,
            &mut self.line_vc,
            vid,
            source,
            an,
            RANGE,
        );
        if linetarget.is_none() {
            an = an.wrapping_add(1 << 26);
            let vid = self.bump_validcount();
            let (s, t) = aim_line_attack(
                &self.level,
                &self.render,
                &self.mobjs,
                &mut self.line_vc,
                vid,
                source,
                an,
                RANGE,
            );
            slope = s;
            linetarget = t;
            if linetarget.is_none() {
                an = an.wrapping_sub(2 << 26);
                let vid = self.bump_validcount();
                let (s, t) = aim_line_attack(
                    &self.level,
                    &self.render,
                    &self.mobjs,
                    &mut self.line_vc,
                    vid,
                    source,
                    an,
                    RANGE,
                );
                slope = s;
                linetarget = t;
                if linetarget.is_none() {
                    an = an0;
                    slope = 0;
                }
            }
        }

        let z = self.mobjs.get(source).map(|m| m.z).unwrap_or(0) + 4 * 8 * FRAC_UNIT;
        let th = crate::p_mobj::spawn_mobj(
            &mut self.mobjs,
            &mut self.thinkers,
            &self.level,
            &self.render,
            ty,
            sx,
            sy,
            z,
        );
        let seesound = self.mobjs.get(th).map(|m| m.info().seesound).unwrap_or(crate::sounds::Sfx::sfx_None);
        if seesound != crate::sounds::Sfx::sfx_None {
            self.play_sfx(seesound);
        }
        if let Some(m) = self.mobjs.get_mut(th) {
            m.target = Some(source);
            m.angle = an;
        }
        let speed = self.mobjs.get(th).map(|m| m.info().speed).unwrap_or(0);
        let a = (an >> crate::tables::ANGLETOFINESHIFT) & (crate::tables::FINEANGLES as u32 - 1);
        let momx = fixed_mul(speed, crate::tables::finecosine_index(a));
        let momy = fixed_mul(speed, crate::tables::finesine_index(a));
        let momz = fixed_mul(speed, slope);
        if let Some(m) = self.mobjs.get_mut(th) {
            m.momx = momx;
            m.momy = momy;
            m.momz = momz;
        }

        self.check_missile_spawn(th);
    }
}

impl World {
    /// Post-move gameplay hooks that the pure move code defers, mirroring the
    /// tail of vanilla `P_TryMove`:
    ///
    /// * **crossed special lines** — lines whose side changed between the
    ///   old and new spot fire `P_CrossSpecialLine` (the vanilla `spechit`
    ///   processing);
    /// * **pickups** — a player overlapping an `MF_SPECIAL` thing runs
    ///   `P_TouchSpecialThing` (and the deathmatch respawn queue gets the
    ///   taken item's spawn spot, like `P_RemoveMobj` does).
    pub fn post_move_hooks(&mut self, id: MobjId, oldx: Fixed, oldy: Fixed) {
        let Some((newx, newy, radius, is_player)) = self.mobjs.get(id).map(|m| {
            (m.x, m.y, m.radius, m.player.is_some())
        }) else {
            return;
        };
        if newx == oldx && newy == oldy {
            return;
        }

        // ---- crossed special lines --------------------------------------
        if let Some((orgx, orgy, bw, _bh)) = blockmap_dims(&self.level) {
            let vid = self.bump_validcount();
            let mut crossed: Vec<(usize, i32)> = Vec::new();
            let xl = ((oldx.min(newx) - radius - orgx) >> MAPBLOCKSHIFT) - 1;
            let _ = bw;
            let xh = ((oldx.max(newx) + radius - orgx) >> MAPBLOCKSHIFT) + 1;
            let yl = ((oldy.min(newy) - radius - orgy) >> MAPBLOCKSHIFT) - 1;
            let yh = ((oldy.max(newy) + radius - orgy) >> MAPBLOCKSHIFT) + 1;
            for bx in xl..=xh {
                for by in yl..=yh {
                    let _ = block_lines(&self.level, &mut self.line_vc, vid, bx, by, |idx, line| {
                        if line.special == 0 {
                            return true;
                        }
                        let side_old = crate::p_maputl::point_on_line_side(
                            &self.level, oldx, oldy, line,
                        );
                        let side_new = crate::p_maputl::point_on_line_side(
                            &self.level, newx, newy, line,
                        );
                        if side_old != side_new {
                            crossed.push((idx, side_new));
                        }
                        true
                    });
                }
            }
            for (li, side) in crossed {
                crate::p_spec::cross_special_line(self, li, side as usize, id);
                if !self.mobjs.is_live(id) {
                    return;
                }
            }
        }

        // ---- pickups -----------------------------------------------------
        if !is_player {
            return;
        }
        if let Some((orgx, orgy, bw, bh)) = blockmap_dims(&self.level) {
            let xl = (newx - radius - orgx - MAXRADIUS) >> MAPBLOCKSHIFT;
            let xh = (newx + radius - orgx + MAXRADIUS) >> MAPBLOCKSHIFT;
            let yl = (newy - radius - orgy - MAXRADIUS) >> MAPBLOCKSHIFT;
            let yh = (newy + radius - orgy + MAXRADIUS) >> MAPBLOCKSHIFT;
            let mut buf: Vec<MobjId> = Vec::new();
            for bx in xl..=xh {
                for by in yl..=yh {
                    let Some(block) = block_at(orgx, orgy, bw, bh, bx, by) else {
                        continue;
                    };
                    buf.clear();
                    self.mobjs.collect_block(block, &mut buf);
                    for special in buf.iter().copied() {
                        let Some(sp) = self.mobjs.get(special) else {
                            continue;
                        };
                        if sp.flags & MF_SPECIAL == 0 {
                            continue;
                        }
                        let blockdist = sp.radius + radius;
                        if (sp.x - newx).abs() >= blockdist || (sp.y - newy).abs() >= blockdist {
                            continue;
                        }
                        // Record the respawn spot before the thing goes away.
                        let (ty, spawnpoint, dropped) =
                            self.mobjs.get(special).map(|m| (m.ty, m.spawnpoint, m.flags & crate::p_mobj::MF_DROPPED != 0)).unwrap_or((crate::info::MobjType::MT_PLAYER, Default::default(), true));
                        if !dropped
                            && ty != crate::info::MobjType::MT_INV
                            && ty != crate::info::MobjType::MT_INS
                        {
                            if self.item_respawn_queue.len() >= crate::p_mobj::ITEMQUESIZE {
                                self.item_respawn_queue.remove(0);
                            }
                            self.item_respawn_queue
                                .push((spawnpoint, self.leveltime));
                        }
                        let ctx = self.interaction();
                        crate::p_inter::touch_special_thing(
                            &mut self.mobjs,
                            &mut self.thinkers,
                            &mut self.players,
                            &ctx,
                            special,
                            id,
                        );
                    }
                }
            }
        }
    }

    /// `P_ChangeSector` (+ `PIT_ChangeSector`): re-check heights for every
    /// thing in the sector after a plane moved; crunch what does not fit.
    ///
    /// Vanilla iterates the sector's blockmap `blockbox`; the port walks the
    /// sector thing list instead, which covers exactly the things standing
    /// in the sector (things in neighbouring blocks are only re-clipped, a
    /// no-op for their own sector's heights).
    pub fn change_sector(&mut self, sector: usize, crunch: bool) -> bool {
        use crate::info::{MobjType, StateId};

        let mut nofit = false;

        let mut things = Vec::new();
        let mut cur = self.mobjs.sector_head(sector);
        while let Some(id) = cur {
            things.push(id);
            cur = self.mobjs.get(id).and_then(|m| m.snext);
        }

        for id in things {
            if !self.mobjs.is_live(id) {
                continue;
            }
            let vid = self.bump_validcount();
            if thing_height_clip(
                &self.level,
                &self.render,
                &mut self.mobjs,
                &mut self.line_vc,
                vid,
                id,
            ) {
                // keep checking
                continue;
            }

            let Some(m) = self.mobjs.get(id) else {
                continue;
            };

            // crunch bodies to giblets
            if m.health <= 0 {
                self.set_mobj_state(id, StateId::S_GIBS);
                if let Some(m) = self.mobjs.get_mut(id) {
                    m.flags &= !MF_SOLID;
                    m.height = 0;
                    m.radius = 0;
                }
                // keep checking
                continue;
            }

            // crunch dropped items
            if m.flags & MF_DROPPED != 0 {
                crate::p_mobj::remove_mobj(&mut self.mobjs, &mut self.thinkers, id);
                // keep checking
                continue;
            }

            if m.flags & MF_SHOOTABLE == 0 {
                // assume it is bloody gibs or something
                continue;
            }

            nofit = true;

            if crunch && self.leveltime & 3 == 0 {
                self.damage_mobj(id, None, None, 10);

                // spray blood in a random direction
                let Some(m) = self.mobjs.get(id) else {
                    continue;
                };
                let (x, y, z) = (m.x, m.y, m.z + m.height / 2);
                let blood = crate::p_mobj::spawn_mobj(
                    &mut self.mobjs,
                    &mut self.thinkers,
                    &self.level,
                    &self.render,
                    MobjType::MT_BLOOD,
                    x,
                    y,
                    z,
                );
                let r1 = self.rng.p.next() as i32;
                let r2 = self.rng.p.next() as i32;
                let momx = (r1 - r2) << 12;
                let r1 = self.rng.p.next() as i32;
                let r2 = self.rng.p.next() as i32;
                let momy = (r1 - r2) << 12;
                if let Some(b) = self.mobjs.get_mut(blood) {
                    b.momx = momx;
                    b.momy = momy;
                }
            }
        }

        nofit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::MobjType;
    use crate::level::{Line, Sector, SlopeType, Subsector, Vertex};
    use crate::p_mobj::{Mobj, MobjId};
    use alloc::vec;

    fn sector(floor: Fixed, ceiling: Fixed) -> Sector {
        Sector {
            floorheight: floor,
            ceilingheight: ceiling,
            ..Sector::default()
        }
    }

    /// A level with one subsector pointing at `sector` and (optionally) a
    /// blockmap holding `lines`.
    fn level(front: Sector, lines: Vec<Line>, blockmap: Vec<i16>) -> Level {
        Level {
            vertexes: vec![Vertex { x: -(64 << 16), y: 0 }, Vertex { x: 64 << 16, y: 0 }],
            sectors: vec![front, sector(0, 64 << 16)],
            lines,
            subsectors: vec![Subsector {
                sector: 0,
                numlines: 0,
                firstline: 0,
            }],
            blockmap: crate::level::Blockmap {
                data: blockmap,
                ..Default::default()
            },
            ..Level::default()
        }
    }

    fn spawn(mobjs: &mut Mobjs, x: Fixed, y: Fixed) -> MobjId {
        mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, x, y, 0))
    }

    #[test]
    fn check_position_uses_subsector_heights() {
        let lvl = level(sector(16 << 16, 96 << 16), Vec::new(), Vec::new());
        let render = Render::new();
        let mut mobjs = Mobjs::new(2);
        let id = spawn(&mut mobjs, 0, 0);
        let mut vc = Vec::new();

        let check = check_position(&lvl, &render, &mobjs, &mut vc, 1, id, 0, 0).unwrap();
        assert_eq!(check.floorz, 16 << 16);
        assert_eq!(check.ceilingz, 96 << 16);
    }

    #[test]
    fn one_sided_line_blocks() {
        let line = Line {
            v1: 0,
            v2: 1,
            dx: 128 << 16,
            dy: 0,
            slopetype: SlopeType::Horizontal,
            bbox: [-(64 << 16), 64 << 16, -(64 << 16), 64 << 16],
            frontsector: Some(0),
            ..Line::default()
        };
        // 1x1 blockmap holding line 0.
        let lvl = level(sector(0, 128 << 16), vec![line], vec![0, 0, 1, 1, 5, 0, -1]);
        let render = Render::new();
        let mut mobjs = Mobjs::new(2);
        let id = spawn(&mut mobjs, 0, 0);
        let mut vc = vec![0; 1];

        assert!(check_position(&lvl, &render, &mobjs, &mut vc, 1, id, 0, 0).is_none());
    }

    #[test]
    fn two_sided_line_lowers_ceiling() {
        let line = Line {
            v1: 0,
            v2: 1,
            dx: 128 << 16,
            dy: 0,
            slopetype: SlopeType::Horizontal,
            bbox: [-(64 << 16), 64 << 16, -(64 << 16), 64 << 16],
            frontsector: Some(0),
            backsector: Some(1),
            ..Line::default()
        };
        // sector0 ceiling 128; sector1 ceiling 64 -> opening caps at 64.
        let lvl = level(sector(0, 128 << 16), vec![line], vec![0, 0, 1, 1, 5, 0, -1]);
        let render = Render::new();
        let mut mobjs = Mobjs::new(2);
        let id = spawn(&mut mobjs, 0, 0);
        let mut vc = vec![0; 1];

        let check = check_position(&lvl, &render, &mobjs, &mut vc, 1, id, 0, 0).unwrap();
        assert_eq!(check.ceilingz, 64 << 16);
        assert_eq!(check.ceilingline, Some(0));
    }

    #[test]
    fn try_move_relinks_and_updates() {
        let lvl = level(sector(0, 128 << 16), Vec::new(), Vec::new());
        let render = Render::new();
        let mut mobjs = Mobjs::new(2);
        let id = spawn(&mut mobjs, 0, 0);
        let mut vc = Vec::new();

        assert!(try_move(
            &lvl,
            &render,
            &mut mobjs,
            &mut vc,
            1,
            id,
            32 << 16,
            8 << 16
        ));

        let m = mobjs.get(id).unwrap();
        assert_eq!(m.x, 32 << 16);
        assert_eq!(m.y, 8 << 16);
        assert_eq!(m.floorz, 0);
        assert_eq!(m.ceilingz, 128 << 16);
        assert_eq!(mobjs.sector_head(0), Some(id));
    }
}

// ---------------------------------------------------------------------------
// Sector change crunching (`P_ChangeSector` + `PIT_ChangeSector`)
// ---------------------------------------------------------------------------

