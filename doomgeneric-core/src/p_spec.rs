//! Sector-special helpers and shared state, port of the `p_spec.c` basics.
//!
//! Holds the tag/height search helpers every mover (`p_doors`, `p_floor`,
//! `p_ceilng`, `p_plats`) needs, plus [`Specials`], the arena that owns the
//! mover payloads addressed by [`crate::p_tick::ThinkerKind`] discriminants.
//!
//! The remaining `p_spec.c` machinery (line activation `P_CrossSpecialLine`,
//! `P_UseSpecials`, lights, scrolling walls, `EV_DoDonut`, `EV_LightTurnOn`)
//! belongs to the line-activation phase and is still deferred.

use alloc::vec::Vec;

use crate::level::Level;
use crate::p_world::World;
use crate::p_ceilng::Ceiling;
use crate::p_doors::VlDoor;
use crate::p_floor::FloorMove;
use crate::p_lights::Light;
use crate::p_plats::Plat;
use crate::p_switch::Button;
use crate::fixed::FRAC_UNIT;

/// `FLOORSPEED`.
pub const FLOORSPEED: i32 = FRAC_UNIT;
/// `CEILSPEED`.
pub const CEILSPEED: i32 = FRAC_UNIT;
/// `VDOORSPEED`.
pub const VDOORSPEED: i32 = FRAC_UNIT * 2;
/// `VDOORWAIT` (tics to wait at the top).
pub const VDOORWAIT: i32 = 150;
/// `PLATSPEED`.
pub const PLATSPEED: i32 = FRAC_UNIT;
/// `PLATWAIT` (seconds).
pub const PLATWAIT: i32 = 3;
/// `MAXPLATS` (`activeplats` slots).
pub const MAXPLATS: usize = 30;
/// `MAXCEILINGS` (`activeceilings` slots).
pub const MAXCEILINGS: usize = 30;

/// `result_e`: outcome of a `T_MovePlane` step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneResult {
    Ok,
    Crushed,
    PastDest,
}

/// `getNextSector`: the sector on the other side of a two-sided line.
pub fn get_next_sector(level: &Level, line: usize, sec: usize) -> Option<usize> {
    let l = &level.lines[line];
    if l.flags & crate::doomdef::ML_TWOSIDED == 0 {
        return None;
    }
    if l.frontsector == Some(sec) {
        l.backsector
    } else {
        l.frontsector
    }
}

/// `P_FindLowestFloorSurrounding`.
pub fn find_lowest_floor_surrounding(level: &Level, sec: usize) -> i32 {
    let mut floor = level.sectors[sec].floorheight;
    for &line in &level.sectors[sec].lines {
        let Some(other) = get_next_sector(level, line as usize, sec) else {
            continue;
        };
        if level.sectors[other].floorheight < floor {
            floor = level.sectors[other].floorheight;
        }
    }
    floor
}

/// `P_FindHighestFloorSurrounding`.
pub fn find_highest_floor_surrounding(level: &Level, sec: usize) -> i32 {
    let mut floor = -500 * FRAC_UNIT;
    for &line in &level.sectors[sec].lines {
        let Some(other) = get_next_sector(level, line as usize, sec) else {
            continue;
        };
        if level.sectors[other].floorheight > floor {
            floor = level.sectors[other].floorheight;
        }
    }
    floor
}

/// `P_FindNextHighestFloor` (with the vanilla 22-sector overflow emulation).
pub fn find_next_highest_floor(level: &Level, sec: usize, currentheight: i32) -> i32 {
    const MAX_ADJOINING_SECTORS: usize = 20;
    let mut height = currentheight;
    let mut heightlist = [0i32; MAX_ADJOINING_SECTORS + 2];
    let mut h = 0usize;

    for &line in &level.sectors[sec].lines {
        let Some(other) = get_next_sector(level, line as usize, sec) else {
            continue;
        };
        let fh = level.sectors[other].floorheight;
        if fh > height {
            // Emulation of the vanilla stack overflow.
            if h == MAX_ADJOINING_SECTORS + 1 {
                height = fh;
            } else if h == MAX_ADJOINING_SECTORS + 2 {
                // Fatal overflow in vanilla (I_Error); clamp here.
                break;
            }
            heightlist[h] = fh;
            h += 1;
        }
    }

    if h == 0 {
        return currentheight;
    }
    let mut min = heightlist[0];
    for &h_i in &heightlist[1..h] {
        if h_i < min {
            min = h_i;
        }
    }
    min
}

