//! Switch texture flipping and buttons, port of `p_switch.c`.
//!
//! `switchlist` is resolved from WAD texture names at init
//! ([`init_switch_list`]); sides still carry [`LumpName`]s in this port, so
//! the active list stores names rather than vanilla texture indices.

use alloc::vec::Vec;
use crate::p_world::World;
use crate::sounds::Sfx;
use crate::wad::LumpName;

/// `BUTTONTIME`.
pub const BUTTONTIME: i32 = 35;
/// `MAXBUTTONS`.
pub const MAXBUTTONS: usize = 16;

/// `bwhere_e`: which sidedef texture slot a button animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonWhere {
    Top,
    Middle,
    Bottom,
}

/// `button_t` (the timer is advanced by `P_UpdateSpecials`, not a thinker).
#[derive(Clone, Debug)]
pub struct Button {
    pub line: usize,
    pub where_: ButtonWhere,
    pub texture: LumpName,
    pub timer: i32,
}

/// `alphSwitchList` entry.
struct SwitchNames {
    name1: &'static str,
    name2: &'static str,
    episode: u8,
}

/// `alphSwitchList`: (off, on, episode availability).
static ALPH_SWITCH_LIST: [SwitchNames; 40] = [
    // Doom shareware episode 1 switches
    SwitchNames { name1: "SW1BRCOM", name2: "SW2BRCOM", episode: 1 },
    SwitchNames { name1: "SW1BRN1", name2: "SW2BRN1", episode: 1 },
    SwitchNames { name1: "SW1BRN2", name2: "SW2BRN2", episode: 1 },
    SwitchNames { name1: "SW1BRNGN", name2: "SW2BRNGN", episode: 1 },
    SwitchNames { name1: "SW1BROWN", name2: "SW2BROWN", episode: 1 },
    SwitchNames { name1: "SW1COMM", name2: "SW2COMM", episode: 1 },
    SwitchNames { name1: "SW1COMP", name2: "SW2COMP", episode: 1 },
    SwitchNames { name1: "SW1DIRT", name2: "SW2DIRT", episode: 1 },
    SwitchNames { name1: "SW1EXIT", name2: "SW2EXIT", episode: 1 },
    SwitchNames { name1: "SW1GRAY", name2: "SW2GRAY", episode: 1 },
    SwitchNames { name1: "SW1GRAY1", name2: "SW2GRAY1", episode: 1 },
    SwitchNames { name1: "SW1METAL", name2: "SW2METAL", episode: 1 },
    SwitchNames { name1: "SW1PIPE", name2: "SW2PIPE", episode: 1 },
    SwitchNames { name1: "SW1SLAD", name2: "SW2SLAD", episode: 1 },
    SwitchNames { name1: "SW1STARG", name2: "SW2STARG", episode: 1 },
    SwitchNames { name1: "SW1STON1", name2: "SW2STON1", episode: 1 },
    SwitchNames { name1: "SW1STON2", name2: "SW2STON2", episode: 1 },
    SwitchNames { name1: "SW1STONE", name2: "SW2STONE", episode: 1 },
    SwitchNames { name1: "SW1STRTN", name2: "SW2STRTN", episode: 1 },
    // Doom registered episodes 2&3 switches
    SwitchNames { name1: "SW1BLUE", name2: "SW2BLUE", episode: 2 },
    SwitchNames { name1: "SW1CMT", name2: "SW2CMT", episode: 2 },
    SwitchNames { name1: "SW1GARG", name2: "SW2GARG", episode: 2 },
    SwitchNames { name1: "SW1GSTON", name2: "SW2GSTON", episode: 2 },
    SwitchNames { name1: "SW1HOT", name2: "SW2HOT", episode: 2 },
    SwitchNames { name1: "SW1LION", name2: "SW2LION", episode: 2 },
    SwitchNames { name1: "SW1SATYR", name2: "SW2SATYR", episode: 2 },
    SwitchNames { name1: "SW1SKIN", name2: "SW2SKIN", episode: 2 },
    SwitchNames { name1: "SW1VINE", name2: "SW2VINE", episode: 2 },
    SwitchNames { name1: "SW1WOOD", name2: "SW2WOOD", episode: 2 },
    // Doom II switches
    SwitchNames { name1: "SW1PANEL", name2: "SW2PANEL", episode: 3 },
    SwitchNames { name1: "SW1ROCK", name2: "SW2ROCK", episode: 3 },
    SwitchNames { name1: "SW1MET2", name2: "SW2MET2", episode: 3 },
    SwitchNames { name1: "SW1WDMET", name2: "SW2WDMET", episode: 3 },
    SwitchNames { name1: "SW1BRIK", name2: "SW2BRIK", episode: 3 },
    SwitchNames { name1: "SW1MOD1", name2: "SW2MOD1", episode: 3 },
    SwitchNames { name1: "SW1ZIM", name2: "SW2ZIM", episode: 3 },
    SwitchNames { name1: "SW1STON6", name2: "SW2STON6", episode: 3 },
    SwitchNames { name1: "SW1TEK", name2: "SW2TEK", episode: 3 },
    SwitchNames { name1: "SW1MARB", name2: "SW2MARB", episode: 3 },
    SwitchNames { name1: "SW1SKULL", name2: "SW2SKULL", episode: 3 },
];

