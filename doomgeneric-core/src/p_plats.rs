//! Plats (elevator platforms) raising/lowering, port of `p_plats.c`.

use crate::doomdef::TICRATE;
use crate::fixed::FRAC_UNIT;
use crate::p_spec::{
    find_highest_floor_surrounding, find_lowest_floor_surrounding, find_next_highest_floor,
    find_sector_from_line_tag, PlaneResult, PLATSPEED, PLATWAIT,
};
use crate::p_tick::{ThinkerId, ThinkerKind};
use crate::p_world::World;
use crate::sounds::Sfx;

/// `plattype_e`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatType {
    RaiseToNearestAndChange,
    RaiseAndChange,
    DownWaitUpStay,
    BlazeDWUS,
    PerpetualRaise,
}

/// `plat_e` status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatStatus {
    Up,
    Down,
    Waiting,
    InStasis,
}

/// `plat_t`.
#[derive(Clone, Debug)]
pub struct Plat {
    /// Link into [`crate::p_tick::Thinkers`].
    pub thinker: ThinkerId,
    pub sector: usize,
    pub speed: i32,
    pub low: i32,
    pub high: i32,
    pub wait: i32,
    pub count: i32,
    pub status: PlatStatus,
    pub oldstatus: PlatStatus,
    pub crush: bool,
    pub tag: i16,
    pub ty: PlatType,
}

/// `T_PlatRaise`: move a plat up and down.
pub fn t_plat_raise(world: &mut World, plat: usize) {
    let Some(p) = world.specials.plats.get(plat) else {
        return;
    };
    let (sector, speed, high, low, crush, ty, status, wait, count) = (
        p.sector,
        p.speed,
        p.high,
        p.low,
        p.crush,
        p.ty,
        p.status,
        p.wait,
        p.count,
    );

    match status {
        PlatStatus::Up => {
            let res = world.move_plane(sector, speed, high, crush, false, 1);

            if (ty == PlatType::RaiseAndChange || ty == PlatType::RaiseToNearestAndChange)
                && world.leveltime & 7 == 0
            {
                world.play_sfx(Sfx::sfx_stnmov);
            }

            if res == PlaneResult::Crushed && !crush {
                set_plat(world, plat, |p| {
                    p.count = wait;
                    p.status = PlatStatus::Down;
                });
                world.play_sfx(Sfx::sfx_pstart);
            } else if res == PlaneResult::PastDest {
                set_plat(world, plat, |p| {
                    p.count = wait;
                    p.status = PlatStatus::Waiting;
                });
                world.play_sfx(Sfx::sfx_pstop);

                match ty {
                    PlatType::BlazeDWUS
                    | PlatType::DownWaitUpStay
                    | PlatType::RaiseAndChange
                    | PlatType::RaiseToNearestAndChange => {
                        remove_plat(world, plat);
                    }
                    _ => {}
                }
            }
        }
        PlatStatus::Down => {
            let res = world.move_plane(sector, speed, low, false, false, -1);
            if res == PlaneResult::PastDest {
                set_plat(world, plat, |p| {
                    p.count = wait;
                    p.status = PlatStatus::Waiting;
                });
                world.play_sfx(Sfx::sfx_pstop);
            }
        }
        PlatStatus::Waiting => {
            let count = count - 1;
            set_plat(world, plat, |p| p.count = count);
            if count == 0 {
                let floorheight = world.level.sectors[sector].floorheight;
                let status = if floorheight == low {
                    PlatStatus::Up
                } else {
                    PlatStatus::Down
                };
                set_plat(world, plat, |p| p.status = status);
                world.play_sfx(Sfx::sfx_pstart);
            }
        }
        PlatStatus::InStasis => {}
    }
}

fn set_plat(world: &mut World, plat: usize, f: impl FnOnce(&mut Plat)) {
    if let Some(p) = world.specials.plats.get_mut(plat) {
        f(p);
    }
}

/// `P_RemoveActivePlat`: clear `specialdata`, unlink the thinker and free the
/// active slot.
fn remove_plat(world: &mut World, plat: usize) {
    let Some(p) = world.specials.plats.get(plat) else {
        return;
    };
    let (thinker, sector) = (p.thinker, p.sector);
    world.level.sectors[sector].specialdata = None;
    world.thinkers.remove(thinker);
    world.specials.remove_active_plat(plat);
}