/// `P_FindLowestCeilingSurrounding`.
pub fn find_lowest_ceiling_surrounding(level: &Level, sec: usize) -> i32 {
    let mut height = i32::MAX;
    for &line in &level.sectors[sec].lines {
        let Some(other) = get_next_sector(level, line as usize, sec) else {
            continue;
        };
        if level.sectors[other].ceilingheight < height {
            height = level.sectors[other].ceilingheight;
        }
    }
    height
}

/// `P_FindHighestCeilingSurrounding`.
pub fn find_highest_ceiling_surrounding(level: &Level, sec: usize) -> i32 {
    let mut height = 0;
    for &line in &level.sectors[sec].lines {
        let Some(other) = get_next_sector(level, line as usize, sec) else {
            continue;
        };
        if level.sectors[other].ceilingheight > height {
            height = level.sectors[other].ceilingheight;
        }
    }
    height
}

/// `P_FindSectorFromLineTag`: next sector with the line's tag after `start`.
pub fn find_sector_from_line_tag(level: &Level, line: usize, start: Option<usize>) -> Option<usize> {
    let tag = level.lines[line].tag;
    (start.map_or(0, |s| s + 1)..level.sectors.len())
        .find(|&i| level.sectors[i].tag == tag)
}

/// Scan sectors by tag directly (used by the by-tag EV helpers).
pub fn find_sector_from_tag(level: &Level, tag: i16, start: Option<usize>) -> Option<usize> {
    (start.map_or(0, |s| s + 1)..level.sectors.len())
        .find(|&i| level.sectors[i].tag == tag)
}

/// `P_FindMinSurroundingLight`.
pub fn find_min_surrounding_light(level: &Level, sec: usize, max: i32) -> i32 {
    let mut min = max;
    for &line in &level.sectors[sec].lines {
        let Some(other) = get_next_sector(level, line as usize, sec) else {
            continue;
        };
        if (level.sectors[other].lightlevel as i32) < min {
            min = level.sectors[other].lightlevel as i32;
        }
    }
    min
}

/// The mover arenas (`vldoor_t`/`ceiling_t`/`floormove_t`/`plat_t`) plus the
/// `activeceilings`/`activeplats` tables used for in-stasis bookkeeping.
///
/// Payload slots are never removed; a finished mover only unlinks its thinker
/// and clears `sector.specialdata`, mirroring the C list nodes that stay
/// allocated until the level ends.
#[derive(Default)]
pub struct Specials {
    pub doors: Vec<VlDoor>,
    pub ceilings: Vec<Ceiling>,
    pub floors: Vec<FloorMove>,
    pub plats: Vec<Plat>,
    pub lights: Vec<Light>,
    pub buttons: Vec<Button>,
    pub active_ceilings: Vec<Option<usize>>,
    pub active_plats: Vec<Option<usize>>,
}

impl Specials {
    pub fn new() -> Self {
        Self::default()
    }

    /// `P_AddActiveCeiling`: first free slot, up to `MAXCEILINGS`
    /// (vanilla silently drops beyond that).
    pub fn add_active_ceiling(&mut self, ceiling: usize) {
        if let Some(slot) = self.active_ceilings.iter_mut().find(|s| s.is_none()) {
            *slot = Some(ceiling);
        } else if self.active_ceilings.len() < MAXCEILINGS {
            self.active_ceilings.push(Some(ceiling));
        }
    }

    /// `P_RemoveActiveCeiling` slot bookkeeping only (the caller unlinks the
    /// thinker and clears `specialdata`).
    pub fn remove_active_ceiling(&mut self, ceiling: usize) {
        for slot in self.active_ceilings.iter_mut() {
            if *slot == Some(ceiling) {
                *slot = None;
                break;
            }
        }
    }

    /// `P_AddActivePlat`: first free slot, up to `MAXPLATS`
    /// (vanilla `I_Error`s beyond that; the port drops instead).
    pub fn add_active_plat(&mut self, plat: usize) {
        if let Some(slot) = self.active_plats.iter_mut().find(|s| s.is_none()) {
            *slot = Some(plat);
        } else if self.active_plats.len() < MAXPLATS {
            self.active_plats.push(Some(plat));
        }
    }

