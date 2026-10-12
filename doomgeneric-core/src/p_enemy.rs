//! Monster AI, port of the action half of `p_enemy.c`.
//!
//! All functions operate on [`World`] (`p_world` owns the state). Vanilla's
//! `P_UseSpecialLine` (door opening while wandering) is deferred — `P_Move`
//! stops at blocked lines instead.

use crate::fixed::{fixed_mul, Fixed, FRAC_UNIT};
use crate::info::{MobjType, StateId};
use crate::p_map::aim_line_attack;
use crate::p_maputl::aprox_distance;
use crate::p_mobj::{
    MobjId, MF_AMBUSH, MF_FLOAT, MF_JUSTATTACKED, MF_JUSTHIT, MF_SHOOTABLE, MF_SHADOW, MF_SKULLFLY,
    MF_SOLID,
};
use crate::p_sight;
use crate::p_world::World;
use crate::r_main::point_to_angle_from;
use crate::sounds::Sfx;
use crate::tables::{finecosine_index, finesine_index, ANGLETOFINESHIFT, ANG90, ANG270};

// `dirtype_t` (`p_enemy.c`).
pub const DI_EAST: i32 = 0;
pub const DI_NORTHEAST: i32 = 1;
pub const DI_NORTH: i32 = 2;
pub const DI_NORTHWEST: i32 = 3;
pub const DI_WEST: i32 = 4;
pub const DI_SOUTHWEST: i32 = 5;
pub const DI_SOUTH: i32 = 6;
pub const DI_SOUTHEAST: i32 = 7;
pub const DI_NODIR: i32 = 8;

/// `xspeed` (`p_enemy.c`).
pub const XSPEED: [i32; 8] = [FRAC_UNIT, 47000, 0, -47000, -FRAC_UNIT, -47000, 0, 47000];
/// `yspeed` (`p_enemy.c`).
pub const YSPEED: [i32; 8] = [0, 47000, FRAC_UNIT, 47000, 0, -47000, -FRAC_UNIT, -47000];

/// `opposite` (`p_enemy.c`).
pub const OPPOSITE: [i32; 9] = [
    DI_WEST, DI_SOUTHWEST, DI_SOUTH, DI_SOUTHEAST, DI_EAST, DI_NORTHEAST, DI_NORTH, DI_NORTHWEST,
    DI_NODIR,
];

/// `diags` (`p_enemy.c`).
pub const DIAGS: [i32; 4] = [DI_NORTHWEST, DI_NORTHEAST, DI_SOUTHWEST, DI_SOUTHEAST];

/// `P_CheckMeleeRange`.
pub fn check_melee_range(world: &mut World, id: MobjId) -> bool {
    let (dist_ok, t) = {
        let Some(a) = world.mobjs.get(id) else { return false };
        let Some(t) = a.target else { return false };
        let Some(tgt) = world.mobjs.get(t) else { return false };
        let dist = aprox_distance(tgt.x - a.x, tgt.y - a.y);
        (
            dist < MELEERANGE - 20 * FRAC_UNIT + tgt.info().radius,
            t,
        )
    };
    if !dist_ok {
        return false;
    }
    world.sight_between(id, t)
}

/// `P_CheckMissileRange` (properly bumps `validcount` for the sight check).
pub fn check_missile_range_world(world: &mut World, id: MobjId) -> bool {
    let Some(t) = world.mobjs.get(id).and_then(|m| m.target) else {
        return false;
    };
    {
        let a = world.mobjs.get(id).unwrap();
        if a.flags & MF_JUSTHIT != 0 {
            let m = world.mobjs.get_mut(id).unwrap();
            m.flags &= !MF_JUSTHIT;
            return true; // fight back!
        }
        if a.reactiontime != 0 {
            return false;
        }
    }
    if !world.sight_between(id, t) {
        return false;
    }

    let (a, tgt) = (world.mobjs.get(id).unwrap(), world.mobjs.get(t).unwrap());
    let mut dist = aprox_distance(a.x - tgt.x, a.y - tgt.y) - 64 * FRAC_UNIT;

    if a.info().meleestate == StateId::S_NULL {
        dist -= 128 * FRAC_UNIT; // no melee attack, so fire more
    }
    dist >>= 16;

    if dist > 200 {
        dist = 200;
    }
    if a.ty == MobjType::MT_VILE && dist > 14 * 64 {
        return false;
    }
    if a.ty == MobjType::MT_UNDEAD {
        if dist < 196 {
            return false;
        }
        dist >>= 1;
    }
    if a.ty == MobjType::MT_CYBORG || a.ty == MobjType::MT_SPIDER || a.ty == MobjType::MT_SKULL {
        dist >>= 1;
    }
    if a.ty == MobjType::MT_CYBORG && dist > 160 {
        dist = 160;
    }
    (world.rng.p.next() as i32) < dist
}

/// `P_CheckMissileRange`.
pub fn check_missile_range_ext(
    level: &crate::level::Level,
    mobjs: &crate::p_mobj::Mobjs,
    line_vc: &mut [i32],
    validcount: i32,
    rng: &mut crate::random::Random,
    id: MobjId,
) -> bool {
    let (Some(t),) = (mobjs.get(id).and_then(|m| m.target),) else {
        return false;
    };
    {
        let a = mobjs.get(id).unwrap();
        if a.flags & MF_JUSTHIT != 0 {
            return true; // fight back!
        }
        if a.reactiontime != 0 {
            return false;
        }
    }
    if !p_sight::check_sight(level, mobjs, line_vc, validcount, id, t) {
        return false;
    }

    let (a, tgt) = (mobjs.get(id).unwrap(), mobjs.get(t).unwrap());
    let mut dist = aprox_distance(a.x - tgt.x, a.y - tgt.y) - 64 * FRAC_UNIT;

    if a.info().meleestate == StateId::S_NULL {
        dist -= 128 * FRAC_UNIT; // no melee attack, so fire more
    }
    dist >>= 16;

    if dist > 200 {
        dist = 200;
    }
    if a.ty == MobjType::MT_VILE && dist > 14 * 64 {
        return false;
    }
    if a.ty == MobjType::MT_UNDEAD {
        if dist < 196 {
            return false;
        }
        dist >>= 1;
    }
    if a.ty == MobjType::MT_CYBORG || a.ty == MobjType::MT_SPIDER || a.ty == MobjType::MT_SKULL {
        dist >>= 1;
    }
    if a.ty == MobjType::MT_CYBORG && dist > 160 {
        dist = 160;
    }

    (rng.p.next() as i32) < dist
}

/// `P_Move` minus door opening.
pub fn p_move(world: &mut World, id: MobjId) -> bool {
    let Some(m) = world.mobjs.get(id) else { return false };
    let md = m.movedir;
    if md == DI_NODIR || md < 0 || md as usize >= 8 {
        return false;
    }
    let (speed, rx, ry) = (m.info().speed, m.x, m.y);
    let tryx = rx + speed * XSPEED[md as usize];
    let tryy = ry + speed * YSPEED[md as usize];
    let vid = world.bump_validcount();
    let ok = crate::p_map::try_move(
        &world.level,
        &world.render,
        &mut world.mobjs,
        &mut world.line_vc,
        vid,
        id,
        tryx,
        tryy,
    );
    if !ok {
        return false;
    }
    if let Some(m) = world.mobjs.get_mut(id) {
        if m.flags & MF_FLOAT == 0 {
            m.z = m.floorz;
        }
    }
    true
}

/// `P_TryWalk`.
pub fn p_try_walk(world: &mut World, id: MobjId) -> bool {
    if !p_move(world, id) {
        return false;
    }
    let r = world.rng.p.next() as i32;
    if let Some(m) = world.mobjs.get_mut(id) {
        m.movecount = r & 15;
    }
    true
}

