//! Floor animation (raising stairs, moving floors), port of `p_floor.c`.
//!
//! `T_MovePlane` lives here like in the C source; it is a method on
//! [`World`] because a failed move must run `P_ChangeSector` (crush checks)
//! against the mobj world.

use crate::fixed::FRAC_UNIT;
use crate::wad::LumpName;
use crate::p_spec::{
    find_highest_floor_surrounding, find_lowest_ceiling_surrounding,
    find_lowest_floor_surrounding, find_next_highest_floor, find_sector_from_line_tag,
    PlaneResult, FLOORSPEED,
};
use crate::p_tick::{ThinkerId, ThinkerKind};
use crate::p_world::World;
use crate::sounds::Sfx;

/// `floor_e`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloorType {
    /// lower floor to highest surrounding floor.
    LowerFloor,
    /// lower floor to lowest surrounding floor.
    LowerFloorToLowest,
    /// lower floor to highest surrounding floor VERY FAST.
    TurboLower,
    /// raise floor to lowest surrounding ceiling.
    RaiseFloor,
    /// raise floor to highest surrounding ceiling.
    RaiseFloorToHighest,
    RaiseFloorToNearest,
    RaiseFloor24,
    RaiseFloor512,
    RaiseFloor24AndChange,
    RaiseFloorCrush,
    RaiseFloorTurbo,
    /// raise to next highest surrounding floor.
    RaiseToTexture,
    LowerAndChange,
    /// `EV_DoDonut` (`p_spec.c`).
    DonutRaise,
}

/// `stair_e`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StairType {
    Build8,
    Turbo16,
}

/// `floormove_t`.
#[derive(Clone, Debug)]
pub struct FloorMove {
    /// Link into [`crate::p_tick::Thinkers`].
    pub thinker: ThinkerId,
    pub ty: FloorType,
    pub crush: bool,
    pub sector: usize,
    /// `-1` down, `1` up.
    pub direction: i32,
    pub newspecial: i16,
    pub texture: LumpName,
    pub floordestheight: i32,
    pub speed: i32,
}

impl World {
    /// `T_MovePlane`: move a plane (floor or ceiling) and check for crushing.
    ///
    /// `ceiling` selects the plane (`floorOrCeiling`: 0 floor, 1 ceiling);
    /// `direction` is `1` up / `-1` down.
    #[allow(clippy::too_many_arguments)]
    pub fn move_plane(
        &mut self,
        sector: usize,
        speed: i32,
        dest: i32,
        crush: bool,
        ceiling: bool,
        direction: i32,
    ) -> PlaneResult {
        // Height accessor by value to keep `self` free for `change_sector`.
        let height = |w: &Self| {
            if ceiling {
                w.level.sectors[sector].ceilingheight
            } else {
                w.level.sectors[sector].floorheight
            }
        };
        let set_height = |w: &mut Self, h: i32| {
            if ceiling {
                w.level.sectors[sector].ceilingheight = h;
            } else {
                w.level.sectors[sector].floorheight = h;
            }
        };

        if !ceiling {
            // FLOOR
            match direction {
                -1 => {
                    // DOWN
                    if height(self) - speed < dest {
                        let lastpos = height(self);
                        set_height(self, dest);
                        let flag = self.change_sector(sector, crush);
                        if flag {
                            set_height(self, lastpos);
                            self.change_sector(sector, crush);
                        }
                        return PlaneResult::PastDest;
                    }
                    let lastpos = height(self);
                    set_height(self, lastpos - speed);
                    let flag = self.change_sector(sector, crush);
                    if flag {
                        set_height(self, lastpos);
                        self.change_sector(sector, crush);
                        return PlaneResult::Crushed;
                    }
                }
                1 => {
                    // UP
                    if height(self) + speed > dest {
                        let lastpos = height(self);
                        set_height(self, dest);
                        let flag = self.change_sector(sector, crush);
                        if flag {
                            set_height(self, lastpos);
                            self.change_sector(sector, crush);
                        }
                        return PlaneResult::PastDest;
                    }
                    // COULD GET CRUSHED
                    let lastpos = height(self);
                    set_height(self, lastpos + speed);
                    let flag = self.change_sector(sector, crush);
                    if flag {
                        if crush {
                            return PlaneResult::Crushed;
                        }
                        set_height(self, lastpos);
                        self.change_sector(sector, crush);
                        return PlaneResult::Crushed;
                    }
                }
                _ => {}
            }
        } else {
            // CEILING
            match direction {
                -1 => {
                    // DOWN
                    if height(self) - speed < dest {
                        let lastpos = height(self);
                        set_height(self, dest);
                        let flag = self.change_sector(sector, crush);
                        if flag {
                            set_height(self, lastpos);
                            self.change_sector(sector, crush);
                        }
                        return PlaneResult::PastDest;
                    }
                    // COULD GET CRUSHED
                    let lastpos = height(self);
                    set_height(self, lastpos - speed);
                    let flag = self.change_sector(sector, crush);
                    if flag {
                        if crush {
                            return PlaneResult::Crushed;
                        }
                        set_height(self, lastpos);
                        self.change_sector(sector, crush);
                        return PlaneResult::Crushed;
                    }
                }
                1 => {
                    // UP
                    if height(self) + speed > dest {
                        let lastpos = height(self);
                        set_height(self, dest);
                        let flag = self.change_sector(sector, crush);
                        if flag {
                            set_height(self, lastpos);
                            self.change_sector(sector, crush);
                        }
                        return PlaneResult::PastDest;
                    }
                    let lastpos = height(self);
                    set_height(self, lastpos + speed);
                    // No crush check on the way up (the vanilla `#if 0` block).
                }
                _ => {}
            }
        }
        PlaneResult::Ok
    }
}

