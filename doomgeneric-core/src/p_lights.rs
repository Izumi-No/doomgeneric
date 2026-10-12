//! Sector lighting thinkers, port of `p_lights.c`.

use crate::p_spec::{find_min_surrounding_light, find_sector_from_line_tag, get_next_sector};
use crate::p_tick::{ThinkerId, ThinkerKind};
use crate::p_world::World;

/// `FASTDARK` / `SLOWDARK` strobe timings (`p_lights.c`).
pub const FASTDARK: i32 = 15;
pub const SLOWDARK: i32 = 32;
/// `STROBEBRIGHT`.
pub const STROBEBRIGHT: i32 = 16;
/// `GLOWSPEED`.
pub const GLOWSPEED: i32 = 8;

/// Which `p_lights.c` thinker a [`Light`] slot represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    /// `T_FireFlicker`.
    FireFlicker { minlight: i32, maxlight: i32 },
    /// `T_LightFlash`.
    Flash {
        minlight: i32,
        maxlight: i32,
        maxtime: i32,
        mintime: i32,
    },
    /// `T_StrobeFlash`.
    Strobe {
        minlight: i32,
        maxlight: i32,
        darktime: i32,
        brighttime: i32,
    },
    /// `T_Glow`.
    Glow { minlight: i32, maxlight: i32, direction: i32 },
}

/// One light thinker payload (`fireflicker_t`/`lightflash_t`/`strobe_t`/`glow_t`).
#[derive(Clone, Debug)]
pub struct Light {
    /// Link into [`crate::p_tick::Thinkers`].
    pub thinker: ThinkerId,
    pub sector: usize,
    pub count: i32,
    pub kind: LightKind,
}

fn add_light(world: &mut World, sector: usize, count: i32, kind: LightKind) -> usize {
    let idx = world.specials.lights.len();
    world.specials.lights.push(Light {
        thinker: 0,
        sector,
        count,
        kind,
    });
    let thinker = world.thinkers.add(ThinkerKind::Light(idx));
    world.specials.lights[idx].thinker = thinker;
    idx
}

/// `T_FireFlicker` / `T_LightFlash` / `T_StrobeFlash` / `T_Glow` dispatch.
pub fn t_light(world: &mut World, light: usize) {
    let Some(l) = world.specials.lights.get(light) else {
        return;
    };
    let (sector, count, kind) = (l.sector, l.count, l.kind);

    let new_count = match kind {
        LightKind::FireFlicker { minlight, maxlight } => {
            if count - 1 != 0 {
                count - 1
            } else {
                let amount = (world.rng.p.next() as i32 & 3) * 16;
                let s = &mut world.level.sectors[sector];
                if (s.lightlevel as i32) - amount < minlight {
                    s.lightlevel = minlight as i16;
                } else {
                    s.lightlevel = (maxlight - amount) as i16;
                }
                4
            }
        }
        LightKind::Flash {
            minlight,
            maxlight,
            maxtime,
            mintime,
        } => {
            if count - 1 != 0 {
                count - 1
            } else {
                let s = &mut world.level.sectors[sector];
                let r = world.rng.p.next() as i32;
                if s.lightlevel as i32 == maxlight {
                    s.lightlevel = minlight as i16;
                    (r & mintime) + 1
                } else {
                    s.lightlevel = maxlight as i16;
                    (r & maxtime) + 1
                }
            }
        }
        LightKind::Strobe {
            minlight,
            maxlight,
            darktime,
            brighttime,
        } => {
            if count - 1 != 0 {
                count - 1
            } else {
                let s = &mut world.level.sectors[sector];
                if s.lightlevel as i32 == minlight {
                    s.lightlevel = maxlight as i16;
                    brighttime
                } else {
                    s.lightlevel = minlight as i16;
                    darktime
                }
            }
        }
        LightKind::Glow {
            minlight,
            maxlight,
            direction,
        } => {
            let s = &mut world.level.sectors[sector];
            match direction {
                -1 => {
                    s.lightlevel -= GLOWSPEED as i16;
                    if s.lightlevel as i32 <= minlight {
                        s.lightlevel += GLOWSPEED as i16;
                        let l = &mut world.specials.lights[light];
                        if let LightKind::Glow { direction, .. } = &mut l.kind {
                            *direction = 1;
                        }
                    }
                }
                1 => {
                    s.lightlevel += GLOWSPEED as i16;
                    if s.lightlevel as i32 >= maxlight {
                        s.lightlevel -= GLOWSPEED as i16;
                        let l = &mut world.specials.lights[light];
                        if let LightKind::Glow { direction, .. } = &mut l.kind {
                            *direction = -1;
                        }
                    }
                }
                _ => {}
            }
            count
        }
    };
    if let Some(l) = world.specials.lights.get_mut(light) {
        l.count = new_count;
    }
}

/// `P_SpawnFireFlicker` (sector special 17).
pub fn spawn_fire_flicker(world: &mut World, sector: usize) {
    world.level.sectors[sector].special = 0;
    let maxlight = world.level.sectors[sector].lightlevel as i32;
    let minlight = find_min_surrounding_light(&world.level, sector, maxlight) + 16;
    add_light(
        world,
        sector,
        4,
        LightKind::FireFlicker { minlight, maxlight },
    );
}