fn set_movedir(world: &mut World, id: MobjId, dir: i32) {
    if let Some(m) = world.mobjs.get_mut(id) {
        m.movedir = dir;
    }
}

/// `P_NewChaseDir`.
pub fn p_new_chase_dir(world: &mut World, id: MobjId) {
    let (rx, ry, tx, ty, olddir) = {
        let Some(m) = world.mobjs.get(id) else { return };
        let Some(t) = m.target else { return };
        let (x, y) = (m.x, m.y);
        let tod = world.mobjs.get(t).map(|b| (b.x, b.y));
        let Some((tx, ty)) = tod else { return };
        (x, y, tx, ty, m.movedir)
    };

    let turnaround = OPPOSITE[clamp_dir(olddir) as usize];
    let deltax = tx - rx;
    let deltay = ty - ry;

    let mut d = [DI_NODIR, DI_NODIR, DI_NODIR];
    if deltax > 10 * FRAC_UNIT {
        d[1] = DI_EAST;
    } else if deltax < -10 * FRAC_UNIT {
        d[1] = DI_WEST;
    }
    if deltay < -10 * FRAC_UNIT {
        d[2] = DI_SOUTH;
    } else if deltay > 10 * FRAC_UNIT {
        d[2] = DI_NORTH;
    }

    // Try the direct diagonal first.
    if d[1] != DI_NODIR && d[2] != DI_NODIR {
        let diag = DIAGS[((deltay < 0) as usize) * 2 + (deltax > 0) as usize];
        if diag != turnaround {
            set_movedir(world, id, diag);
            if p_try_walk(world, id) {
                return;
            }
        }
    }

    // Random half of the time prefer the second direction.
    if (world.rng.p.next() as i32) > 200 || deltax.abs() <= deltay.abs() {
        d.swap(1, 2);
    }
    if d[1] == turnaround {
        d[1] = DI_NODIR;
    }
    if d[2] == turnaround {
        d[2] = DI_NODIR;
    }
    for dir in [d[1], d[2]] {
        if dir != DI_NODIR {
            set_movedir(world, id, dir);
            if p_try_walk(world, id) {
                return;
            }
        }
    }

    if olddir != DI_NODIR {
        set_movedir(world, id, clamp_dir(olddir));
        if p_try_walk(world, id) {
            return;
        }
    }

    // Randomly search in any direction.
    if (world.rng.p.next() & 1) != 0 {
        for tdir in 0..8 {
            if tdir != turnaround {
                set_movedir(world, id, tdir);
                if p_try_walk(world, id) {
                    return;
                }
            }
        }
    } else {
        for tdir in (0..8).rev() {
            if tdir != turnaround {
                set_movedir(world, id, tdir);
                if p_try_walk(world, id) {
                    return;
                }
            }
        }
    }

    if turnaround != DI_NODIR {
        set_movedir(world, id, turnaround);
        if p_try_walk(world, id) {
            return;
        }
    }
    if let Some(m) = world.mobjs.get_mut(id) {
        m.movedir = DI_NODIR;
    }
}

#[inline]
fn clamp_dir(d: i32) -> i32 {
    if (0..=8).contains(&d) { d } else { DI_NODIR }
}

/// `P_LookForPlayers`: scans player slots for a visible, live, shootable
/// target and returns its player mobj id.
pub fn look_for_players(world: &mut World, id: MobjId, allaround: bool) -> bool {
    let (ax, ay, ang) = {
        let Some(a) = world.mobjs.get(id) else { return false };
        (a.x, a.y, a.angle)
    };

    let mut lastlook = world.mobjs.get(id).map(|m| m.lastlook).unwrap_or(0);
    let mut count = 0;
    let stop = (lastlook - 1) & 3;
    loop {
        if count == 2 || lastlook == stop {
            return false;
        }
        if !world.player_in_game[lastlook as usize] {
            count += 1;
            lastlook = (lastlook + 1) & 3;
            continue;
        }
        count += 1;
        let Some(pm) = world.players[lastlook as usize].mo else {
            lastlook = (lastlook + 1) & 3;
            continue;
        };
        let sight_ok = {
            let vid = world.bump_validcount();
            p_sight::check_sight(&world.level, &world.mobjs, &mut world.line_vc, vid, id, pm)
        };
        let mut ok = sight_ok;
        if ok && !allaround {
            let an = point_to_angle_from(ax, ay, pm2_x(world, pm), pm2_y(world, pm))
                .wrapping_sub(ang);
            if an > ANG90 && an < ANG270 {
                let dist = aprox_distance(pm2_x(world, pm) - ax, pm2_y(world, pm) - ay);
                if dist > 64 * FRAC_UNIT {
                    ok = false;
                }
            }
        }
        if ok {
            if let Some(m) = world.mobjs.get_mut(id) {
                m.lastlook = (lastlook + 1) & 3;
                m.target = Some(pm);
            }
            return true;
        }
        lastlook = (lastlook + 1) & 3;
    }
}

#[inline]
fn pm2_x(w: &World, pm: MobjId) -> i32 {
    w.mobjs.get(pm).map(|m| m.x).unwrap_or(0)
}

#[inline]
fn pm2_y(w: &World, pm: MobjId) -> i32 {
    w.mobjs.get(pm).map(|m| m.y).unwrap_or(0)
}

// `p_enemy.c`/`p_local.h` attack constants.
pub const MELEERANGE: Fixed = 64 * FRAC_UNIT;
pub const MISSILERANGE: Fixed = 32 * 64 * FRAC_UNIT;
const SKULLSPEED: Fixed = 20 * FRAC_UNIT;
const TRACEANGLE: u32 = 0xc000000;

/// Run `P_LineAttack` and apply `damage` to the thing it hits.
fn line_attack_with(world: &mut World, id: MobjId, angle: u32, slope: Fixed, damage: i32) {
    world.line_attack(id, angle, MISSILERANGE, slope, damage);
}

/// `P_RecursiveSound`: flood-fill adjacent sectors with a sound.
fn recursive_sound(world: &mut World, sec: usize, soundblocks: i32, target: MobjId, validcount: i32) {
    let sector = &mut world.level.sectors[sec];
    if sector.validcount == validcount && sector.soundtraversed <= soundblocks + 1 {
        return; // already flooded
    }
    sector.validcount = validcount;
    sector.soundtraversed = soundblocks + 1;
    sector.soundtarget = Some(target);

    let lines = sector.lines.clone();
    for li in lines {
        let line = &world.level.lines[li as usize];
        if line.flags & crate::doomdef::ML_TWOSIDED == 0 {
            continue;
        }
        let open = crate::p_maputl::line_opening(&world.level, line);
        if open.openbottom >= open.opentop {
            continue; // closed door
        }
        // The sector on the other side of this line.
        let other = if line.frontsector == Some(sec) {
            line.backsector
        } else {
            line.frontsector
        };
        let Some(other) = other else { continue };
        if line.flags & crate::doomdef::ML_SOUNDBLOCK != 0 {
            if soundblocks == 0 {
                recursive_sound(world, other, 1, target, validcount);
            }
        } else {
            recursive_sound(world, other, soundblocks, target, validcount);
        }
    }
}

/// `P_NoiseAlert`: if a monster yells at a player, it will alert other
/// monsters to the player.
pub fn noise_alert(world: &mut World, target: MobjId, emitter: MobjId) {
    let Some(sector) = world.mobjs.get(emitter).map(|m| m.sector) else {
        return;
    };
    let vid = world.bump_validcount();
    recursive_sound(world, sector, 0, target, vid);
}