fn add_plat(world: &mut World, sector: usize, p: Plat) -> usize {
    let idx = world.specials.plats.len();
    let mut p = p;
    p.sector = sector;
    world.specials.plats.push(p);
    let thinker = world.thinkers.add(ThinkerKind::Plat(idx));
    world.specials.plats[idx].thinker = thinker;
    world.level.sectors[sector].specialdata = Some(thinker);
    world.specials.add_active_plat(idx);
    idx
}

/// `EV_DoPlat`: "amount" is only used for SOME platforms.
pub fn ev_do_plat(world: &mut World, line: usize, ty: PlatType, amount: i32) -> bool {
    // Activate all <type> plats that are in_stasis.
    if ty == PlatType::PerpetualRaise {
        activate_in_stasis(world, world.level.lines[line].tag);
    }

    let mut rtn = false;
    let mut secn = None;
    while let Some(sec) = find_sector_from_line_tag(&world.level, line, secn) {
        secn = Some(sec);
        if world.level.sectors[sec].specialdata.is_some() {
            continue;
        }

        // Find lowest & highest floors around sector.
        rtn = true;
        let (floorheight, tag) = {
            let s = &world.level.sectors[sec];
            (s.floorheight, s.tag)
        };
        let mut p = Plat {
            thinker: 0,
            sector: 0,
            speed: 0,
            low: 0,
            high: 0,
            wait: 0,
            count: 0,
            status: PlatStatus::Up,
            oldstatus: PlatStatus::Up,
            crush: false,
            tag,
            ty,
        };

        match ty {
            PlatType::RaiseToNearestAndChange => {
                p.speed = PLATSPEED / 2;
                if let Some(front) = world.level.lines[line].frontsector {
                    let pic = world.level.sectors[front].floorpic;
                    world.level.sectors[sec].floorpic = pic;
                }
                p.high = find_next_highest_floor(&world.level, sec, floorheight);
                p.wait = 0;
                p.status = PlatStatus::Up;
                // NO MORE DAMAGE, IF APPLICABLE
                world.level.sectors[sec].special = 0;

                world.play_sfx(Sfx::sfx_stnmov);
            }
            PlatType::RaiseAndChange => {
                p.speed = PLATSPEED / 2;
                if let Some(front) = world.level.lines[line].frontsector {
                    let pic = world.level.sectors[front].floorpic;
                    world.level.sectors[sec].floorpic = pic;
                }
                p.high = floorheight + amount * FRAC_UNIT;
                p.wait = 0;
                p.status = PlatStatus::Up;

                world.play_sfx(Sfx::sfx_stnmov);
            }
            PlatType::DownWaitUpStay => {
                p.speed = PLATSPEED * 4;
                p.low = find_lowest_floor_surrounding(&world.level, sec);
                if p.low > floorheight {
                    p.low = floorheight;
                }
                p.high = floorheight;
                p.wait = (TICRATE as i32) * PLATWAIT;
                p.status = PlatStatus::Down;
                world.play_sfx(Sfx::sfx_pstart);
            }
            PlatType::BlazeDWUS => {
                p.speed = PLATSPEED * 8;
                p.low = find_lowest_floor_surrounding(&world.level, sec);
                if p.low > floorheight {
                    p.low = floorheight;
                }
                p.high = floorheight;
                p.wait = (TICRATE as i32) * PLATWAIT;
                p.status = PlatStatus::Down;
                world.play_sfx(Sfx::sfx_pstart);
            }
            PlatType::PerpetualRaise => {
                p.speed = PLATSPEED;
                p.low = find_lowest_floor_surrounding(&world.level, sec);
                if p.low > floorheight {
                    p.low = floorheight;
                }
                p.high = find_highest_floor_surrounding(&world.level, sec);
                if p.high < floorheight {
                    p.high = floorheight;
                }
                p.wait = (TICRATE as i32) * PLATWAIT;
                p.status = if world.rng.p.next() & 1 == 0 {
                    PlatStatus::Up
                } else {
                    PlatStatus::Down
                };

                world.play_sfx(Sfx::sfx_pstart);
            }
        }

        add_plat(world, sec, p);
    }
    rtn
}