/// `switchlist`: pairs of (off, on) texture names, resolved per game episode.
///
/// `episode` mirrors vanilla: 1 shareware, 2 registered/retail, 3 commercial.
/// Entries whose names the WAD does not carry keep their names anyway; the
/// side-texture comparison is by name so missing textures simply never match.
#[derive(Clone, Debug, Default)]
pub struct SwitchList {
    pub pairs: Vec<(LumpName, LumpName)>,
}

impl SwitchList {
    /// `P_InitSwitchList`.
    pub fn init(commercial: bool, registered_or_retail: bool) -> Self {
        let episode = if commercial {
            3
        } else if registered_or_retail {
            2
        } else {
            1
        };
        let mut pairs = Vec::new();
        for s in ALPH_SWITCH_LIST.iter() {
            if s.episode <= episode {
                pairs.push((name(s.name1), name(s.name2)));
            }
        }
        SwitchList { pairs }
    }

    /// The opposite texture of `tex`, if it is a switch (`switchlist[i^1]`).
    fn opposite(&self, tex: &LumpName) -> Option<LumpName> {
        for (a, b) in &self.pairs {
            if a == tex {
                return Some(*b);
            }
            if b == tex {
                return Some(*a);
            }
        }
        None
    }
}

fn name(s: &str) -> LumpName {
    LumpName::from_bytes(s.as_bytes())
}

/// `P_StartButton`: start a button counting down till it turns off.
fn start_button(world: &mut World, line: usize, where_: ButtonWhere, texture: LumpName, time: i32) {
    // See if button is already pressed.
    for b in &world.specials.buttons {
        if b.timer != 0 && b.line == line {
            return;
        }
    }
    for i in 0..world.specials.buttons.len() {
        if world.specials.buttons[i].timer == 0 {
            let b = &mut world.specials.buttons[i];
            b.line = line;
            b.where_ = where_;
            b.texture = texture;
            b.timer = time;
            return;
        }
    }
    // Vanilla `I_Error`s here; the port grows one slot instead.
    world.specials.buttons.push(Button {
        line,
        where_,
        texture,
        timer: time,
    });
}

/// `P_ChangeSwitchTexture`: change the wall texture of a switch to its
/// opposite. `use_again == true` schedules it to flip back (a button).
pub fn change_switch_texture(world: &mut World, line: usize, use_again: bool) {
    if !use_again {
        world.level.lines[line].special = 0;
    }

    let side = world.level.lines[line].sidenum[0];
    if side == -1 {
        return;
    }
    let side = side as usize;

    let sound = if world.level.lines[line].special == 11 {
        Sfx::sfx_swtchx // EXIT SWITCH?
    } else {
        Sfx::sfx_swtchn
    };

    let switches = &world.switches;
    let (tex_top, tex_mid, tex_bot) = {
        let s = &world.level.sides[side];
        (s.toptexture, s.midtexture, s.bottomtexture)
    };
    for (tex, where_) in [
        (tex_top, ButtonWhere::Top),
        (tex_mid, ButtonWhere::Middle),
        (tex_bot, ButtonWhere::Bottom),
    ] {
        if let Some(alt) = switches.opposite(&tex) {
            world.play_sfx(sound);
            let s = &mut world.level.sides[side];
            match where_ {
                ButtonWhere::Top => s.toptexture = alt,
                ButtonWhere::Middle => s.midtexture = alt,
                ButtonWhere::Bottom => s.bottomtexture = alt,
            }
            if use_again {
                start_button(world, line, where_, tex, BUTTONTIME);
            }
            return;
        }
    }
}