/// `P_SpawnLightFlash` (sector special 1).
pub fn spawn_light_flash(world: &mut World, sector: usize) {
    world.level.sectors[sector].special = 0;
    let maxlight = world.level.sectors[sector].lightlevel as i32;
    let minlight = find_min_surrounding_light(&world.level, sector, maxlight);
    let count = (world.rng.p.next() as i32 & 64) + 1;
    add_light(
        world,
        sector,
        count,
        LightKind::Flash {
            minlight,
            maxlight,
            maxtime: 64,
            mintime: 7,
        },
    );
}

/// `P_SpawnStrobeFlash` (sector specials 2, 3, 4, 12, 13 and
/// `EV_StartLightStrobing`).
pub fn spawn_strobe_flash(world: &mut World, sector: usize, fast_or_slow: i32, in_sync: bool) {
    let maxlight = world.level.sectors[sector].lightlevel as i32;
    let mut minlight = find_min_surrounding_light(&world.level, sector, maxlight);
    if minlight == maxlight {
        minlight = 0;
    }

    // nothing special about it during gameplay
    world.level.sectors[sector].special = 0;

    let count = if in_sync {
        1
    } else {
        (world.rng.p.next() as i32 & 7) + 1
    };
    add_light(
        world,
        sector,
        count,
        LightKind::Strobe {
            minlight,
            maxlight,
            darktime: fast_or_slow,
            brighttime: STROBEBRIGHT,
        },
    );
}

/// `P_SpawnGlowingLight` (sector special 8).
pub fn spawn_glowing_light(world: &mut World, sector: usize) {
    let maxlight = world.level.sectors[sector].lightlevel as i32;
    let minlight = find_min_surrounding_light(&world.level, sector, maxlight);
    world.level.sectors[sector].special = 0;
    add_light(
        world,
        sector,
        0,
        LightKind::Glow {
            minlight,
            maxlight,
            direction: -1,
        },
    );
}

/// `EV_StartLightStrobing` (line special 17).
pub fn ev_start_light_strobing(world: &mut World, line: usize) {
    let mut secn = None;
    while let Some(sec) = find_sector_from_line_tag(&world.level, line, secn) {
        secn = Some(sec);
        if world.level.sectors[sec].specialdata.is_some() {
            continue;
        }
        spawn_strobe_flash(world, sec, SLOWDARK, false);
    }
}

/// `EV_TurnTagLightsOff` (line special 104).
pub fn ev_turn_tag_lights_off(world: &mut World, line: usize) {
    let tag = world.level.lines[line].tag;
    for sector in 0..world.level.sectors.len() {
        if world.level.sectors[sector].tag != tag {
            continue;
        }
        let mut min = world.level.sectors[sector].lightlevel as i32;
        for &li in &world.level.sectors[sector].lines.clone() {
            let Some(tsec) = get_next_sector(&world.level, li as usize, sector) else {
                continue;
            };
            let ll = world.level.sectors[tsec].lightlevel as i32;
            if ll < min {
                min = ll;
            }
        }
        world.level.sectors[sector].lightlevel = min as i16;
    }
}

/// `EV_LightTurnOn` (line specials 12, 13, 35, 79..81, 138, 139).
pub fn ev_light_turn_on(world: &mut World, line: usize, bright: i32) {
    let tag = world.level.lines[line].tag;
    let mut bright = bright;
    for sector in 0..world.level.sectors.len() {
        if world.level.sectors[sector].tag != tag {
            continue;
        }
        // bright = 0 means to search for highest light level
        // surrounding the sector.
        if bright == 0 {
            for &li in &world.level.sectors[sector].lines.clone() {
                let Some(temp) = get_next_sector(&world.level, li as usize, sector) else {
                    continue;
                };
                let ll = world.level.sectors[temp].lightlevel as i32;
                if ll > bright {
                    bright = ll;
                }
            }
        }
        world.level.sectors[sector].lightlevel = bright as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::level::{Level, Line, Sector};
    use crate::r_main::Render;

    fn light_level() -> World {
        let mut level = Level::default();
        for ll in [200, 100] {
            level.sectors.push(Sector {
                lightlevel: ll,
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
    fn light_turn_on_line_tag() {
        let mut w = light_level();
        ev_light_turn_on(&mut w, 0, 255);
        assert_eq!(w.level.sectors[1].lightlevel, 255);
        // EV_TurnTagLightsOff starts from the sector's own level and takes
        // the minimum neighbour: min(255, 200) = 200.
        ev_turn_tag_lights_off(&mut w, 0);
        assert_eq!(w.level.sectors[1].lightlevel, 200);
        // bright=0 picks the brightest surrounding sector.
        ev_light_turn_on(&mut w, 0, 0);
        assert_eq!(w.level.sectors[1].lightlevel, 200);
    }

    #[test]
    fn strobe_flashes_between_levels() {
        let mut w = light_level();
        spawn_strobe_flash(&mut w, 1, SLOWDARK, true);
        assert_eq!(w.specials.lights.len(), 1);
        // The strobe clears the sector special (none set here) and starts at
        // max light with count 1; one tick flips to minlight (0: no dimmer
        // neighbour means minlight=0 since minlight==maxlight).
        for _ in 0..2 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].lightlevel, 0);
        for _ in 0..SLOWDARK {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].lightlevel, 100);
    }

    #[test]
    fn glow_sweeps_and_reverses() {
        let mut w = light_level();
        spawn_glowing_light(&mut w, 1);
        let mut saw_max = false;
        for _ in 0..100 {
            w.run_thinkers();
            let ll = w.level.sectors[1].lightlevel as i32;
            saw_max |= ll == 100;
            assert!((0..=100).contains(&ll));
        }
        assert!(saw_max);
    }
}
