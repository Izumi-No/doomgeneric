//! Ceiling animation (lowering, crushing, raising), port of `p_ceilng.c`.

use crate::fixed::FRAC_UNIT;
use crate::p_spec::{
    find_highest_ceiling_surrounding, find_sector_from_line_tag, CEILSPEED, PlaneResult,
};
use crate::p_tick::{ThinkerId, ThinkerKind};
use crate::p_world::World;
use crate::sounds::Sfx;

/// `ceiling_e`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeilingType {
    RaiseToHighest,
    LowerAndCrush,
    SilentCrushAndRaise,
    FastCrushAndRaise,
    CrushAndRaise,
    LowerToFloor,
}

/// `ceiling_t`. Direction `0` means in-stasis.
#[derive(Clone, Debug)]
pub struct Ceiling {
    /// Link into [`crate::p_tick::Thinkers`].
    pub thinker: ThinkerId,
    pub sector: usize,
    pub crush: bool,
    pub bottomheight: i32,
    pub topheight: i32,
    pub speed: i32,
    /// `-1` down, `0` in stasis, `1` up.
    pub direction: i32,
    pub tag: i16,
    pub olddirection: i32,
    pub ty: CeilingType,
}

/// `T_MoveCeiling`.
pub fn t_move_ceiling(world: &mut World, ceiling: usize) {
    let Some(c) = world.specials.ceilings.get(ceiling) else {
        return;
    };
    let (sector, ty, direction, topheight, bottomheight, speed, crush) = (
        c.sector,
        c.ty,
        c.direction,
        c.topheight,
        c.bottomheight,
        c.speed,
        c.crush,
    );

    match direction {
        0 => {
            // IN STASIS
        }
        1 => {
            // UP
            let res = world.move_plane(sector, speed, topheight, false, true, 1);

            if world.leveltime & 7 == 0 && ty != CeilingType::SilentCrushAndRaise {
                world.play_sfx(Sfx::sfx_stnmov);
            }

            if res == PlaneResult::PastDest {
                match ty {
                    CeilingType::RaiseToHighest => remove_ceiling(world, ceiling),
                    CeilingType::SilentCrushAndRaise => {
                        world.play_sfx(Sfx::sfx_pstop);
                        set_ceiling(world, ceiling, |c| c.direction = -1);
                    }
                    CeilingType::FastCrushAndRaise | CeilingType::CrushAndRaise => {
                        set_ceiling(world, ceiling, |c| c.direction = -1);
                    }
                    _ => {}
                }
            }
        }
        -1 => {
            // DOWN
            let res = world.move_plane(sector, speed, bottomheight, crush, true, -1);

            if world.leveltime & 7 == 0 && ty != CeilingType::SilentCrushAndRaise {
                world.play_sfx(Sfx::sfx_stnmov);
            }

            if res == PlaneResult::PastDest {
                match ty {
                    CeilingType::SilentCrushAndRaise => {
                        world.play_sfx(Sfx::sfx_pstop);
                        set_ceiling(world, ceiling, |c| {
                            c.speed = CEILSPEED;
                            c.direction = 1;
                        });
                    }
                    CeilingType::CrushAndRaise => {
                        set_ceiling(world, ceiling, |c| {
                            c.speed = CEILSPEED;
                            c.direction = 1;
                        });
                    }
                    CeilingType::FastCrushAndRaise => {
                        set_ceiling(world, ceiling, |c| c.direction = 1);
                    }
                    CeilingType::LowerAndCrush | CeilingType::LowerToFloor => {
                        remove_ceiling(world, ceiling);
                    }
                    _ => {}
                }
            } else if res == PlaneResult::Crushed {
                match ty {
                    CeilingType::SilentCrushAndRaise
                    | CeilingType::CrushAndRaise
                    | CeilingType::LowerAndCrush => {
                        set_ceiling(world, ceiling, |c| c.speed = CEILSPEED / 8);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn set_ceiling(world: &mut World, ceiling: usize, f: impl FnOnce(&mut Ceiling)) {
    if let Some(c) = world.specials.ceilings.get_mut(ceiling) {
        f(c);
    }
}

/// `P_RemoveActiveCeiling`: clear `specialdata`, unlink the thinker and free
/// the active slot.
fn remove_ceiling(world: &mut World, ceiling: usize) {
    let Some(c) = world.specials.ceilings.get(ceiling) else {
        return;
    };
    let (thinker, sector) = (c.thinker, c.sector);
    world.level.sectors[sector].specialdata = None;
    world.thinkers.remove(thinker);
    world.specials.remove_active_ceiling(ceiling);
}

/// `EV_DoCeiling`: move a ceiling up/down and all around!
pub fn ev_do_ceiling(world: &mut World, line: usize, ty: CeilingType) -> bool {
    // Reactivate in-stasis ceilings...for certain types.
    match ty {
        CeilingType::FastCrushAndRaise
        | CeilingType::SilentCrushAndRaise
        | CeilingType::CrushAndRaise => {
            activate_in_stasis_ceiling(world, world.level.lines[line].tag);
        }
        _ => {}
    }

    let mut rtn = false;
    let mut secn = None;
    while let Some(sec) = find_sector_from_line_tag(&world.level, line, secn) {
        secn = Some(sec);
        if world.level.sectors[sec].specialdata.is_some() {
            continue;
        }

        // New ceiling thinker.
        rtn = true;
        let (floorheight, ceilingheight, tag) = {
            let s = &world.level.sectors[sec];
            (s.floorheight, s.ceilingheight, s.tag)
        };
        let mut c = Ceiling {
            thinker: 0,
            sector: sec,
            crush: false,
            bottomheight: 0,
            topheight: 0,
            speed: 0,
            direction: 0,
            tag,
            olddirection: 0,
            ty,
        };

        match ty {
            CeilingType::FastCrushAndRaise => {
                c.crush = true;
                c.topheight = ceilingheight;
                c.bottomheight = floorheight + 8 * FRAC_UNIT;
                c.direction = -1;
                c.speed = CEILSPEED * 2;
            }
            CeilingType::SilentCrushAndRaise | CeilingType::CrushAndRaise => {
                c.crush = true;
                c.topheight = ceilingheight;
                c.bottomheight = floorheight + 8 * FRAC_UNIT;
                c.direction = -1;
                c.speed = CEILSPEED;
            }
            CeilingType::LowerAndCrush | CeilingType::LowerToFloor => {
                c.bottomheight = floorheight;
                if ty != CeilingType::LowerToFloor {
                    c.bottomheight += 8 * FRAC_UNIT;
                }
                c.direction = -1;
                c.speed = CEILSPEED;
            }
            CeilingType::RaiseToHighest => {
                c.topheight = find_highest_ceiling_surrounding(&world.level, sec);
                c.direction = 1;
                c.speed = CEILSPEED;
            }
        }

        let idx = world.specials.ceilings.len();
        world.specials.ceilings.push(c);
        let thinker = world.thinkers.add(ThinkerKind::Ceiling(idx));
        world.specials.ceilings[idx].thinker = thinker;
        world.level.sectors[sec].specialdata = Some(thinker);
        world.specials.add_active_ceiling(idx);
    }
    rtn
}

/// `P_ActivateInStasisCeiling`: restart ceilings with `tag` that are in stasis.
pub fn activate_in_stasis_ceiling(world: &mut World, tag: i16) {
    let active: alloc::vec::Vec<usize> = world
        .specials
        .active_ceilings
        .iter()
        .copied()
        .flatten()
        .collect();
    for idx in active {
        let Some(c) = world.specials.ceilings.get_mut(idx) else {
            continue;
        };
        if c.tag == tag && c.direction == 0 {
            c.direction = c.olddirection;
        }
    }
}

/// `EV_CeilingCrushStop`: stop a ceiling from crushing!
pub fn ev_ceiling_crush_stop(world: &mut World, line: usize) -> bool {
    let tag = world.level.lines[line].tag;
    let mut rtn = false;
    for idx in world.specials.active_ceilings.iter().copied().flatten() {
        let Some(c) = world.specials.ceilings.get_mut(idx) else { continue };
        if c.tag == tag && c.direction != 0 {
            c.olddirection = c.direction;
            c.direction = 0; // in-stasis
            rtn = true;
        }
    }
    rtn
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::level::{Level, Line, Sector};
    use crate::r_main::Render;

    fn ceiling_level() -> World {
        let mut level = Level::default();
        for (floor, ceil) in [(0, 128), (0, 96)] {
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
    fn lower_to_floor_finishes_at_floor() {
        let mut w = ceiling_level();
        assert!(ev_do_ceiling(&mut w, 0, CeilingType::LowerToFloor));
        // 96 units at 1/tic.
        for _ in 0..95 {
            w.run_thinkers();
        }
        assert!(w.level.sectors[1].ceilingheight > 0);
        for _ in 0..3 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].ceilingheight, 0);
        assert_eq!(w.thinkers.iter().count(), 0);
        assert!(w.level.sectors[1].specialdata.is_none());
    }

    #[test]
    fn crush_and_raise_cycles() {
        let mut w = ceiling_level();
        assert!(ev_do_ceiling(&mut w, 0, CeilingType::CrushAndRaise));
        // Down to floor + 8, then the direction flips up.
        let mut flipped = false;
        for _ in 0..200 {
            w.run_thinkers();
            if w.specials.ceilings[0].direction == 1 {
                flipped = true;
                break;
            }
        }
        assert!(flipped);
        assert_eq!(w.level.sectors[1].ceilingheight, 8 << 16);
        // And back up to the original ceiling, where it flips down again.
        let mut cycled = false;
        for _ in 0..200 {
            w.run_thinkers();
            if w.specials.ceilings[0].direction == -1 {
                cycled = true;
                break;
            }
        }
        assert!(cycled);
        assert_eq!(w.level.sectors[1].ceilingheight, 96 << 16);
        // The thinker is never done.
        assert_eq!(w.thinkers.iter().count(), 1);
    }

    #[test]
    fn crush_stop_and_reactivate() {
        let mut w = ceiling_level();
        assert!(ev_do_ceiling(&mut w, 0, CeilingType::FastCrushAndRaise));
        for _ in 0..10 {
            w.run_thinkers();
        }
        assert!(ev_ceiling_crush_stop(&mut w, 0));
        let height = w.level.sectors[1].ceilingheight;
        assert_eq!(w.specials.ceilings[0].direction, 0);
        for _ in 0..10 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].ceilingheight, height);
        // Reactivating the same type wakes the in-stasis ceiling up; no new
        // thinker is created (specialdata is still set).
        assert!(!ev_do_ceiling(&mut w, 0, CeilingType::FastCrushAndRaise));
        assert_eq!(w.specials.ceilings[0].direction, -1);
    }
}