/// `T_MoveFloor`: move a floor to its destination (up or down).
pub fn t_move_floor(world: &mut World, floor: usize) {
    let Some(f) = world.specials.floors.get(floor) else {
        return;
    };
    let (sector, speed, floordestheight, crush, direction, ty, newspecial, texture) = (
        f.sector,
        f.speed,
        f.floordestheight,
        f.crush,
        f.direction,
        f.ty,
        f.newspecial,
        f.texture,
    );

    let res = world.move_plane(sector, speed, floordestheight, crush, false, direction);

    if world.leveltime & 7 == 0 {
        world.play_sfx(Sfx::sfx_stnmov);
    }

    if res == PlaneResult::PastDest {
        world.level.sectors[sector].specialdata = None;

        if direction == 1 {
            if let FloorType::DonutRaise = ty {
                let s = &mut world.level.sectors[sector];
                s.special = newspecial;
                s.floorpic = texture;
            }
        } else if direction == -1 {
            if let FloorType::LowerAndChange = ty {
                let s = &mut world.level.sectors[sector];
                s.special = newspecial;
                s.floorpic = texture;
            }
        }
        let thinker = world.specials.floors[floor].thinker;
        world.thinkers.remove(thinker);

        world.play_sfx(Sfx::sfx_pstop);
    }
}

pub fn add_floor(world: &mut World, sector: usize, f: FloorMove) -> usize {
    let idx = world.specials.floors.len();
    let mut f = f;
    f.sector = sector;
    world.specials.floors.push(f);
    let thinker = world.thinkers.add(ThinkerKind::Floor(idx));
    world.specials.floors[idx].thinker = thinker;
    world.level.sectors[sector].specialdata = Some(thinker);
    idx
}

/// `EV_DoFloor`: handle floor types on every sector tagged like `line`.
///
/// Texture heights for [`FloorType::RaiseToTexture`] come from
/// `world.texture_heights` (filled by the host from `RenderData`).
pub fn ev_do_floor(world: &mut World, line: usize, ty: FloorType) -> bool {
    ev_do_floor_by_tag(world, world.level.lines[line].tag, ty)
}

