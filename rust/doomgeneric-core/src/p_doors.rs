//! Door animation code (opening/closing), port of `p_doors.c`.
//!
//! `T_VerticalDoor` ticks a [`VlDoor`] and `EV_DoDoor` spawns one per tagged
//! sector. `EV_DoLockedDoor`/`EV_VerticalDoor` (the manual-use and key-locked
//! variants) belong to the line-activation phase (`p_use`) and are deferred
//! together with player messages.

use crate::doomdef::TICRATE;
use crate::fixed::FRAC_UNIT;
use crate::p_spec::{
    find_lowest_ceiling_surrounding, VDOORSPEED, VDOORWAIT,
};
use crate::p_tick::{ThinkerId, ThinkerKind};
use crate::p_world::World;
use crate::sounds::Sfx;

/// `vldoor_e`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VlDoorType {
    Normal,
    Close30ThenOpen,
    Close,
    Open,
    RaiseIn5Mins,
    BlazeRaise,
    BlazeOpen,
    BlazeClose,
}

/// `vldoor_t`.
#[derive(Clone, Debug)]
pub struct VlDoor {
    /// Link into [`crate::p_tick::Thinkers`].
    pub thinker: ThinkerId,
    pub sector: usize,
    pub ty: VlDoorType,
    pub topheight: i32,
    pub speed: i32,
    /// `1` up, `0` waiting, `-1` down, `2` initial wait.
    pub direction: i32,
    /// Tics to wait at the top once fully open.
    pub topwait: i32,
    pub topcountdown: i32,
}