/// `A_Look`.
pub fn a_look(world: &mut World, id: MobjId) {
    if let Some(m) = world.mobjs.get_mut(id) {
        m.threshold = 0;
    }
    let (sector_idx, ambush, seesound_of) = {
        let Some(a) = world.mobjs.get(id) else { return };
        (a.sector, a.flags & MF_AMBUSH != 0, a.info().seesound)
    };

    let mut see = false;
    if let Some(t) = world.level.sectors[sector_idx].soundtarget {
        let shootable = world.mobjs.get(t).map(|m| m.flags & MF_SHOOTABLE != 0).unwrap_or(false);
        if shootable {
            if let Some(m) = world.mobjs.get_mut(id) {
                m.target = Some(t);
            }
            if ambush {
                if world.sight_between(id, t) {
                    see = true;
                }
            } else {
                see = true;
            }
        }
    }

    if !see && !look_for_players(world, id, false) {
        return;
    }

    // "seeyou" block: pick the see sound variant, then enter the see state.
    if seesound_of != Sfx::sfx_None {
        let pick = match seesound_of {
            Sfx::sfx_posit1 | Sfx::sfx_posit2 | Sfx::sfx_posit3 => {
                let which = world.rng.p.next() as usize % 3;
                match which {
                    0 => Sfx::sfx_posit1,
                    1 => Sfx::sfx_posit2,
                    _ => Sfx::sfx_posit3,
                }
            }
            Sfx::sfx_bgsit1 | Sfx::sfx_bgsit2 => {
                if (world.rng.p.next() & 1) != 0 {
                    Sfx::sfx_bgsit2
                } else {
                    Sfx::sfx_bgsit1
                }
            }
            other => other,
        };
        world.play_sfx(pick);
    }
    let seestate = world.mobjs.get(id).map(|m| m.info().seestate).unwrap_or(StateId::S_NULL);
    world.set_mobj_state(id, seestate);
}

/// `A_Chase`.
pub fn a_chase(world: &mut World, id: MobjId) {
    if let Some(m) = world.mobjs.get_mut(id) {
        if m.reactiontime > 0 {
            m.reactiontime -= 1;
        }
    }

    {
        let (threshold, target) = {
            let a = world.mobjs.get(id).unwrap();
            (a.threshold, a.target)
        };
        if threshold != 0 {
            let target_alive = target
                .map(|t| world.mobjs.get(t).map(|m| m.health > 0).unwrap_or(false))
                .unwrap_or(false);
            if !target_alive {
                if let Some(m) = world.mobjs.get_mut(id) {
                    m.threshold = 0;
                }
            } else if let Some(m) = world.mobjs.get_mut(id) {
                m.threshold -= 1;
            }
        }
    }

    // Turn towards the movement direction.
    {
        let movedir = world.mobjs.get(id).map(|m| m.movedir).unwrap_or(-1);
        if (0..8).contains(&movedir) {
            let m = world.mobjs.get_mut(id).unwrap();
            m.angle &= 7 << 29;
            let delta = m.angle.wrapping_sub((movedir << 29) as u32) as i32;
            if delta > 0 {
                m.angle = m.angle.wrapping_sub(ANG90 / 2);
            } else if delta < 0 {
                m.angle = m.angle.wrapping_add(ANG90 / 2);
            }
        }
    }

    // Look for a new target if the old one is gone or unshootable.
    {
        let (target, target_shootable) = {
            let a = world.mobjs.get(id).unwrap();
            let shootable = a
                .target
                .map(|t| world.mobjs.get(t).map(|m| m.flags & MF_SHOOTABLE != 0).unwrap_or(false))
                .unwrap_or(false);
            (a.target, shootable)
        };
        if target.is_none() || !target_shootable {
            if look_for_players(world, id, true) {
                return;
            }
            let spawnstate = world.mobjs.get(id).map(|m| m.info().spawnstate).unwrap_or(StateId::S_NULL);
            world.set_mobj_state(id, spawnstate);
            return;
        }
    }

    // Do not attack twice in a row.
    if world.mobjs.get(id).map(|m| m.flags & MF_JUSTATTACKED != 0).unwrap_or(false) {
        if let Some(m) = world.mobjs.get_mut(id) {
            m.flags &= !MF_JUSTATTACKED;
        }
        if world.skill != 4 && !world.fastparm {
            p_new_chase_dir(world, id);
        }
        return;
    }

    // Melee attack.
    {
        let meleestate = world.mobjs.get(id).map(|m| m.info().meleestate).unwrap_or(StateId::S_NULL);
        if meleestate != StateId::S_NULL && check_melee_range(world, id) {
            let asound = world.mobjs.get(id).map(|m| m.info().attacksound).unwrap_or(Sfx::sfx_None);
            world.play_sfx(asound);
            world.set_mobj_state(id, meleestate);
            return;
        }
    }

    // Missile attack.
    {
        let missilestate = world.mobjs.get(id).map(|m| m.info().missilestate).unwrap_or(StateId::S_NULL);
        if missilestate != StateId::S_NULL {
            let movecount = world.mobjs.get(id).map(|m| m.movecount).unwrap_or(0);
            if !(world.skill < 4 && !world.fastparm && movecount != 0)
                && check_missile_range_world(world, id)
            {
                world.set_mobj_state(id, missilestate);
                if let Some(m) = world.mobjs.get_mut(id) {
                    m.flags |= MF_JUSTATTACKED;
                }
                return;
            }
        }
    }

    // Possibly choose another target (netgame).
    {
        let threshold = world.mobjs.get(id).map(|m| m.threshold).unwrap_or(0);
        if world.netgame && threshold == 0 {
            let seen = {
                let t = world.mobjs.get(id).and_then(|m| m.target);
                t.map(|t| world.sight_between(id, t)).unwrap_or(false)
            };
            if !seen && !look_for_players(world, id, true) {
                return;
            }
        }
    }

    // Chase towards the player.
    let mut movecount = world.mobjs.get(id).map(|m| m.movecount).unwrap_or(0);
    movecount -= 1;
    let moved = p_move(world, id);
    if let Some(m) = world.mobjs.get_mut(id) {
        m.movecount = movecount;
    }
    if movecount < 0 || !moved {
        p_new_chase_dir(world, id);
    }

    // Active sound.
    let active = world.mobjs.get(id).map(|m| m.info().activesound).unwrap_or(Sfx::sfx_None);
    if active != Sfx::sfx_None && (world.rng.p.next() as i32) < 3 {
        world.play_sfx(active);
    }
}

/// `A_FaceTarget`.
pub fn a_face_target(world: &mut World, id: MobjId) {
    let (tx, ty, shadow) = {
        let Some(a) = world.mobjs.get(id) else { return };
        let Some(t) = a.target else { return };
        let Some(tm) = world.mobjs.get(t) else { return };
        (tm.x, tm.y, tm.flags & MF_SHADOW != 0)
    };
    let (ax, ay) = {
        let a = world.mobjs.get(id).unwrap();
        (a.x, a.y)
    };
    let mut angle = point_to_angle_from(ax, ay, tx, ty);
    if shadow {
        let r1 = world.rng.p.next() as i32;
        let r2 = world.rng.p.next() as i32;
        angle = angle.wrapping_add(((r1 - r2) << 21) as u32);
    }
    if let Some(m) = world.mobjs.get_mut(id) {
        m.flags &= !MF_AMBUSH;
        m.angle = angle;
    }
}