/// The `EV_DoFloor` body, driving sectors by tag directly (boss deaths use
/// synthetic tags 666/667).
pub fn ev_do_floor_by_tag(world: &mut World, tag: i16, ty: FloorType) -> bool {
    // The by-tag path has no backing line: floor-changing specials that read
    // `line->frontsector` (RaiseFloor24AndChange) keep the old floor pic,
    // like vanilla's synthetic `line_t junk` (zeroed front sector).
    use crate::p_spec::find_sector_from_tag;
    let mut rtn = false;
    let mut secn = None;
    while let Some(sec) = find_sector_from_tag(&world.level, tag, secn) {
        secn = Some(sec);

        // ALREADY MOVING?  IF SO, KEEP GOING...
        if world.level.sectors[sec].specialdata.is_some() {
            continue;
        }

        // New floor thinker.
        rtn = true;
        let (floorheight, ceilingheight) = {
            let s = &world.level.sectors[sec];
            (s.floorheight, s.ceilingheight)
        };
        let mut f = FloorMove {
            thinker: 0,
            ty,
            crush: false,
            sector: 0,
            direction: 0,
            newspecial: 0,
            texture: LumpName::default(),
            floordestheight: 0,
            speed: 0,
        };

        match ty {
            FloorType::LowerFloor => {
                f.direction = -1;
                f.speed = FLOORSPEED;
                f.floordestheight = find_highest_floor_surrounding(&world.level, sec);
            }
            FloorType::LowerFloorToLowest => {
                f.direction = -1;
                f.speed = FLOORSPEED;
                f.floordestheight = find_lowest_floor_surrounding(&world.level, sec);
            }
            FloorType::TurboLower => {
                f.direction = -1;
                f.speed = FLOORSPEED * 4;
                f.floordestheight = find_highest_floor_surrounding(&world.level, sec);
                if f.floordestheight != floorheight {
                    f.floordestheight += 8 * FRAC_UNIT;
                }
            }
            FloorType::RaiseFloorCrush => {
                f.crush = true;
                f.direction = 1;
                f.speed = FLOORSPEED;
                f.floordestheight = find_lowest_ceiling_surrounding(&world.level, sec);
                if f.floordestheight > ceilingheight {
                    f.floordestheight = ceilingheight;
                }
                f.floordestheight -= 8 * FRAC_UNIT;
            }
            FloorType::RaiseFloor => {
                f.direction = 1;
                f.speed = FLOORSPEED;
                f.floordestheight = find_lowest_ceiling_surrounding(&world.level, sec);
                if f.floordestheight > ceilingheight {
                    f.floordestheight = ceilingheight;
                }
            }
            FloorType::RaiseFloorTurbo => {
                f.direction = 1;
                f.speed = FLOORSPEED * 4;
                f.floordestheight = find_next_highest_floor(&world.level, sec, floorheight);
            }
            FloorType::RaiseFloorToNearest => {
                f.direction = 1;
                f.speed = FLOORSPEED;
                f.floordestheight = find_next_highest_floor(&world.level, sec, floorheight);
            }
            FloorType::RaiseFloor24 => {
                f.direction = 1;
                f.speed = FLOORSPEED;
                f.floordestheight = floorheight + 24 * FRAC_UNIT;
            }
            FloorType::RaiseFloor512 => {
                f.direction = 1;
                f.speed = FLOORSPEED;
                f.floordestheight = floorheight + 512 * FRAC_UNIT;
            }
            FloorType::RaiseFloor24AndChange => {
                f.direction = 1;
                f.speed = FLOORSPEED;
                f.floordestheight = floorheight + 24 * FRAC_UNIT;
                let front: Option<usize> = None; // see the by-tag note above
                if let Some(front) = front {
                    let (pic, special) = {
                        let fs = &world.level.sectors[front];
                        (fs.floorpic, fs.special)
                    };
                    let s = &mut world.level.sectors[sec];
                    s.floorpic = pic;
                    s.special = special;
                }
            }
            FloorType::RaiseToTexture => {
                f.direction = 1;
                f.speed = FLOORSPEED;
                let mut minsize = i32::MAX;
                for &li in &world.level.sectors[sec].lines {
                    let l = &world.level.lines[li as usize];
                    if l.flags & crate::doomdef::ML_TWOSIDED == 0 {
                        continue;
                    }
                    for side in 0..2 {
                        let Some(&sidenum) = l.sidenum.get(side).filter(|&&s| s != -1) else {
                            continue;
                        };
                        let side_def = &world.level.sides[sidenum as usize];
                        if let Some(&h) = world
                            .texture_heights
                            .iter()
                            .find(|(n, _)| *n == side_def.bottomtexture)
                            .map(|(_, h)| h)
                        {
                            if h < minsize {
                                minsize = h;
                            }
                        }
                    }
                }
                // Vanilla overflows if no two-sided texture is found; wrap.
                f.floordestheight = floorheight.wrapping_add(minsize);
            }
            FloorType::LowerAndChange => {
                f.direction = -1;
                f.speed = FLOORSPEED;
                f.floordestheight = find_lowest_floor_surrounding(&world.level, sec);
                f.texture = world.level.sectors[sec].floorpic;

                for &li in &world.level.sectors[sec].lines.clone() {
                    let l = &world.level.lines[li as usize];
                    if l.flags & crate::doomdef::ML_TWOSIDED == 0 {
                        continue;
                    }
                    // The sector on the other side of this line.
                    let other = if l.frontsector == Some(sec) {
                        l.backsector
                    } else {
                        l.frontsector
                    };
                    let Some(other) = other else { continue };
                    if world.level.sectors[other].floorheight == f.floordestheight {
                        let os = &world.level.sectors[other];
                        f.texture = os.floorpic;
                        f.newspecial = os.special;
                        break;
                    }
                }
            }
            FloorType::RaiseFloorToHighest | FloorType::DonutRaise => {}
        }

        add_floor(world, sec, f);
    }
    rtn
}