/// Button timer advance, called from `P_UpdateSpecials` (`p_spec.c`).
pub fn update_buttons(world: &mut World) {
    for i in 0..world.specials.buttons.len() {
        if world.specials.buttons[i].timer == 0 {
            continue;
        }
        world.specials.buttons[i].timer -= 1;
        if world.specials.buttons[i].timer == 0 {
            let b = &world.specials.buttons[i];
            let (line, where_, texture) = (b.line, b.where_, b.texture);
            let side = world.level.lines[line].sidenum[0];
            if side != -1 {
                let s = &mut world.level.sides[side as usize];
                match where_ {
                    ButtonWhere::Top => s.toptexture = texture,
                    ButtonWhere::Middle => s.midtexture = texture,
                    ButtonWhere::Bottom => s.bottomtexture = texture,
                }
            }
            world.play_sfx(Sfx::sfx_swtchn);
        }
    }
    // Reuse: compact dead slots so `start_button` free-slot scans stay short.
    world.specials.buttons.retain(|b| b.timer != 0);
}

/// `P_UseSpecialLine` (`p_switch.c`): called when a thing uses a special
/// line. Only the front sides of lines are usable.
pub fn use_special_line(world: &mut World, thing: usize, line: usize, side: usize) -> bool {
    use crate::p_ceilng::{ev_do_ceiling, CeilingType};
    use crate::p_doors::{
        ev_do_door, ev_do_locked_door, ev_vertical_door, VlDoorType,
    };
    use crate::p_floor::{ev_build_stairs, ev_do_floor, FloorType, StairType};
    use crate::p_plats::{ev_do_plat, PlatType};
    use crate::p_spec::ev_do_donut;

    let special = world.level.lines[line].special;

    // Err... Use the back sides of VERY SPECIAL lines...
    if side != 0 && special != 124 {
        return false;
    }

    // Switches that other things can activate.
    if world.mobjs.get(thing).and_then(|m| m.player).is_none() {
        // never open secret doors
        if world.level.lines[line].flags & crate::doomdef::ML_SECRET != 0 {
            return false;
        }
        if !matches!(special, 1 | 32 | 33 | 34) {
            return false;
        }
    }

    // do something
    macro_rules! once {
        ($e:expr) => {{
            if $e {
                change_switch_texture(world, line, false);
            }
        }};
    }
    macro_rules! again {
        ($e:expr) => {{
            if $e {
                change_switch_texture(world, line, true);
            }
        }};
    }

    match special {
        // MANUALS
        1 | 26 | 27 | 28 | 31 | 32 | 33 | 34 | 117 | 118 => {
            ev_vertical_door(world, line, thing);
        }

        // SWITCHES
        7 => once!(ev_build_stairs(world, line, StairType::Build8)),
        9 => once!(ev_do_donut(world, line)),
        11 => {
            // Exit level
            change_switch_texture(world, line, false);
            world.pending_exit = Some(false);
        }
        14 => once!(ev_do_plat(world, line, PlatType::RaiseAndChange, 32)),
        15 => once!(ev_do_plat(world, line, PlatType::RaiseAndChange, 24)),
        18 => once!(ev_do_floor(world, line, FloorType::RaiseFloorToNearest)),
        20 => once!(ev_do_plat(world, line, PlatType::RaiseToNearestAndChange, 0)),
        21 => once!(ev_do_plat(world, line, PlatType::DownWaitUpStay, 0)),
        23 => once!(ev_do_floor(world, line, FloorType::LowerFloorToLowest)),
        29 => once!(ev_do_door(world, line, VlDoorType::Normal)),
        41 => once!(ev_do_ceiling(world, line, CeilingType::LowerToFloor)),
        71 => once!(ev_do_floor(world, line, FloorType::TurboLower)),
        49 => once!(ev_do_ceiling(world, line, CeilingType::CrushAndRaise)),
        50 => once!(ev_do_door(world, line, VlDoorType::Close)),
        51 => {
            // Secret EXIT
            change_switch_texture(world, line, false);
            world.pending_exit = Some(true);
        }
        55 => once!(ev_do_floor(world, line, FloorType::RaiseFloorCrush)),
        101 => once!(ev_do_floor(world, line, FloorType::RaiseFloor)),
        102 => once!(ev_do_floor(world, line, FloorType::LowerFloor)),
        103 => once!(ev_do_door(world, line, VlDoorType::Open)),
        111 => once!(ev_do_door(world, line, VlDoorType::BlazeRaise)),
        112 => once!(ev_do_door(world, line, VlDoorType::BlazeOpen)),
        113 => once!(ev_do_door(world, line, VlDoorType::BlazeClose)),
        122 => once!(ev_do_plat(world, line, PlatType::BlazeDWUS, 0)),
        127 => once!(ev_build_stairs(world, line, StairType::Turbo16)),
        131 => once!(ev_do_floor(world, line, FloorType::RaiseFloorTurbo)),
        133 | 135 | 137 => {
            // BlzOpenDoor BLUE/RED/YELLOW
            if ev_do_locked_door(world, line, VlDoorType::BlazeOpen, thing) {
                change_switch_texture(world, line, false);
            }
        }
        140 => once!(ev_do_floor(world, line, FloorType::RaiseFloor512)),

        // BUTTONS
        42 => again!(ev_do_door(world, line, VlDoorType::Close)),
        43 => again!(ev_do_ceiling(world, line, CeilingType::LowerToFloor)),
        45 => again!(ev_do_floor(world, line, FloorType::LowerFloor)),
        60 => again!(ev_do_floor(world, line, FloorType::LowerFloorToLowest)),
        61 => again!(ev_do_door(world, line, VlDoorType::Open)),
        62 => again!(ev_do_plat(world, line, PlatType::DownWaitUpStay, 1)),
        63 => again!(ev_do_door(world, line, VlDoorType::Normal)),
        64 => again!(ev_do_floor(world, line, FloorType::RaiseFloor)),
        66 => again!(ev_do_plat(world, line, PlatType::RaiseAndChange, 24)),
        67 => again!(ev_do_plat(world, line, PlatType::RaiseAndChange, 32)),
        65 => again!(ev_do_floor(world, line, FloorType::RaiseFloorCrush)),
        68 => again!(ev_do_plat(world, line, PlatType::RaiseToNearestAndChange, 0)),
        69 => again!(ev_do_floor(world, line, FloorType::RaiseFloorToNearest)),
        70 => again!(ev_do_floor(world, line, FloorType::TurboLower)),
        114 => again!(ev_do_door(world, line, VlDoorType::BlazeRaise)),
        115 => again!(ev_do_door(world, line, VlDoorType::BlazeOpen)),
        116 => again!(ev_do_door(world, line, VlDoorType::BlazeClose)),
        123 => again!(ev_do_plat(world, line, PlatType::BlazeDWUS, 0)),
        132 => again!(ev_do_floor(world, line, FloorType::RaiseFloorTurbo)),
        99 | 134 | 136 => {
            // BlzOpenDoor BLUE/RED/YELLOW
            if ev_do_locked_door(world, line, VlDoorType::BlazeOpen, thing) {
                change_switch_texture(world, line, true);
            }
        }
        138 => {
            // Light Turn On
            crate::p_lights::ev_light_turn_on(world, line, 255);
            change_switch_texture(world, line, true);
        }
        139 => {
            // Light Turn Off
            crate::p_lights::ev_light_turn_on(world, line, 35);
            change_switch_texture(world, line, true);
        }

        _ => {}
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Line, Sector, Side};
    use crate::r_main::Render;

    #[test]
    fn switch_list_filters_by_episode() {
        let sw1 = SwitchList::init(false, false);
        assert_eq!(sw1.pairs.len(), 19);
        let sw2 = SwitchList::init(false, true);
        assert_eq!(sw2.pairs.len(), 29);
        let commercial = SwitchList::init(true, false);
        assert_eq!(commercial.pairs.len(), 40);
        assert_eq!(sw1.pairs[0].0, name("SW1BRCOM"));
    }

    #[test]
    fn change_switch_flips_and_reverts() {
        let mut level = crate::level::Level::default();
        level.sectors.push(Sector::default());
        level.sides.push(Side {
            toptexture: name("SW1BRCOM"),
            ..Side::default()
        });
        level.lines.push(Line {
            special: 62,
            sidenum: [0, -1],
            frontsector: Some(0),
            ..Line::default()
        });
        let mut w = World::new(level, Render::new());
        w.switches = SwitchList::init(false, false);

        change_switch_texture(&mut w, 0, true);
        assert_eq!(w.level.sides[0].toptexture, name("SW2BRCOM"));
        assert_eq!(w.level.lines[0].special, 62); // kept: use again
        assert_eq!(w.specials.buttons.len(), 1);
        assert_eq!(w.specials.buttons[0].timer, BUTTONTIME);

        // Flips back after the timer runs out.
        for _ in 0..BUTTONTIME {
            update_buttons(&mut w);
        }
        assert_eq!(w.level.sides[0].toptexture, name("SW1BRCOM"));
        assert!(w.specials.buttons.is_empty());

        // One-shot use clears the special.
        change_switch_texture(&mut w, 0, false);
        assert_eq!(w.level.lines[0].special, 0);
        assert_eq!(w.level.sides[0].toptexture, name("SW2BRCOM"));
    }
}