    /// `P_RemoveActivePlat` slot bookkeeping only.
    pub fn remove_active_plat(&mut self, plat: usize) {
        for slot in self.active_plats.iter_mut() {
            if *slot == Some(plat) {
                *slot = None;
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Events: line activation (`p_spec.c` "EVENTS" section)
// ---------------------------------------------------------------------------

use crate::p_doors::VlDoorType;
use crate::p_floor::{ev_build_stairs, ev_do_floor, FloorType, StairType};
use crate::p_ceilng::{ev_ceiling_crush_stop, ev_do_ceiling, CeilingType};
use crate::p_doors::ev_do_door;
use crate::p_lights::{
    ev_light_turn_on, ev_start_light_strobing, ev_turn_tag_lights_off,
};
use crate::p_mobj::MobjId;
use crate::p_plats::{ev_do_plat, ev_stop_plat, PlatType};
use crate::p_telept::ev_teleport;

/// `P_CrossSpecialLine` — a thing origin is about to cross a line with a
/// non-zero special.
pub fn cross_special_line(world: &mut World, line: usize, side: usize, thing: MobjId) {
    let special = world.level.lines[line].special;
    let is_player = world.mobjs.get(thing).and_then(|m| m.player).is_some();

    // Triggers that other things can activate.
    if !is_player {
        // Things that should NOT trigger specials...
        if let Some(ty) = world.mobjs.get(thing).map(|m| m.ty) {
            use crate::info::MobjType::*;
            if matches!(
                ty,
                MT_ROCKET | MT_PLASMA | MT_BFG | MT_TROOPSHOT | MT_HEADSHOT | MT_BRUISERSHOT
            ) {
                return;
            }
        }
        let ok = matches!(special, 39 | 97 | 125 | 126 | 4 | 10 | 88);
        if !ok {
            return;
        }
    }

    let once = |special: i16| -> bool {
        matches!(
            special,
            2..=41 | 53 | 54 | 56..=59 | 100 | 104 | 108..=110 | 119 | 121 | 125 | 130 | 141
        )
    };

    // Dispatch. `once` specials are zeroed after the first trigger.
    macro_rules! fire {
        ($e:expr) => {{
            $e;
            if once(special) {
                world.level.lines[line].special = 0;
            }
        }};
    }

    match special {
        // TRIGGERS
        2 => fire!(ev_do_door(world, line, VlDoorType::Open)),
        3 => fire!(ev_do_door(world, line, VlDoorType::Close)),
        4 => fire!(ev_do_door(world, line, VlDoorType::Normal)),
        5 => fire!(ev_do_floor(world, line, FloorType::RaiseFloor)),
        6 => fire!(
            ev_do_ceiling(world, line, CeilingType::FastCrushAndRaise)
        ),
        8 => fire!(ev_build_stairs(world, line, StairType::Build8)),
        10 => fire!(ev_do_plat(world, line, PlatType::DownWaitUpStay, 0)),
        12 => fire!(ev_light_turn_on(world, line, 0)),
        13 => fire!(ev_light_turn_on(world, line, 255)),
        16 => fire!(
            ev_do_door(world, line, VlDoorType::Close30ThenOpen)
        ),
        17 => fire!(ev_start_light_strobing(world, line)),
        19 => fire!(ev_do_floor(world, line, FloorType::LowerFloor)),
        22 => fire!(
            ev_do_plat(world, line, PlatType::RaiseToNearestAndChange, 0)
        ),
        25 => fire!(
            ev_do_ceiling(world, line, CeilingType::CrushAndRaise)
        ),
        30 => fire!(ev_do_floor(world, line, FloorType::RaiseToTexture)),
        35 => fire!(ev_light_turn_on(world, line, 35)),
        36 => fire!(ev_do_floor(world, line, FloorType::TurboLower)),
        37 => fire!(ev_do_floor(world, line, FloorType::LowerAndChange)),
        38 => fire!(
            ev_do_floor(world, line, FloorType::LowerFloorToLowest)
        ),
        39 => {
            ev_teleport(world, line, side, thing);
            world.level.lines[line].special = 0;
        }
        40 => {
            ev_do_ceiling(world, line, CeilingType::RaiseToHighest);
            ev_do_floor(world, line, FloorType::LowerFloorToLowest);
            world.level.lines[line].special = 0;
        }
        44 => fire!(
            ev_do_ceiling(world, line, CeilingType::LowerAndCrush)
        ),
        52 => world.pending_exit = Some(false), // EXIT!
        53 => fire!(ev_do_plat(world, line, PlatType::PerpetualRaise, 0)),
        54 => {
            ev_stop_plat(world, line);
            world.level.lines[line].special = 0;
        }
        56 => fire!(ev_do_floor(world, line, FloorType::RaiseFloorCrush)),
        57 => {
            ev_ceiling_crush_stop(world, line);
            world.level.lines[line].special = 0;
        }
        58 => fire!(ev_do_floor(world, line, FloorType::RaiseFloor24)),
        59 => fire!(
            ev_do_floor(world, line, FloorType::RaiseFloor24AndChange)
        ),
        104 => fire!(ev_turn_tag_lights_off(world, line)),
        108 => fire!(ev_do_door(world, line, VlDoorType::BlazeRaise)),
        109 => fire!(ev_do_door(world, line, VlDoorType::BlazeOpen)),
        100 => fire!(ev_build_stairs(world, line, StairType::Turbo16)),
        110 => fire!(ev_do_door(world, line, VlDoorType::BlazeClose)),
        119 => fire!(
            ev_do_floor(world, line, FloorType::RaiseFloorToNearest)
        ),
        121 => fire!(ev_do_plat(world, line, PlatType::BlazeDWUS, 0)),
        124 => world.pending_exit = Some(true), // Secret EXIT
        125 => {
            // TELEPORT MonsterONLY
            if !is_player {
                ev_teleport(world, line, side, thing);
                world.level.lines[line].special = 0;
            }
        }
        130 => fire!(ev_do_floor(world, line, FloorType::RaiseFloorTurbo)),
        141 => fire!(
            ev_do_ceiling(world, line, CeilingType::SilentCrushAndRaise)
        ),

        // RETRIGGERS
        72 => {
            ev_do_ceiling(world, line, CeilingType::LowerAndCrush);
        }
        73 => {
            ev_do_ceiling(world, line, CeilingType::CrushAndRaise);
        }
        74 => {
            ev_ceiling_crush_stop(world, line);
        }
        75 => {
            ev_do_door(world, line, VlDoorType::Close);
        }
        76 => {
            ev_do_door(world, line, VlDoorType::Close30ThenOpen);
        }
        77 => {
            ev_do_ceiling(world, line, CeilingType::FastCrushAndRaise);
        }
        79 => {
            ev_light_turn_on(world, line, 35);
        }
        80 => {
            ev_light_turn_on(world, line, 0);
        }
        81 => {
            ev_light_turn_on(world, line, 255);
        }
        82 => {
            ev_do_floor(world, line, FloorType::LowerFloorToLowest);
        }
        83 => {
            ev_do_floor(world, line, FloorType::LowerFloor);
        }
        84 => {
            ev_do_floor(world, line, FloorType::LowerAndChange);
        }
        86 => {
            ev_do_door(world, line, VlDoorType::Open);
        }
        87 => {
            ev_do_plat(world, line, PlatType::PerpetualRaise, 0);
        }
        88 => {
            ev_do_plat(world, line, PlatType::DownWaitUpStay, 0);
        }
        89 => {
            ev_stop_plat(world, line);
        }
        90 => {
            ev_do_door(world, line, VlDoorType::Normal);
        }
        91 => {
            ev_do_floor(world, line, FloorType::RaiseFloor);
        }
        92 => {
            ev_do_floor(world, line, FloorType::RaiseFloor24);
        }
        93 => {
            ev_do_floor(world, line, FloorType::RaiseFloor24AndChange);
        }
        94 => {
            ev_do_floor(world, line, FloorType::RaiseFloorCrush);
        }
        95 => {
            ev_do_plat(world, line, PlatType::RaiseToNearestAndChange, 0);
        }
        96 => {
            ev_do_floor(world, line, FloorType::RaiseToTexture);
        }
        97 => {
            ev_teleport(world, line, side, thing);
        }
        98 => {
            ev_do_floor(world, line, FloorType::TurboLower);
        }
        105 => {
            ev_do_door(world, line, VlDoorType::BlazeRaise);
        }
        106 => {
            ev_do_door(world, line, VlDoorType::BlazeOpen);
        }
        107 => {
            ev_do_door(world, line, VlDoorType::BlazeClose);
        }
        120 => {
            ev_do_plat(world, line, PlatType::BlazeDWUS, 0);
        }
        126 => {
            // TELEPORT MonsterONLY
            if !is_player {
                ev_teleport(world, line, side, thing);
            }
        }
        128 => {
            ev_do_floor(world, line, FloorType::RaiseFloorToNearest);
        }
        129 => {
            ev_do_floor(world, line, FloorType::RaiseFloorTurbo);
        }
        _ => {}
    }
}

/// `P_ShootSpecialLine` — impact specials.
pub fn shoot_special_line(world: &mut World, line: usize, thing: MobjId) {
    let special = world.level.lines[line].special;
    let is_player = world.mobjs.get(thing).and_then(|m| m.player).is_some();

    // Impacts that other things can activate.
    if !is_player && special != 46 {
        // OPEN DOOR IMPACT
        return;
    }

    match special {
        24 => {
            // RAISE FLOOR
            ev_do_floor(world, line, FloorType::RaiseFloor);
            crate::p_switch::change_switch_texture(world, line, false);
        }
        46 => {
            // OPEN DOOR
            ev_do_door(world, line, VlDoorType::Open);
            crate::p_switch::change_switch_texture(world, line, true);
        }
        47 => {
            // RAISE FLOOR NEAR AND CHANGE
            ev_do_plat(world, line, PlatType::RaiseToNearestAndChange, 0);
            crate::p_switch::change_switch_texture(world, line, false);
        }
        _ => {}
    }
}

/// `EV_DoDonut` — the slime donut special (raise the ring, lower the pillar).
pub fn ev_do_donut(world: &mut World, line: usize) -> bool {
    let mut rtn = false;
    let mut secn = None;
    while let Some(sec1) = find_sector_from_line_tag(&world.level, line, secn) {
        secn = Some(sec1);

        // ALREADY MOVING?  IF SO, KEEP GOING...
        if world.level.sectors[sec1].specialdata.is_some() {
            continue;
        }
        rtn = true;

        let Some(first_line) = world.level.sectors[sec1].lines.first().copied() else {
            break;
        };
        let Some(sec2) = get_next_sector(&world.level, first_line as usize, sec1) else {
            // Vanilla reads through one-sided linedefs here; skip instead of
            // reading random memory.
            break;
        };

        for &li in &world.level.sectors[sec2].lines.clone() {
            let Some(sec3) = world.level.lines[li as usize].backsector else {
                continue;
            };
            if sec3 == sec1 {
                continue;
            }
            let (s3_floorheight, s3_floorpic) = {
                let s = &world.level.sectors[sec3];
                (s.floorheight, s.floorpic)
            };

            // Spawn rising slime carrying the target floor's texture.
            crate::p_floor::add_floor(
                world,
                sec2,
                FloorMove {
                    thinker: 0,
                    ty: FloorType::DonutRaise,
                    crush: false,
                    sector: 0,
                    direction: 1,
                    newspecial: 0,
                    texture: s3_floorpic,
                    floordestheight: s3_floorheight,
                    speed: FLOORSPEED / 2,
                },
            );

            // Spawn lowering donut-hole.
            crate::p_floor::add_floor(
                world,
                sec1,
                FloorMove {
                    thinker: 0,
                    ty: FloorType::LowerFloor,
                    crush: false,
                    sector: 0,
                    direction: -1,
                    newspecial: 0,
                    texture: s3_floorpic,
                    floordestheight: s3_floorheight,
                    speed: FLOORSPEED / 2,
                },
            );
            break;
        }
    }
    rtn
}

/// `P_PlayerInSpecialSector` — called every tic the player origin is in a
/// special sector.
pub fn player_in_special_sector(world: &mut World, pnum: usize) {
    let Some(mo) = world.players[pnum].mo else { return };
    let Some(sector) = world.mobjs.get(mo).map(|m| m.sector) else { return };

    // Falling, not all the way down yet?
    let (z, floorheight) = (
        world.mobjs.get(mo).map(|m| m.z).unwrap_or(0),
        world.level.sectors[sector].floorheight,
    );
    if z != floorheight {
        return;
    }

    let special = world.level.sectors[sector].special;
    let has_ironfeet = world.players[pnum].powers[crate::doomdef::PowerType::IronFeet as usize] != 0;
    match special {
        5 => {
            // HELLSLIME DAMAGE
            if !has_ironfeet && world.leveltime & 0x1f == 0 {
                world.damage_mobj(mo, None, None, 10);
            }
        }
        7 => {
            // NUKAGE DAMAGE
            if !has_ironfeet && world.leveltime & 0x1f == 0 {
                world.damage_mobj(mo, None, None, 5);
            }
        }
        16 | 4 => {
            // SUPER HELLSLIME DAMAGE / STROBE HURT
            if (!has_ironfeet || (world.rng.p.next() as i32) < 5)
                && world.leveltime & 0x1f == 0
            {
                world.damage_mobj(mo, None, None, 20);
            }
        }
        9 => {
            // SECRET SECTOR
            world.players[pnum].secretcount += 1;
            world.level.sectors[sector].special = 0;
        }
        11 => {
            // EXIT SUPER DAMAGE! (for E1M8 finale)
            world.players[pnum].cheats &= !crate::d_player::Player::CF_GODMODE;

            if world.leveltime & 0x1f == 0 {
                world.damage_mobj(mo, None, None, 20);
            }

            if world.players[pnum].health <= 10 {
                world.pending_exit = Some(false);
            }
        }
        _ => {
            // Vanilla `I_Error`s on unknown specials; ignore instead.
        }
    }
}

/// `P_UpdateSpecials` — animate scrolling walls and advance button timers.
/// (Flat/texture animation lives in the render layer; the level timer lives
/// in `line_specials`.)
pub fn update_specials(world: &mut World) {
    // LEVEL TIMER
    if world.line_specials.level_timer {
        world.line_specials.level_time_count -= 1;
        if world.line_specials.level_time_count == 0 {
            world.pending_exit = Some(false);
        }
    }

    // ANIMATE LINE SPECIALS: 48 = EFFECT FIRSTCOL SCROLL+
    for &li in &world.line_specials.scroll_lines.clone() {
        if world.level.lines[li].special != 48 {
            continue;
        }
        let side = world.level.lines[li].sidenum[0];
        if side != -1 {
            world.level.sides[side as usize].textureoffset += FRAC_UNIT;
        }
    }

    // DO BUTTONS
    crate::p_switch::update_buttons(world);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::level::{Level, Line, Sector};

    /// Two adjacent sectors (0 tall, 1 short) joined by a two-sided line with
    /// `tag` 7, plus a one-sided line in sector 1.
    fn two_room_level() -> Level {
        let mut level = Level::default();
        for (floor, ceil) in [(0, 128), (64, 96)] {
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
        level.lines.push(Line {
            frontsector: Some(1),
            tag: 99,
            ..Line::default()
        });
        level.sectors[0].lines = vec![0];
        level.sectors[1].lines = vec![0, 1];
        level
    }

    #[test]
    fn find_helpers_two_rooms() {
        let level = two_room_level();
        assert_eq!(find_lowest_floor_surrounding(&level, 0), 0);
        assert_eq!(find_highest_floor_surrounding(&level, 0), 64 << 16);
        assert_eq!(find_lowest_ceiling_surrounding(&level, 0), 96 << 16);
        assert_eq!(find_highest_ceiling_surrounding(&level, 0), 96 << 16);
        // One-sided lines are skipped by get_next_sector: sector 1's only
        // neighbour through line 0 is sector 0 (floor 0, ceiling 128).
        assert_eq!(find_highest_floor_surrounding(&level, 1), 0);
        assert_eq!(find_lowest_floor_surrounding(&level, 1), 0);
        assert_eq!(find_lowest_ceiling_surrounding(&level, 1), 128 << 16);
        // No higher neighbour: next-highest is the current floor itself.
        assert_eq!(find_next_highest_floor(&level, 1, 64 << 16), 64 << 16);
        // From the floor, the next highest step is the ledge.
        assert_eq!(find_next_highest_floor(&level, 0, 0), 64 << 16);
        // Nothing higher than the ceiling: keeps the current height.
        assert_eq!(find_next_highest_floor(&level, 0, 96 << 16), 96 << 16);
    }

    #[test]
    fn find_sector_from_line_tag_walks_all_matches() {
        let level = two_room_level();
        assert_eq!(find_sector_from_line_tag(&level, 0, None), Some(1));
        assert_eq!(find_sector_from_line_tag(&level, 0, Some(1)), None);
        // No sector carries tag 99.
        assert_eq!(find_sector_from_line_tag(&level, 1, None), None);
    }
}