/// `A_SargAttack`.
pub fn a_sarg_attack(world: &mut World, id: MobjId) {
    let Some(t) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if check_melee_range(world, id) {
        let damage = (world.rng.p.next() as i32 % 10 + 1) * 4;
        world.damage_mobj(t, Some(id), Some(id), damage);
    }
}

/// `A_TroopAttack`.
pub fn a_troop_attack(world: &mut World, id: MobjId) {
    let Some(t) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if check_melee_range(world, id) {
        world.play_sfx(Sfx::sfx_claw);
        let damage = (world.rng.p.next() as i32 % 8 + 1) * 3;
        world.damage_mobj(t, Some(id), Some(id), damage);
        return;
    }
    world.spawn_missile(id, t, MobjType::MT_TROOPSHOT);
}

/// `A_HeadAttack`.
pub fn a_head_attack(world: &mut World, id: MobjId) {
    let Some(t) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if check_melee_range(world, id) {
        let damage = (world.rng.p.next() as i32 % 6 + 1) * 10;
        world.damage_mobj(t, Some(id), Some(id), damage);
        return;
    }
    world.spawn_missile(id, t, MobjType::MT_HEADSHOT);
}

/// `A_BruisAttack`.
pub fn a_bruis_attack(world: &mut World, id: MobjId) {
    let Some(t) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if check_melee_range(world, id) {
        world.play_sfx(Sfx::sfx_claw);
        let damage = (world.rng.p.next() as i32 % 8 + 1) * 10;
        world.damage_mobj(t, Some(id), Some(id), damage);
        return;
    }
    world.spawn_missile(id, t, MobjType::MT_BRUISERSHOT);
}

/// `A_SkullAttack`.
pub fn a_skull_attack(world: &mut World, id: MobjId) {
    let (tx, ty, tz, theight) = {
        let Some(a) = world.mobjs.get(id) else { return };
        let Some(d) = a.target else { return };
        let Some(dm) = world.mobjs.get(d) else { return };
        (dm.x, dm.y, dm.z, dm.height)
    };
    if let Some(m) = world.mobjs.get_mut(id) {
        m.flags |= MF_SKULLFLY;
    }
    world.play_sfx(world.mobjs.get(id).map(|m| m.info().attacksound).unwrap_or(Sfx::sfx_None));
    a_face_target(world, id);
    let angle = world.mobjs.get(id).map(|m| m.angle).unwrap_or(0);
    let a = angle >> ANGLETOFINESHIFT;
    let (sx, sy, sz) = {
        let m = world.mobjs.get(id).unwrap();
        (m.x, m.y, m.z)
    };
    if let Some(m) = world.mobjs.get_mut(id) {
        m.momx = fixed_mul(SKULLSPEED, finecosine_index(a));
        m.momy = fixed_mul(SKULLSPEED, finesine_index(a));
    }
    let mut dist = aprox_distance(tx - sx, ty - sy) / SKULLSPEED;
    if dist < 1 {
        dist = 1;
    }
    let momz = (tz + (theight >> 1) - sz) / dist;
    if let Some(m) = world.mobjs.get_mut(id) {
        m.momz = momz;
    }
}

/// `A_PosAttack`.
pub fn a_pos_attack(world: &mut World, id: MobjId) {
    if world.mobjs.get(id).and_then(|m| m.target).is_none() {
        return;
    }
    a_face_target(world, id);
    let angle = world.mobjs.get(id).map(|m| m.angle).unwrap_or(0);
    let vid = world.bump_validcount();
    let (slope, _) = aim_line_attack(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        id,
        angle,
        MISSILERANGE,
    );
    world.play_sfx(Sfx::sfx_pistol);
    let disp = ((world.rng.p.next() as i32 - world.rng.p.next() as i32) << 20) as u32;
    let angle = angle.wrapping_add(disp);
    let damage = (world.rng.p.next() as i32 % 5 + 1) * 3;
    line_attack_with(world, id, angle, slope, damage);
}

/// `A_SPosAttack`.
pub fn a_spos_attack(world: &mut World, id: MobjId) {
    if world.mobjs.get(id).and_then(|m| m.target).is_none() {
        return;
    }
    world.play_sfx(Sfx::sfx_shotgn);
    a_face_target(world, id);
    let bangle = world.mobjs.get(id).map(|m| m.angle).unwrap_or(0);
    let vid = world.bump_validcount();
    let (slope, _) = aim_line_attack(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        id,
        bangle,
        MISSILERANGE,
    );
    for _ in 0..3 {
        let disp = ((world.rng.p.next() as i32 - world.rng.p.next() as i32) << 20) as u32;
        let angle = bangle.wrapping_add(disp);
        let damage = (world.rng.p.next() as i32 % 5 + 1) * 3;
        line_attack_with(world, id, angle, slope, damage);
    }
}

/// `A_CPosAttack`.
pub fn a_cpos_attack(world: &mut World, id: MobjId) {
    if world.mobjs.get(id).and_then(|m| m.target).is_none() {
        return;
    }
    a_face_target(world, id);
    let angle = world.mobjs.get(id).map(|m| m.angle).unwrap_or(0);
    let vid = world.bump_validcount();
    let (slope, _) = aim_line_attack(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        id,
        angle,
        MISSILERANGE,
    );
    world.play_sfx(Sfx::sfx_shotgn);
    let disp = ((world.rng.p.next() as i32 - world.rng.p.next() as i32) << 20) as u32;
    let angle = angle.wrapping_add(disp);
    let damage = (world.rng.p.next() as i32 % 5 + 1) * 3;
    line_attack_with(world, id, angle, slope, damage);
}

/// `A_CPosRefire` / `A_SPidRefire` (refire chance differs).
fn cpos_or_spid_refire(world: &mut World, id: MobjId, chance: i32) {
    a_face_target(world, id);
    if (world.rng.p.next() as i32) < chance {
        return;
    }
    let bad = {
        let a = world.mobjs.get(id).unwrap();
        let t = a.target;
        let dead = t.map(|t| world.mobjs.get(t).map(|m| m.health <= 0).unwrap_or(true)).unwrap_or(true);
        dead || !t.map(|t| world.sight_between(id, t)).unwrap_or(false)
    };
    if bad {
        let seestate = world.mobjs.get(id).map(|m| m.info().seestate).unwrap_or(StateId::S_NULL);
        world.set_mobj_state(id, seestate);
    }
}

/// `A_CPosRefire`.
pub fn a_cpos_refire(world: &mut World, id: MobjId) {
    cpos_or_spid_refire(world, id, 40);
}

/// `A_SPidRefire`.
pub fn a_spid_refire(world: &mut World, id: MobjId) {
    cpos_or_spid_refire(world, id, 10);
}

/// `A_Metal`.
pub fn a_metal(world: &mut World, _id: MobjId) {
    world.play_sfx(Sfx::sfx_metal);
}

/// `A_BabyMetal`.
pub fn a_baby_metal(world: &mut World, _id: MobjId) {
    world.play_sfx(Sfx::sfx_metal);
}

/// `A_Hoof`.
pub fn a_hoof(world: &mut World, _id: MobjId) {
    world.play_sfx(Sfx::sfx_hoof);
}

/// `A_BspiAttack`.
pub fn a_bspi_attack(world: &mut World, id: MobjId) {
    let Some(t) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if let Some(missile) = world.spawn_missile(id, t, MobjType::MT_ARACHPLAZ) {
        world.play_sfx(world.mobjs.get(missile).map(|m| m.info().seesound).unwrap_or(Sfx::sfx_None));
    }
}

/// `A_CyberAttack`.
pub fn a_cyber_attack(world: &mut World, id: MobjId) {
    let Some(t) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if let Some(missile) = world.spawn_missile(id, t, MobjType::MT_ROCKET) {
        world.play_sfx(world.mobjs.get(missile).map(|m| m.info().seesound).unwrap_or(Sfx::sfx_None));
    }
}