/// `EV_BuildStairs`: build a staircase!
pub fn ev_build_stairs(world: &mut World, line: usize, ty: StairType) -> bool {
    let (speed, stairsize) = match ty {
        StairType::Build8 => (FLOORSPEED / 4, 8 * FRAC_UNIT),
        StairType::Turbo16 => (FLOORSPEED * 4, 16 * FRAC_UNIT),
    };

    let mut rtn = false;
    let mut secn = None;
    while let Some(sec0) = find_sector_from_line_tag(&world.level, line, secn) {
        secn = Some(sec0);

        // ALREADY MOVING?  IF SO, KEEP GOING...
        if world.level.sectors[sec0].specialdata.is_some() {
            continue;
        }

        // New floor thinker.
        rtn = true;
        let mut height = world.level.sectors[sec0].floorheight + stairsize;
        add_floor(
            world,
            sec0,
            FloorMove {
                thinker: 0,
                ty: FloorType::RaiseFloor, // unused by the thinker here
                crush: false,
                sector: 0,
                direction: 1,
                newspecial: 0,
                texture: LumpName::default(),
                floordestheight: height,
                speed,
            },
        );

        let texture = world.level.sectors[sec0].floorpic;

        // Find next sector to raise:
        // 1. Find 2-sided line with same sector side[0]
        // 2. Other side is the next sector to raise
        let mut sec = sec0;
        loop {
            let lines = world.level.sectors[sec].lines.clone();
            let mut ok = false;
            for li in lines {
                let l = &world.level.lines[li as usize];
                if l.flags & crate::doomdef::ML_TWOSIDED == 0 {
                    continue;
                }
                if l.frontsector != Some(sec) {
                    continue;
                }
                let Some(tsec) = l.backsector else {
                    continue;
                };
                if world.level.sectors[tsec].floorpic != texture {
                    continue;
                }
                height += stairsize;
                if world.level.sectors[tsec].specialdata.is_some() {
                    continue;
                }

                sec = tsec;
                add_floor(
                    world,
                    tsec,
                    FloorMove {
                        thinker: 0,
                        ty: FloorType::RaiseFloor,
                        crush: false,
                        sector: 0,
                        direction: 1,
                        newspecial: 0,
                        texture: LumpName::default(),
                        floordestheight: height,
                        speed,
                    },
                );
                ok = true;
                break;
            }
            if !ok {
                break;
            }
        }
    }
    rtn
}

