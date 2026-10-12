//! Teleportation, port of `p_telept.c`.

use crate::fixed::Fixed;
use crate::info::MobjType;
use crate::p_map::check_position;
use crate::p_mobj::{MobjId, MF_MISSILE};
use crate::p_world::World;
use crate::sounds::Sfx;
use crate::tables::{finecosine_index, finesine_index, ANGLETOFINESHIFT};

/// `P_TeleportMove`: move `thing` instantly to `(x, y)` if it fits.
pub fn teleport_move(world: &mut World, thing: MobjId, x: Fixed, y: Fixed) -> bool {
    let vid = world.bump_validcount();
    let Some(check) = check_position(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        thing,
        x,
        y,
    ) else {
        return false;
    };

    // The move is ok: unlink, move, relink.
    world.mobjs.unset_thing_position(thing);
    if let Some(t) = world.mobjs.get_mut(thing) {
        t.floorz = check.floorz;
        t.ceilingz = check.ceilingz;
        t.x = x;
        t.y = y;
    }
    let subsector = world.render.point_in_subsector(&world.level, x, y);
    let sector = world.level.subsectors[subsector].sector;
    world.mobjs.set_thing_position(thing, &world.level, &world.render, sector);
    true
}

/// `EV_Teleport`: teleport `thing` to the `MT_TELEPORTMAN` in a sector
/// tagged like `line`. `side == 1` (back side) never teleports.
pub fn ev_teleport(world: &mut World, line: usize, side: usize, thing: MobjId) -> bool {
    // don't teleport missiles
    if world.mobjs.get(thing).map(|m| m.flags & MF_MISSILE != 0) == Some(true) {
        return false;
    }
    // Don't teleport if hit back of line, so you can get out of teleporter.
    if side == 1 {
        return false;
    }

    let tag = world.level.lines[line].tag;
    for i in 0..world.level.sectors.len() {
        if world.level.sectors[i].tag != tag {
            continue;
        }
        let dest_sector = i;

        let candidates: alloc::vec::Vec<MobjId> = world
            .mobjs
            .iter_ids()
            .filter(|&id| {
                world
                    .mobjs
                    .get(id)
                    .map(|m| m.ty == MobjType::MT_TELEPORTMAN && m.sector == dest_sector)
                    .unwrap_or(false)
            })
            .collect();

        // The vanilla loop returns from the function on the first teleportman
        // (success or failed fit), so only the first candidate matters.
        let man = candidates.first();
        if let Some(&man) = man {
            let (oldx, oldy, oldz) = {
                let Some(t) = world.mobjs.get(thing) else {
                    return false;
                };
                (t.x, t.y, t.z)
            };
            let (mx, my, mangle) = {
                let m = world.mobjs.get(man).unwrap();
                (m.x, m.y, m.angle)
            };

            if !teleport_move(world, thing, mx, my) {
                return false;
            }

            {
                if let Some(t) = world.mobjs.get_mut(thing) {
                    t.z = t.floorz;
                }
            }
            let pnum = world.mobjs.get(thing).and_then(|m| m.player);
            if let Some(pnum) = pnum {
                let (z, vh) = (
                    world.mobjs.get(thing).map(|m| m.z).unwrap_or(0),
                    world.players[pnum].viewheight,
                );
                world.players[pnum].viewz = z + vh;
            }

            // spawn teleport fog at source and destination
            crate::p_mobj::spawn_mobj(
                &mut world.mobjs,
                &mut world.thinkers,
                &world.level,
                &world.render,
                MobjType::MT_TFOG,
                oldx,
                oldy,
                oldz,
            );
            world.play_sfx(Sfx::sfx_telept);
            let an = (mangle >> ANGLETOFINESHIFT) & (crate::tables::FINEANGLES as u32 - 1);
            let z = world.mobjs.get(thing).map(|m| m.z).unwrap_or(0);
            crate::p_mobj::spawn_mobj(
                &mut world.mobjs,
                &mut world.thinkers,
                &world.level,
                &world.render,
                MobjType::MT_TFOG,
                mx + 20 * finecosine_index(an),
                my + 20 * finesine_index(an),
                z,
            );

            // don't move for a bit
            if pnum.is_some() {
                if let Some(t) = world.mobjs.get_mut(thing) {
                    t.reactiontime = 18;
                }
            }

            if let Some(t) = world.mobjs.get_mut(thing) {
                t.angle = mangle;
                t.momx = 0;
                t.momy = 0;
                t.momz = 0;
            }
            return true;
        }
    }
    false
}