/// `A_SkelWhoosh`.
pub fn a_skel_whoosh(world: &mut World, id: MobjId) {
    if world.mobjs.get(id).and_then(|m| m.target).is_some() {
        a_face_target(world, id);
        world.play_sfx(Sfx::sfx_skeswg);
    }
}

/// `A_SkelFist`.
pub fn a_skel_fist(world: &mut World, id: MobjId) {
    let Some(t) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if check_melee_range(world, id) {
        let damage = (world.rng.p.next() as i32 % 10 + 1) * 6;
        world.play_sfx(Sfx::sfx_skepch);
        world.damage_mobj(t, Some(id), Some(id), damage);
    }
}

/// `A_SkelMissile`.
pub fn a_skel_missile(world: &mut World, id: MobjId) {
    let target = world.mobjs.get(id).and_then(|m| m.target);
    let Some(t) = target else {
        return;
    };
    a_face_target(world, id);
    if let Some(m) = world.mobjs.get_mut(id) {
        m.z += 16 * FRAC_UNIT;
    }
    let missile = world.spawn_missile(id, t, MobjType::MT_TRACER);
    if let Some(m) = world.mobjs.get_mut(id) {
        m.z -= 16 * FRAC_UNIT;
    }
    if let Some(missile) = missile {
        if let Some(mm) = world.mobjs.get_mut(missile) {
            mm.x += mm.momx;
            mm.y += mm.momy;
            mm.tracer = Some(t);
        }
    }
}

/// `A_Tracer`.
pub fn a_tracer(world: &mut World, id: MobjId) {
    if world.leveltime & 3 != 0 {
        return;
    }

    let (x, y, z) = {
        let m = world.mobjs.get(id).unwrap();
        (m.x, m.y, m.z)
    };

    // `P_SpawnPuff` at the tracer's tip.
    let r1 = world.rng.p.next() as i32;
    let r2 = world.rng.p.next() as i32;
    let pz = z.wrapping_add((r1 - r2) << 10);
    let puff = crate::p_mobj::spawn_mobj(
        &mut world.mobjs, &mut world.thinkers, &world.level, &world.render,
        MobjType::MT_PUFF, x, y, pz,
    );
    if let Some(m) = world.mobjs.get_mut(puff) {
        m.momz = FRAC_UNIT;
        let mut tics = m.tics - (world.rng.p.next() as i32 & 3);
        if tics < 1 {
            tics = 1;
        }
        m.tics = tics;
    }

    // Smoke behind the tracer.
    let (mx, my) = {
        let m = world.mobjs.get(id).unwrap();
        (m.momx, m.momy)
    };
    let smoke = crate::p_mobj::spawn_mobj(
        &mut world.mobjs, &mut world.thinkers, &world.level, &world.render,
        MobjType::MT_SMOKE, x - mx, y - my, z,
    );
    if let Some(m) = world.mobjs.get_mut(smoke) {
        m.momz = FRAC_UNIT;
        let mut tics = m.tics - (world.rng.p.next() as i32 & 3);
        if tics < 1 {
            tics = 1;
        }
        m.tics = tics;
    }

    let (tx, ty, tz) = {
        let Some(d) = world.mobjs.get(id).and_then(|m| m.tracer) else { return };
        let Some(dm) = world.mobjs.get(d) else { return };
        if dm.health <= 0 {
            return;
        }
        (dm.x, dm.y, dm.z)
    };

    // Adjust the heading to the tracer target.
    let speed = world.mobjs.get(id).map(|m| m.info().speed).unwrap_or(0);
    let (ax, ay, az) = {
        let m = world.mobjs.get(id).unwrap();
        (m.x, m.y, m.z)
    };
    let exact = point_to_angle_from(ax, ay, tx, ty);
    let mut angle = world.mobjs.get(id).map(|m| m.angle).unwrap_or(0);
    if exact != angle {
        let diff = exact.wrapping_sub(angle);
        if diff > 0x8000_0000 {
            angle = angle.wrapping_sub(TRACEANGLE);
            if exact.wrapping_sub(angle) < 0x8000_0000 {
                angle = exact;
            }
        } else {
            angle = angle.wrapping_add(TRACEANGLE);
            if exact.wrapping_sub(angle) > 0x8000_0000 {
                angle = exact;
            }
        }
    }
    if let Some(m) = world.mobjs.get_mut(id) {
        m.angle = angle;
    }
    let a = angle >> ANGLETOFINESHIFT;
    let (momx, momy) = (fixed_mul(speed, finecosine_index(a)), fixed_mul(speed, finesine_index(a)));
    if let Some(m) = world.mobjs.get_mut(id) {
        m.momx = momx;
        m.momy = momy;
    }

    // Change the slope.
    let mut dist = aprox_distance(tx - ax, ty - ay) / speed.max(1);
    if dist < 1 {
        dist = 1;
    }
    let slope = (tz + 40 * FRAC_UNIT - az) / dist;
    if let Some(m) = world.mobjs.get_mut(id) {
        if slope < m.momz {
            m.momz -= FRAC_UNIT / 8;
        } else {
            m.momz += FRAC_UNIT / 8;
        }
    }
}

/// `A_Fall`.
pub fn a_fall(world: &mut World, id: MobjId) {
    if let Some(m) = world.mobjs.get_mut(id) {
        m.flags &= !MF_SOLID;
    }
}

/// `A_Pain`.
pub fn a_pain(world: &mut World, id: MobjId) {
    let painsound = world.mobjs.get(id).map(|m| m.info().painsound).unwrap_or(Sfx::sfx_None);
    if painsound != Sfx::sfx_None {
        world.play_sfx(painsound);
    }
}

/// `A_Scream`.
pub fn a_scream(world: &mut World, id: MobjId) {
    let (deathsound, ty) = {
        let a = world.mobjs.get(id).unwrap();
        (a.info().deathsound, a.ty)
    };
    let sound = match deathsound {
        Sfx::sfx_None => return,
        Sfx::sfx_podth1 | Sfx::sfx_podth2 | Sfx::sfx_podth3 => {
            let which = world.rng.p.next() as usize % 3;
            match which {
                0 => Sfx::sfx_podth1,
                1 => Sfx::sfx_podth2,
                _ => Sfx::sfx_podth3,
            }
        }
        Sfx::sfx_bgdth1 | Sfx::sfx_bgdth2 => {
            if (world.rng.p.next() & 1) != 0 {
                Sfx::sfx_bgdth2
            } else {
                Sfx::sfx_bgdth1
            }
        }
        other => other,
    };
    // Bosses play at full volume (stub has no volume, so this is a no-op).
    let _ = (ty, sound);
    world.play_sfx(sound);
}

/// `A_XScream`.
pub fn a_xscream(world: &mut World, id: MobjId) {
    let _ = id;
    world.play_sfx(Sfx::sfx_slop);
}

/// `A_Explode`.
pub fn a_explode(world: &mut World, id: MobjId) {
    let target = world.mobjs.get(id).and_then(|m| m.target);
    world.radius_attack(id, target, 128);
}

// ---------------------------------------------------------------------------
// DOOM II actions (`A_Vile*`, `A_Fat*`, `A_BossDeath`, pain elemental,
// `A_KeenDie`, the `A_Brain*` series)
// ---------------------------------------------------------------------------

/// `FATSPREAD`.
const FATSPREAD: u32 = ANG90 / 8;