impl FloorType {
    /// Savegame restore: table index -> enum.
    pub fn from_index(i: i32) -> FloorType {
        const ALL: [FloorType; 14] = [
            FloorType::LowerFloor,
            FloorType::LowerFloorToLowest,
            FloorType::TurboLower,
            FloorType::RaiseFloor,
            FloorType::RaiseFloorToHighest,
            FloorType::RaiseFloorToNearest,
            FloorType::RaiseFloor24,
            FloorType::RaiseFloor512,
            FloorType::RaiseFloor24AndChange,
            FloorType::RaiseFloorCrush,
            FloorType::RaiseFloorTurbo,
            FloorType::RaiseToTexture,
            FloorType::LowerAndChange,
            FloorType::DonutRaise,
        ];
        ALL[(i as usize).min(13)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::level::Level;
    use crate::level::{Line, Sector};
    use crate::r_main::Render;

    /// Room (0) at floor 0 over a pit (1) at floor -32; both ceiling 128.
    fn step_level() -> World {
        let mut level = Level::default();
        for (floor, ceil) in [(0, 128), (-32, 128)] {
            level.sectors.push(Sector {
                floorheight: floor << 16,
                ceilingheight: ceil << 16,
                ..Sector::default()
            });
        }
        level.sectors[1].tag = 7;
        // Pit (1) is the front sector so stair chains walk into the room.
        level.lines.push(Line {
            flags: crate::doomdef::ML_TWOSIDED,
            tag: 7,
            frontsector: Some(1),
            backsector: Some(0),
            ..Line::default()
        });
        level.sectors[0].lines = vec![0];
        level.sectors[1].lines = vec![0];
        World::new(level, Render::new())
    }

    #[test]
    fn raise_floor24_reaches_destination() {
        let mut w = step_level();
        assert!(ev_do_floor(&mut w, 0, FloorType::RaiseFloor24));
        assert_eq!(w.specials.floors.len(), 1);
        // -32 + 24 = -8; 1 unit per tic.
        for _ in 0..23 {
            w.run_thinkers();
        }
        assert!(w.level.sectors[1].floorheight < -8 << 16);
        for _ in 0..3 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].floorheight, -8 << 16);
        assert_eq!(w.thinkers.iter().count(), 0);
        assert!(w.level.sectors[1].specialdata.is_none());
    }

    #[test]
    fn turbo_lower_keeps_gap() {
        let mut w = step_level();
        assert!(ev_do_floor(&mut w, 0, FloorType::TurboLower));
        // Pit floor lowers to room floor (0), but not equal: + 8.
        for _ in 0..12 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].floorheight, 8 << 16);
        assert_eq!(w.specials.floors[0].floordestheight, 8 << 16);
    }

    #[test]
    fn build_stairs_walks_the_chain() {
        let mut w = step_level();
        // Chain: sector 1 (pit, tag 7) raises first, its two-sided neighbour
        // (sector 0) shares the floorpic and raises next.
        assert!(ev_build_stairs(&mut w, 0, StairType::Build8));
        assert_eq!(w.specials.floors.len(), 2);
        assert_eq!(w.specials.floors[0].floordestheight, -24 << 16); // -32 + 8
        assert_eq!(w.specials.floors[1].floordestheight, -16 << 16); // chain + 8
        for _ in 0..70 {
            w.run_thinkers();
        }
        assert_eq!(w.level.sectors[1].floorheight, -24 << 16);
        assert_eq!(w.level.sectors[0].floorheight, -16 << 16);
    }
}