/// `P_ActivateInStasis`.
pub fn activate_in_stasis(world: &mut World, tag: i16) {
    let active: alloc::vec::Vec<usize> = world
        .specials
        .active_plats
        .iter()
        .copied()
        .flatten()
        .collect();
    for idx in active {
        let Some(p) = world.specials.plats.get_mut(idx) else {
            continue;
        };
        if p.tag == tag && p.status == PlatStatus::InStasis {
            p.status = p.oldstatus;
        }
    }
}

/// `EV_StopPlat`.
pub fn ev_stop_plat(world: &mut World, line: usize) {
    let tag = world.level.lines[line].tag;
    let active: alloc::vec::Vec<usize> = world
        .specials
        .active_plats
        .iter()
        .copied()
        .flatten()
        .collect();
    for idx in active {
        let Some(p) = world.specials.plats.get_mut(idx) else {
            continue;
        };
        if p.status != PlatStatus::InStasis && p.tag == tag {
            p.oldstatus = p.status;
            p.status = PlatStatus::InStasis;
        }
    }
}

impl PlatType {
    /// Savegame restore: table index -> enum.
    pub fn from_index(i: i32) -> PlatType {
        const ALL: [PlatType; 5] = [
            PlatType::RaiseToNearestAndChange,
            PlatType::RaiseAndChange,
            PlatType::DownWaitUpStay,
            PlatType::BlazeDWUS,
            PlatType::PerpetualRaise,
        ];
        ALL[(i as usize).min(4)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::level::Level;
    use crate::level::{Line, Sector};
    use crate::r_main::Render;

    /// Room (0) at floor 0 beside a raised ledge (1, tag 7) at floor 64.
    fn plat_level() -> World {
        let mut level = Level::default();
        for (floor, ceil) in [(0, 256), (64, 256)] {
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
    fn down_wait_up_stay_finishes_low() {
        let mut w = plat_level();
        assert!(ev_do_plat(&mut w, 0, PlatType::DownWaitUpStay, 0));
        // Ledge lowers 64 units at 4/tic, then waits at the bottom.
        for _ in 0..20 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].floorheight, 0);
        assert_eq!(w.specials.plats[0].status, PlatStatus::Waiting);
        assert_eq!(w.thinkers.iter().count(), 1);
        // After the wait it rises again; only then is the thinker removed.
        for _ in 0..200 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].floorheight, 64 << 16);
        assert_eq!(w.thinkers.iter().count(), 0);
        assert!(w.level.sectors[1].specialdata.is_none());
    }

    #[test]
    fn perpetual_raise_oscillates() {
        let mut w = plat_level();
        assert!(ev_do_plat(&mut w, 0, PlatType::PerpetualRaise, 0));
        let p = &w.specials.plats[0];
        assert_eq!(p.low, 0);
        assert_eq!(p.high, 64 << 16);
        // 64 units at 1/tic plus 105 wait tics each way, over a few cycles.
        for _ in 0..600 {
            w.run_thinkers();
        }
        // The plat is still alive and somewhere in its cycle.
        assert_eq!(w.thinkers.iter().count(), 1);
        let h = w.level.sectors[1].floorheight;
        assert!(h == 0 || h == 64 << 16 || (h > 0 && h < 64 << 16));
    }

    #[test]
    fn stop_and_reactivate_perpetual() {
        let mut w = plat_level();
        assert!(ev_do_plat(&mut w, 0, PlatType::PerpetualRaise, 0));
        for _ in 0..10 {
            w.run_thinkers();
        }
        ev_stop_plat(&mut w, 0);
        assert_eq!(w.specials.plats[0].status, PlatStatus::InStasis);
        let height = w.level.sectors[1].floorheight;
        for _ in 0..10 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].floorheight, height);
        // A new perpetual plat wakes the stopped one instead of adding.
        assert!(!ev_do_plat(&mut w, 0, PlatType::PerpetualRaise, 0));
        assert_ne!(w.specials.plats[0].status, PlatStatus::InStasis);
        assert_eq!(w.specials.plats.len(), 1);
    }
}