/// `PIT_VileCheck`: find a corpse that could be raised. Returns the corpse.
fn vile_check(world: &mut World, vile: MobjId, viletryx: Fixed, viletryy: Fixed) -> Option<MobjId> {
    let (candidates, vile_radius) = {
        let vr = crate::info::MOBJINFO[MobjType::MT_VILE as usize].radius;
        let ids: alloc::vec::Vec<MobjId> = world.mobjs.iter_ids().collect();
        (ids, vr)
    };
    for thing in candidates {
        let Some(m) = world.mobjs.get(thing) else { continue };
        if m.flags & crate::p_mobj::MF_CORPSE == 0 {
            continue; // not a monster
        }
        if m.tics != -1 {
            continue; // not lying still yet
        }
        if m.info().raisestate == StateId::S_NULL {
            continue; // monster doesn't have a raise state
        }
        let maxdist = m.info().radius + vile_radius;
        if (m.x - viletryx).abs() > maxdist || (m.y - viletryy).abs() > maxdist {
            continue; // not actually touching
        }
        // zero momentum, grow for the fit check, shrink back.
        if let Some(m) = world.mobjs.get_mut(thing) {
            m.momx = 0;
            m.momy = 0;
            m.height <<= 2;
        }
        let vid = world.bump_validcount();
        let fits = crate::p_map::check_position(
            &world.level,
            &world.render,
            &world.mobjs,
            &mut world.line_vc,
            vid,
            thing,
            world.mobjs.get(thing).map(|m| m.x).unwrap_or(0),
            world.mobjs.get(thing).map(|m| m.y).unwrap_or(0),
        )
        .is_some();
        if let Some(m) = world.mobjs.get_mut(thing) {
            m.height >>= 2;
        }
        if !fits {
            continue; // doesn't fit here
        }
        let _ = vile;
        return Some(thing); // got one, so stop checking
    }
    None
}

/// `A_VileChase`: check for resurrecting a body.
pub fn a_vile_chase(world: &mut World, id: MobjId) {
    let (movedir, x, y, speed) = {
        let Some(m) = world.mobjs.get(id) else { return };
        (m.movedir, m.x, m.y, m.info().speed)
    };

    if movedir != -1 {
        // check for corpses to raise
        let viletryx = x + speed * XSPEED[movedir as usize];
        let viletryy = y + speed * YSPEED[movedir as usize];

        if let Some(corpse) = vile_check(world, id, viletryx, viletryy) {
            // got one!
            let temp = world.mobjs.get(id).and_then(|m| m.target);
            if let Some(m) = world.mobjs.get_mut(id) {
                m.target = Some(corpse);
            }
            a_face_target(world, id);
            if let Some(m) = world.mobjs.get_mut(id) {
                m.target = temp;
            }

            world.set_mobj_state(id, StateId::S_VILE_HEAL1);
            world.play_sfx(Sfx::sfx_slop);

            let (raisestate, flags, spawnhealth) = {
                let info = world.mobjs.get(corpse).unwrap().info();
                (info.raisestate, info.flags, info.spawnhealth)
            };
            world.set_mobj_state(corpse, raisestate);
            if let Some(m) = world.mobjs.get_mut(corpse) {
                m.height <<= 2;
                m.flags = flags;
                m.health = spawnhealth;
                m.target = None;
            }
            return;
        }
    }

    // Return to normal attack.
    a_chase(world, id);
}

/// `A_VileStart`.
pub fn a_vile_start(world: &mut World, id: MobjId) {
    let _ = id;
    world.play_sfx(Sfx::sfx_vilatk);
}

/// `A_Fire`: keep fire in front of the player unless out of sight.
pub fn a_fire(world: &mut World, id: MobjId) {
    let (Some(dest), target) = (
        world.mobjs.get(id).and_then(|m| m.tracer),
        world.mobjs.get(id).and_then(|m| m.target),
    ) else {
        return;
    };

    // don't move it if the vile lost sight
    let target = match target {
        Some(t) => t,
        None => id, // P_SubstNullMobj
    };
    if !world.sight_between(target, dest) {
        return;
    }

    let (dx, dy, dz) = {
        let d = world.mobjs.get(dest).unwrap();
        (d.x, d.y, d.z)
    };
    let an = (world.mobjs.get(dest).map(|m| m.angle).unwrap_or(0)) >> ANGLETOFINESHIFT;
    let nx = dx + fixed_mul(24 * FRAC_UNIT, crate::tables::finecosine_index(an));
    let ny = dy + fixed_mul(24 * FRAC_UNIT, crate::tables::finesine_index(an));

    world.mobjs.unset_thing_position(id);
    if let Some(m) = world.mobjs.get_mut(id) {
        m.x = nx;
        m.y = ny;
        m.z = dz;
    }
    let subsector = world.render.point_in_subsector(&world.level, nx, ny);
    let sector = world.level.subsectors[subsector].sector;
    world.mobjs.set_thing_position(id, &world.level, &world.render, sector);
}

/// `A_StartFire`.
pub fn a_start_fire(world: &mut World, id: MobjId) {
    world.play_sfx(Sfx::sfx_flamst);
    a_fire(world, id);
}

/// `A_FireCrackle`.
pub fn a_fire_crackle(world: &mut World, id: MobjId) {
    world.play_sfx(Sfx::sfx_flame);
    a_fire(world, id);
}

/// `A_VileTarget`: spawn the hellfire.
pub fn a_vile_target(world: &mut World, id: MobjId) {
    let Some(target) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);

    let (tx, ty, tz) = {
        let t = world.mobjs.get(target).unwrap();
        (t.x, t.y, t.z)
    };
    let fog = crate::p_mobj::spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        MobjType::MT_FIRE,
        tx,
        ty,
        tz,
    );
    if let Some(f) = world.mobjs.get_mut(fog) {
        f.target = Some(id);
        f.tracer = Some(target);
    }
    if let Some(m) = world.mobjs.get_mut(id) {
        m.tracer = Some(fog);
    }
    a_fire(world, fog);
}

/// `A_VileAttack`.
pub fn a_vile_attack(world: &mut World, id: MobjId) {
    let Some(target) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);

    if !world.sight_between(id, target) {
        return;
    }

    world.play_sfx(Sfx::sfx_barexp);
    world.damage_mobj(target, Some(id), Some(id), 20);
    let mass = world.mobjs.get(target).map(|m| m.info().mass).unwrap_or(100);
    if let Some(m) = world.mobjs.get_mut(target) {
        m.momz = 1000 * FRAC_UNIT / mass.max(1);
    }

    let an = (world.mobjs.get(id).map(|m| m.angle).unwrap_or(0)) >> ANGLETOFINESHIFT;
    let fire = world.mobjs.get(id).and_then(|m| m.tracer);
    let Some(fire) = fire else { return };

    // move the fire between the vile and the player
    let (tx, ty) = {
        let t = world.mobjs.get(target).unwrap();
        (t.x, t.y)
    };
    if let Some(f) = world.mobjs.get_mut(fire) {
        f.x = tx - fixed_mul(24 * FRAC_UNIT, crate::tables::finecosine_index(an));
        f.y = ty - fixed_mul(24 * FRAC_UNIT, crate::tables::finesine_index(an));
    }
    world.radius_attack(fire, Some(id), 70);
}

/// `A_FatRaise`.
pub fn a_fat_raise(world: &mut World, id: MobjId) {
    a_face_target(world, id);
    world.play_sfx(Sfx::sfx_manatk);
}