/// `T_VerticalDoor`.
pub fn t_vertical_door(world: &mut World, door: usize) {
    let Some(d) = world.specials.doors.get(door) else {
        return;
    };
    let (sector, ty, topheight, speed, direction, topwait, topcountdown) = (
        d.sector,
        d.ty,
        d.topheight,
        d.speed,
        d.direction,
        d.topwait,
        d.topcountdown,
    );

    match direction {
        0 => {
            // WAITING
            let topcountdown = topcountdown - 1;
            set_door(world, door, |d| d.topcountdown = topcountdown);
            if topcountdown == 0 {
                match ty {
                    VlDoorType::BlazeRaise => {
                        set_door(world, door, |d| d.direction = -1);
                        world.play_sfx(Sfx::sfx_bdcls);
                    }
                    VlDoorType::Normal => {
                        set_door(world, door, |d| d.direction = -1);
                        world.play_sfx(Sfx::sfx_dorcls);
                    }
                    VlDoorType::Close30ThenOpen => {
                        set_door(world, door, |d| d.direction = 1);
                        world.play_sfx(Sfx::sfx_doropn);
                    }
                    _ => {}
                }
            }
        }
        2 => {
            // INITIAL WAIT
            let topcountdown = topcountdown - 1;
            set_door(world, door, |d| d.topcountdown = topcountdown);
            if topcountdown == 0 && ty == VlDoorType::RaiseIn5Mins {
                set_door(world, door, |d| {
                    d.direction = 1;
                    d.ty = VlDoorType::Normal;
                });
                world.play_sfx(Sfx::sfx_doropn);
            }
        }
        -1 => {
            // DOWN
            let dest = world.level.sectors[sector].floorheight;
            let res = world.move_plane(sector, speed, dest, false, true, -1);
            if res == crate::p_spec::PlaneResult::PastDest {
                match ty {
                    VlDoorType::BlazeRaise | VlDoorType::BlazeClose => {
                        finish_door(world, door);
                        world.play_sfx(Sfx::sfx_bdcls);
                    }
                    VlDoorType::Normal | VlDoorType::Close => {
                        finish_door(world, door);
                    }
                    VlDoorType::Close30ThenOpen => {
                        set_door(world, door, |d| {
                            d.direction = 0;
                            d.topcountdown = (TICRATE as i32) * 30;
                        });
                    }
                    _ => {}
                }
            } else if res == crate::p_spec::PlaneResult::Crushed {
                match ty {
                    // DO NOT GO BACK UP!
                    VlDoorType::BlazeClose | VlDoorType::Close => {}
                    _ => {
                        set_door(world, door, |d| d.direction = 1);
                        world.play_sfx(Sfx::sfx_doropn);
                    }
                }
            }
        }
        1 => {
            // UP
            let res = world.move_plane(sector, speed, topheight, false, true, 1);
            if res == crate::p_spec::PlaneResult::PastDest {
                match ty {
                    VlDoorType::BlazeRaise | VlDoorType::Normal => {
                        set_door(world, door, |d| {
                            d.direction = 0;
                            d.topcountdown = topwait;
                        });
                    }
                    VlDoorType::Close30ThenOpen
                    | VlDoorType::BlazeOpen
                    | VlDoorType::Open => {
                        finish_door(world, door);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn set_door(world: &mut World, door: usize, f: impl FnOnce(&mut VlDoor)) {
    if let Some(d) = world.specials.doors.get_mut(door) {
        f(d);
    }
}

fn finish_door(world: &mut World, door: usize) {
    let Some(d) = world.specials.doors.get(door) else {
        return;
    };
    let (thinker, sector) = (d.thinker, d.sector);
    world.level.sectors[sector].specialdata = None;
    world.thinkers.remove(thinker);
}

/// Add a door thinker on `sector`; returns the door slot.
fn add_door(world: &mut World, sector: usize, d: VlDoor) -> usize {
    let idx = world.specials.doors.len();
    let mut d = d;
    d.sector = sector;
    world.specials.doors.push(d);
    let thinker = world.thinkers.add(ThinkerKind::Door(idx));
    world.specials.doors[idx].thinker = thinker;
    world.level.sectors[sector].specialdata = Some(thinker);
    idx
}

/// `EV_DoDoor`: move a door up/down on every sector tagged like `line`.
pub fn ev_do_door(world: &mut World, line: usize, ty: VlDoorType) -> bool {
    ev_do_door_by_tag(world, world.level.lines[line].tag, ty)
}

/// The `EV_DoDoor` body, driving sectors by tag directly (the boss-death
/// specials use a synthetic tag with no backing line, like vanilla's
/// `line_t junk`).
pub fn ev_do_door_by_tag(world: &mut World, tag: i16, ty: VlDoorType) -> bool {
    use crate::p_spec::find_sector_from_tag;
    let mut rtn = false;
    let mut secn = None;
    while let Some(sec) = find_sector_from_tag(&world.level, tag, secn) {
        secn = Some(sec);
        if world.level.sectors[sec].specialdata.is_some() {
            continue;
        }

        rtn = true;
        let idx = add_door(
            world,
            sec,
            VlDoor {
                thinker: 0,
                sector: 0,
                ty,
                topheight: 0,
                speed: VDOORSPEED,
                direction: 0,
                topwait: VDOORWAIT,
                topcountdown: 0,
            },
        );

        let sec_ref = &world.level.sectors[sec];
        let ceilingheight = sec_ref.ceilingheight;
        let mut topheight;
        let mut direction = 0;
        let mut speed = VDOORSPEED;
        match ty {
            VlDoorType::BlazeClose => {
                topheight = find_lowest_ceiling_surrounding(&world.level, sec);
                topheight -= 4 * FRAC_UNIT;
                direction = -1;
                speed = VDOORSPEED * 4;
                world.play_sfx(Sfx::sfx_bdcls);
            }
            VlDoorType::Close => {
                topheight = find_lowest_ceiling_surrounding(&world.level, sec);
                topheight -= 4 * FRAC_UNIT;
                direction = -1;
                world.play_sfx(Sfx::sfx_dorcls);
            }
            VlDoorType::Close30ThenOpen => {
                topheight = ceilingheight;
                direction = -1;
                world.play_sfx(Sfx::sfx_dorcls);
            }
            VlDoorType::BlazeRaise | VlDoorType::BlazeOpen => {
                direction = 1;
                topheight = find_lowest_ceiling_surrounding(&world.level, sec);
                topheight -= 4 * FRAC_UNIT;
                speed = VDOORSPEED * 4;
                if topheight != ceilingheight {
                    world.play_sfx(Sfx::sfx_bdopn);
                }
            }
            VlDoorType::Normal | VlDoorType::Open => {
                direction = 1;
                topheight = find_lowest_ceiling_surrounding(&world.level, sec);
                topheight -= 4 * FRAC_UNIT;
                if topheight != ceilingheight {
                    world.play_sfx(Sfx::sfx_doropn);
                }
            }
            _ => {
                topheight = 0;
            }
        }
        set_door(world, idx, |d| {
            d.topheight = topheight;
            d.direction = direction;
            d.speed = speed;
        });
    }
    rtn
}

/// `P_SpawnDoorCloseIn30`: spawn a door that closes after 30 seconds
/// (`P_SpawnSpecials` sector special 65).
pub fn spawn_door_close_in_30(world: &mut World, sector: usize) {
    world.level.sectors[sector].special = 0;
    add_door(
        world,
        sector,
        VlDoor {
            thinker: 0,
            sector: 0,
            ty: VlDoorType::Normal,
            topheight: 0,
            speed: VDOORSPEED,
            direction: 0,
            topwait: VDOORWAIT,
            topcountdown: 30 * TICRATE as i32,
        },
    );
}

/// `P_SpawnDoorRaiseIn5Mins`: spawn a door that opens after 5 minutes
/// (`P_SpawnSpecials` sector special 75).
pub fn spawn_door_raise_in_5_mins(world: &mut World, sector: usize) {
    world.level.sectors[sector].special = 0;
    let topheight = find_lowest_ceiling_surrounding(&world.level, sector) - 4 * FRAC_UNIT;
    add_door(
        world,
        sector,
        VlDoor {
            thinker: 0,
            sector: 0,
            ty: VlDoorType::RaiseIn5Mins,
            topheight,
            speed: VDOORSPEED,
            direction: 2,
            topwait: VDOORWAIT,
            topcountdown: 5 * 60 * TICRATE as i32,
        },
    );
}

/// Check the key locks for `line->special`; returns `false` (with the player
/// messaged) when the door stays shut. Shared by `EV_DoLockedDoor` and
/// `EV_VerticalDoor`.
fn check_locks(world: &mut World, line: usize, thing: usize) -> bool {
    use crate::doomdef::Card::*;

    let Some(pnum) = world.mobjs.get(thing).and_then(|m| m.player) else {
        return false;
    };
    let (special, has) = {
        let p = &world.players[pnum];
        let has = |c: crate::doomdef::Card| p.cards[c as usize];
        (
            world.level.lines[line].special,
            (
                has(BlueCard) || has(BlueSkull),
                has(YellowCard) || has(YellowSkull),
                has(RedCard) || has(RedSkull),
            ),
        )
    };

    let msg: Option<&'static str> = match special {
        // Blue Lock
        26 | 32 | 99 | 133 if !has.0 => Some("You need a blue key to open this door"),
        // Yellow Lock
        27 | 34 | 136 | 137 if !has.1 => Some("You need a yellow key to open this door"),
        // Red Lock
        28 | 33 | 134 | 135 if !has.2 => Some("You need a red key to open this door"),
        _ => None,
    };
    if let Some(m) = msg {
        world.players[pnum].message = Some(m);
        world.play_sfx(Sfx::sfx_oof);
        return false;
    }
    true
}

/// `EV_DoLockedDoor`: move a locked door up/down.
pub fn ev_do_locked_door(world: &mut World, line: usize, ty: VlDoorType, thing: usize) -> bool {
    if !check_locks(world, line, thing) {
        return false;
    }
    ev_do_door(world, line, ty)
}

/// `EV_VerticalDoor`: open a door manually (no tag value), from `p_doors.c`.
pub fn ev_vertical_door(world: &mut World, line: usize, thing: usize) {
    let special = world.level.lines[line].special;

    // Check for locks.
    if matches!(special, 26 | 27 | 28 | 32 | 33 | 34 | 99 | 133 | 134 | 135 | 136 | 137)
        && !check_locks(world, line, thing)
    {
        return;
    }

    // if the sector has an active thinker, use it
    // Only the back side of the line faces the using thing here.
    let sec = {
        let side = world.level.lines[line].sidenum[1];
        if side == -1 {
            return;
        }
        world.level.sides[side as usize].sector
    };

    if let Some(tid) = world.level.sectors[sec].specialdata {
        if matches!(special, 1 | 26 | 27 | 28 | 117) {
            // ONLY FOR "RAISE" DOORS, NOT "OPEN"s
            if let Some(idx) = world
                .specials
                .doors
                .iter()
                .position(|d| d.thinker == tid)
            {
                if world.specials.doors[idx].direction == -1 {
                    world.specials.doors[idx].direction = 1; // go back up
                } else if world.mobjs.get(thing).and_then(|m| m.player).is_some() {
                    // JDC: bad guys never close doors
                    world.specials.doors[idx].direction = -1; // start going down immediately
                }
            }
            return;
        }
    }

    // for proper sound
    match special {
        117 | 118 => world.play_sfx(Sfx::sfx_bdopn), // BLAZING DOOR RAISE/OPEN
        _ => world.play_sfx(Sfx::sfx_doropn),        // NORMAL/LOCKED DOOR SOUND
    }

    // new door thinker
    let (ty, speed) = match special {
        1 | 26 | 27 | 28 => (VlDoorType::Normal, VDOORSPEED),
        31..=34 => {
            world.level.lines[line].special = 0;
            (VlDoorType::Open, VDOORSPEED)
        }
        117 => (VlDoorType::BlazeRaise, VDOORSPEED * 4),
        118 => {
            world.level.lines[line].special = 0;
            (VlDoorType::BlazeOpen, VDOORSPEED * 4)
        }
        _ => (VlDoorType::Normal, VDOORSPEED),
    };

    // find the top and bottom of the movement range
    let topheight = find_lowest_ceiling_surrounding(&world.level, sec) - 4 * FRAC_UNIT;
    add_door(
        world,
        sec,
        VlDoor {
            thinker: 0,
            sector: 0,
            ty,
            topheight,
            speed,
            direction: 1,
            topwait: VDOORWAIT,
            topcountdown: 0,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::level::{Level, Line, Sector};
    use crate::r_main::Render;

    /// Sector 1 is a closet (floor 0, ceiling 24) next to a tall room (sector
    /// 0, ceiling 128); a tag-7 line activates the closet door.
    fn door_level() -> World {
        let mut level = Level::default();
        for (floor, ceil) in [(0, 128), (0, 24)] {
            level.sectors.push(Sector {
                floorheight: floor << 16,
                ceilingheight: ceil << 16,
                ..Sector::default()
            });
        }
        level.sectors[1].tag = 7;
        level.lines.push(Line {
            flags: crate::doomdef::ML_TWOSIDED,
            tag: 7,
            frontsector: Some(0),
            backsector: Some(1),
            ..Line::default()
        });
        level.sectors[0].lines = vec![0];
        level.sectors[1].lines = vec![0];
        World::new(level, Render::new())
    }

    #[test]
    fn normal_door_opens_waits_and_closes() {
        let mut w = door_level();
        assert!(ev_do_door(&mut w, 0, VlDoorType::Normal));
        // Slot 1 has a thinker; sector 1 specialdata is set.
        assert_eq!(w.thinkers.iter().count(), 1);
        assert!(w.level.sectors[1].specialdata.is_some());

        // 2 units per tic: 100<<16 of travel takes 50 tics.
        for _ in 0..49 {
            w.run_thinkers();
        }
        assert!(w.level.sectors[1].ceilingheight < 124 << 16);
        for _ in 0..2 {
            w.run_thinkers();
        }
        // Fully open (lowest surrounding ceiling 128 - 4) and waiting.
        assert_eq!(w.level.sectors[1].ceilingheight, 124 << 16);
        let door = &w.specials.doors[0];
        assert_eq!(door.direction, 0);
        assert_eq!(door.topcountdown, VDOORWAIT);

        // Wait out the countdown, then it starts closing.
        for _ in 0..VDOORWAIT {
            w.run_thinkers();
        }
        assert_eq!(w.specials.doors[0].direction, -1);

        // Once fully down (62 tics at 2 units) the thinker is unlinked.
        for _ in 0..70 {
            w.run_thinkers();
        }
        assert_eq!(w.thinkers.iter().count(), 0);
        assert!(w.level.sectors[1].specialdata.is_none());
        assert_eq!(w.level.sectors[1].ceilingheight, 0);
    }

    #[test]
    fn open_door_finishes_at_top() {
        let mut w = door_level();
        assert!(ev_do_door(&mut w, 0, VlDoorType::Open));
        for _ in 0..55 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].ceilingheight, 124 << 16);
        assert_eq!(w.thinkers.iter().count(), 0);
        assert!(w.level.sectors[1].specialdata.is_none());
    }

    #[test]
    fn busy_sector_is_skipped() {
        let mut w = door_level();
        assert!(ev_do_door(&mut w, 0, VlDoorType::Normal));
        // Second activation on the already-moving sector changes nothing.
        assert!(!ev_do_door(&mut w, 0, VlDoorType::Open));
        assert_eq!(w.specials.doors.len(), 1);
        assert_eq!(w.specials.doors[0].ty, VlDoorType::Normal);
    }
}