fn fat_shot(world: &mut World, id: MobjId, target: MobjId, adjust: u32) {
    let Some(mo) = world.spawn_missile(id, target, MobjType::MT_FATSHOT) else {
        return;
    };
    if let Some(m) = world.mobjs.get_mut(mo) {
        m.angle = m.angle.wrapping_add(adjust);
        let an = m.angle >> ANGLETOFINESHIFT;
        let speed = m.info().speed;
        m.momx = fixed_mul(speed, crate::tables::finecosine_index(an));
        m.momy = fixed_mul(speed, crate::tables::finesine_index(an));
    }
}

/// `A_FatAttack1`.
pub fn a_fat_attack1(world: &mut World, id: MobjId) {
    let Some(target) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if let Some(m) = world.mobjs.get_mut(id) {
        m.angle = m.angle.wrapping_add(FATSPREAD);
    }
    world.spawn_missile(id, target, MobjType::MT_FATSHOT);
    fat_shot(world, id, target, FATSPREAD);
}

/// `A_FatAttack2`.
pub fn a_fat_attack2(world: &mut World, id: MobjId) {
    let Some(target) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    if let Some(m) = world.mobjs.get_mut(id) {
        m.angle = m.angle.wrapping_sub(FATSPREAD);
    }
    world.spawn_missile(id, target, MobjType::MT_FATSHOT);
    fat_shot(world, id, target, FATSPREAD.wrapping_mul(2));
}

/// `A_FatAttack3`.
pub fn a_fat_attack3(world: &mut World, id: MobjId) {
    let Some(target) = world.mobjs.get(id).and_then(|m| m.target) else {
        return;
    };
    a_face_target(world, id);
    fat_shot(world, id, target, FATSPREAD / 2);
    fat_shot(world, id, target, 0u32.wrapping_sub(FATSPREAD / 2));
}

/// `A_PainShootSkull`: spawn a lost soul and launch it at the target.
fn pain_shoot_skull(world: &mut World, actor: MobjId, angle: u32) {
    // count total number of skulls currently on the level
    let count = world
        .mobjs
        .iter_ids()
        .filter(|&t| {
            world
                .mobjs
                .get(t)
                .map(|m| m.ty == MobjType::MT_SKULL)
                .unwrap_or(false)
        })
        .count();

    // if there are already 20 skulls on the level, don't spit another one
    if count > 20 {
        return;
    }

    let an = angle >> ANGLETOFINESHIFT;
    let prestep = 4 * FRAC_UNIT
        + 3 * (world.mobjs.get(actor).map(|m| m.info().radius).unwrap_or(0)
            + crate::info::MOBJINFO[MobjType::MT_SKULL as usize].radius)
            / 2;

    let (x, y, z) = {
        let Some(a) = world.mobjs.get(actor) else { return };
        (
            a.x + fixed_mul(prestep, crate::tables::finecosine_index(an)),
            a.y + fixed_mul(prestep, crate::tables::finesine_index(an)),
            a.z + 8 * FRAC_UNIT,
        )
    };

    let newmobj = crate::p_mobj::spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        MobjType::MT_SKULL,
        x,
        y,
        z,
    );

    // Check for movements.
    let (mx, my) = {
        let Some(m) = world.mobjs.get(newmobj) else { return };
        (m.x, m.y)
    };
    let vid = world.bump_validcount();
    if !crate::p_map::try_move(
        &world.level,
        &world.render,
        &mut world.mobjs,
        &mut world.line_vc,
        vid,
        newmobj,
        mx,
        my,
    ) {
        // kill it immediately
        world.damage_mobj(newmobj, Some(actor), Some(actor), 10000);
        return;
    }

    let target = world.mobjs.get(actor).and_then(|m| m.target);
    if let Some(m) = world.mobjs.get_mut(newmobj) {
        m.target = target;
    }
    a_skull_attack(world, newmobj);
}

/// `A_PainAttack`.
pub fn a_pain_attack(world: &mut World, id: MobjId) {
    if world.mobjs.get(id).and_then(|m| m.target).is_none() {
        return;
    }
    a_face_target(world, id);
    let angle = world.mobjs.get(id).map(|m| m.angle).unwrap_or(0);
    pain_shoot_skull(world, id, angle);
}

/// `A_PainDie`.
pub fn a_pain_die(world: &mut World, id: MobjId) {
    a_fall(world, id);
    let angle = world.mobjs.get(id).map(|m| m.angle).unwrap_or(0);
    pain_shoot_skull(world, id, angle.wrapping_add(ANG90));
    pain_shoot_skull(world, id, angle.wrapping_add(crate::tables::ANG180));
    pain_shoot_skull(world, id, angle.wrapping_add(crate::tables::ANG270));
}

/// `CheckBossEnd` (Ultimate Doom logic).
fn check_boss_end(world: &World, ty: MobjType) -> bool {
    match world.gameepisode {
        1 => world.gamemap == 8 && ty == MobjType::MT_BRUISER,
        2 => world.gamemap == 8 && ty == MobjType::MT_CYBORG,
        3 => world.gamemap == 8 && ty == MobjType::MT_SPIDER,
        4 => (world.gamemap == 6 && ty == MobjType::MT_CYBORG)
            || (world.gamemap == 8 && ty == MobjType::MT_SPIDER),
        _ => world.gamemap == 8,
    }
}

/// `A_BossDeath`: possibly trigger special effects if on first boss level.
pub fn a_boss_death(world: &mut World, id: MobjId) {
    let Some(ty) = world.mobjs.get(id).map(|m| m.ty) else {
        return;
    };

    if world.commercial {
        if world.gamemap != 7 {
            return;
        }
        if ty != MobjType::MT_FATSO && ty != MobjType::MT_BABY {
            return;
        }
    } else if !check_boss_end(world, ty) {
        return;
    }

    // make sure there is a player alive for victory
    let alive = (0..crate::doomdef::MAX_PLAYERS).any(|i| {
        world.player_in_game[i] && world.players[i].health > 0
    });
    if !alive {
        return; // no one left alive, so do not end game
    }

    // scan the remaining thinkers to see if all bosses are dead
    for t in world.mobjs.iter_ids() {
        if t == id {
            continue;
        }
        let other = world.mobjs.get(t).map(|m| (m.ty, m.health));
        if let Some((oty, health)) = other {
            if oty == ty && health > 0 {
                return; // other boss not dead
            }
        }
    }

    // victory!
    if world.commercial {
        if world.gamemap == 7 {
            if ty == MobjType::MT_FATSO {
                crate::p_floor::ev_do_floor_by_tag(world, 666, crate::p_floor::FloorType::LowerFloorToLowest);
                return;
            }
            if ty == MobjType::MT_BABY {
                crate::p_floor::ev_do_floor_by_tag(world, 667, crate::p_floor::FloorType::RaiseToTexture);
                return;
            }
        }
    } else {
        match world.gameepisode {
            1 => {
                crate::p_floor::ev_do_floor_by_tag(world, 666, crate::p_floor::FloorType::LowerFloorToLowest);
                return;
            }
            4 => match world.gamemap {
                6 => {
                    crate::p_doors::ev_do_door_by_tag(world, 666, crate::p_doors::VlDoorType::BlazeOpen);
                    return;
                }
                8 => {
                    crate::p_floor::ev_do_floor_by_tag(world, 666, crate::p_floor::FloorType::LowerFloorToLowest);
                    return;
                }
                _ => {}
            },
            _ => {}
        }
    }

    world.pending_exit = Some(false);
}

/// `A_KeenDie` — DOOM II special, map 32. Uses special tag 666.
pub fn a_keen_die(world: &mut World, id: MobjId) {
    a_fall(world, id);

    let ty = world.mobjs.get(id).map(|m| m.ty);

    // scan the remaining thinkers to see if all Keens are dead
    for t in world.mobjs.iter_ids() {
        if t == id {
            continue;
        }
        let other = world.mobjs.get(t).map(|m| (m.ty, m.health));
        if let Some((oty, health)) = other {
            if Some(oty) == ty && health > 0 {
                return; // other Keen not dead
            }
        }
    }

    crate::p_doors::ev_do_door_by_tag(world, 666, crate::p_doors::VlDoorType::Open);
}

/// `A_BrainAwake`.
pub fn a_brain_awake(world: &mut World, _id: MobjId) {
    // find all the target spots
    world.brain_targets.clear();
    world.brain_target_on = 0;
    for t in world.mobjs.iter_ids() {
        if world
            .mobjs
            .get(t)
            .map(|m| m.ty == MobjType::MT_BOSSTARGET)
            .unwrap_or(false)
        {
            world.brain_targets.push(t);
        }
    }
    world.play_sfx(Sfx::sfx_bossit);
}

/// `A_BrainPain`.
pub fn a_brain_pain(world: &mut World, _id: MobjId) {
    world.play_sfx(Sfx::sfx_bospn);
}

/// `A_BrainScream`.
pub fn a_brain_scream(world: &mut World, id: MobjId) {
    let Some((x0, y)) = world.mobjs.get(id).map(|m| (m.x, m.y)) else {
        return;
    };
    let mut x = x0 - 196 * FRAC_UNIT;
    while x < x0 + 320 * FRAC_UNIT {
        let z = 128 + (world.rng.p.next() as i32) * 2 * FRAC_UNIT;
        let th = crate::p_mobj::spawn_mobj(
            &mut world.mobjs,
            &mut world.thinkers,
            &world.level,
            &world.render,
            MobjType::MT_ROCKET,
            x,
            y - 320 * FRAC_UNIT,
            z,
        );
        if let Some(m) = world.mobjs.get_mut(th) {
            m.momz = (world.rng.p.next() as i32) * 512;
        }
        world.set_mobj_state(th, StateId::S_BRAINEXPLODE1);
        if let Some(m) = world.mobjs.get_mut(th) {
            m.tics -= world.rng.p.next() as i32 & 7;
            if m.tics < 1 {
                m.tics = 1;
            }
        }
        x += FRAC_UNIT * 8;
    }
    world.play_sfx(Sfx::sfx_bosdth);
}

/// `A_BrainExplode`.
pub fn a_brain_explode(world: &mut World, id: MobjId) {
    let (x0, y) = {
        let Some(m) = world.mobjs.get(id) else { return };
        (m.x, m.y)
    };
    let x = x0 + (world.rng.p.next() as i32 - world.rng.p.next() as i32) * 2048;
    let z = 128 + (world.rng.p.next() as i32) * 2 * FRAC_UNIT;
    let th = crate::p_mobj::spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        MobjType::MT_ROCKET,
        x,
        y,
        z,
    );
    if let Some(m) = world.mobjs.get_mut(th) {
        m.momz = (world.rng.p.next() as i32) * 512;
    }
    world.set_mobj_state(th, StateId::S_BRAINEXPLODE1);
    if let Some(m) = world.mobjs.get_mut(th) {
        m.tics -= world.rng.p.next() as i32 & 7;
        if m.tics < 1 {
            m.tics = 1;
        }
    }
}

/// `A_BrainDie`.
pub fn a_brain_die(world: &mut World, _id: MobjId) {
    world.pending_exit = Some(false);
}

/// `A_BrainSpit`.
pub fn a_brain_spit(world: &mut World, id: MobjId) {
    world.brain_easy_toggle = !world.brain_easy_toggle;
    if world.skill <= 1 && !world.brain_easy_toggle {
        return;
    }

    // shoot a cube at current target
    let Some(&targ) = world.brain_targets.get(world.brain_target_on) else {
        return;
    };
    world.brain_target_on = (world.brain_target_on + 1) % world.brain_targets.len().max(1);

    // spawn brain missile
    let Some(newmobj) = world.spawn_missile(id, targ, MobjType::MT_SPAWNSHOT) else {
        return;
    };
    let state_tics = world
        .mobjs
        .get(newmobj)
        .map(|m| crate::info::STATES[m.state as usize].tics)
        .unwrap_or(1);
    let ty_ = world.mobjs.get(targ).map(|t| t.y).unwrap_or(0);
    if let Some(m) = world.mobjs.get_mut(newmobj) {
        m.target = Some(targ);
        let momy = m.momy;
        let dy = ty_ - m.y;
        m.reactiontime = dy / momy / state_tics.max(1);
    }

    world.play_sfx(Sfx::sfx_bospit);
}

/// `A_SpawnSound`: travelling cube sound.
pub fn a_spawn_sound(world: &mut World, id: MobjId) {
    world.play_sfx(Sfx::sfx_boscub);
    a_spawn_fly(world, id);
}

/// `A_SpawnFly`: cube lands; spawn a monster.
pub fn a_spawn_fly(world: &mut World, id: MobjId) {
    let reactiontime = world.mobjs.get(id).map(|m| m.reactiontime).unwrap_or(1) - 1;
    if let Some(m) = world.mobjs.get_mut(id) {
        m.reactiontime = reactiontime;
    }
    if reactiontime != 0 {
        return; // still flying
    }

    let targ = world
        .mobjs
        .get(id)
        .and_then(|m| m.target)
        .unwrap_or(id); // P_SubstNullMobj

    let (tx, ty, tz) = {
        let Some(t) = world.mobjs.get(targ) else { return };
        (t.x, t.y, t.z)
    };

    // First spawn teleport fog.
    let fog = crate::p_mobj::spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        MobjType::MT_SPAWNFIRE,
        tx,
        ty,
        tz,
    );
    let _ = fog;
    world.play_sfx(Sfx::sfx_telept);

    // Randomly select monster to spawn. Probability distribution (kind of :),
    // decreasing likelihood.
    let r = world.rng.p.next() as i32;
    let ty_mon = if r < 50 {
        MobjType::MT_TROOP
    } else if r < 90 {
        MobjType::MT_SERGEANT
    } else if r < 120 {
        MobjType::MT_SHADOWS
    } else if r < 130 {
        MobjType::MT_PAIN
    } else if r < 160 {
        MobjType::MT_HEAD
    } else if r < 162 {
        MobjType::MT_VILE
    } else if r < 172 {
        MobjType::MT_UNDEAD
    } else if r < 192 {
        MobjType::MT_BABY
    } else if r < 222 {
        MobjType::MT_FATSO
    } else if r < 246 {
        MobjType::MT_KNIGHT
    } else {
        MobjType::MT_BRUISER
    };

    let newmobj = crate::p_mobj::spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        ty_mon,
        tx,
        ty,
        tz,
    );
    if look_for_players(world, newmobj, true) {
        let seestate = world.mobjs.get(newmobj).map(|m| m.info().seestate).unwrap_or(StateId::S_NULL);
        world.set_mobj_state(newmobj, seestate);
    }

    // telefrag anything in this spot
    crate::p_telept::teleport_move(world, newmobj, tx, ty);

    // remove self (i.e., cube).
    crate::p_mobj::remove_mobj(&mut world.mobjs, &mut world.thinkers, id);
}

/// `A_PlayerScream`.
pub fn a_player_scream(world: &mut World, id: MobjId) {
    // Default death sound.
    let health = world.mobjs.get(id).map(|m| m.health).unwrap_or(0);
    let sound = if world.commercial && health < -50 {
        // IF THE PLAYER DIES LESS THAN -50% WITHOUT GIBBING
        Sfx::sfx_pdiehi
    } else {
        Sfx::sfx_pldeth
    };
    world.play_sfx(sound);
}
