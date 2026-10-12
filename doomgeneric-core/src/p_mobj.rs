//! Map objects, port of the data layer of `p_mobj.c` / `p_mobj.h`.
//!
//! Vanilla stores a `mobj_t` with raw pointers into sector/blockmap lists and
//! a `thinker_t` embedded at the front. Here mobjs live in an index-addressed
//! arena ([`Mobjs`]) inside the world; cross references (`target`, `tracer`,
//! the `snext`/`sprev`/`bnext`/`bprev` links) are [`MobjId`] handles, and the
//! thinker link is a [`ThinkerId`].
//!
//! Blockmap links (`bnext`/`bprev`) are part of the struct but are not wired
//! yet — that needs the blockmap iterator from `p_map`. Sector links are
//! enough for rendering (`r_things`).

use alloc::{vec, vec::Vec};

use crate::fixed::{fixed_mul, Fixed, FRAC_BITS, FRAC_UNIT};

use crate::info::{mobj_type_for_doomednum, MobjInfo, MobjType, Sprite, StateId, MOBJINFO, STATES};
use crate::level::{Level, Thing};
use crate::p_map::{try_move, slide_move};
use crate::p_maputl::aprox_distance;
use crate::p_tick::{ThinkerId, ThinkerKind, Thinkers};
use crate::r_main::Render;
use crate::random::Random;
use crate::tables::ANG45;

/// Handle to a map object in [`Mobjs`].
pub type MobjId = usize;

// `mobjflag_t` (`p_mobj.h`).
pub const MF_SPECIAL: u32 = 1;
pub const MF_SOLID: u32 = 2;
pub const MF_SHOOTABLE: u32 = 4;
pub const MF_NOSECTOR: u32 = 8;
pub const MF_NOBLOCKMAP: u32 = 16;
pub const MF_AMBUSH: u32 = 32;
pub const MF_JUSTHIT: u32 = 64;
pub const MF_JUSTATTACKED: u32 = 128;
pub const MF_SPAWNCEILING: u32 = 256;
pub const MF_NOGRAVITY: u32 = 512;
pub const MF_DROPOFF: u32 = 0x400;
pub const MF_PICKUP: u32 = 0x800;
pub const MF_NOCLIP: u32 = 0x1000;
pub const MF_SLIDE: u32 = 0x2000;
pub const MF_FLOAT: u32 = 0x4000;
pub const MF_TELEPORT: u32 = 0x8000;
pub const MF_MISSILE: u32 = 0x10000;
pub const MF_DROPPED: u32 = 0x20000;
pub const MF_SHADOW: u32 = 0x40000;
pub const MF_NOBLOOD: u32 = 0x80000;
pub const MF_CORPSE: u32 = 0x100000;
pub const MF_INFLOAT: u32 = 0x200000;
pub const MF_COUNTKILL: u32 = 0x400000;
pub const MF_COUNTITEM: u32 = 0x800000;
pub const MF_SKULLFLY: u32 = 0x1000000;
pub const MF_NOTDMATCH: u32 = 0x2000000;
pub const MF_TRANSLATION: u32 = 0xc000000;
pub const MF_TRANSSHIFT: u32 = 26;

/// `ONFLOORZ` / `ONCEILINGZ` spawn sentinels (`p_local.h`).
pub const ONFLOORZ: Fixed = i32::MIN;
pub const ONCEILINGZ: Fixed = i32::MAX;

// `p_mobj.c` movement constants.
const MAXMOVE: Fixed = 30 << 16;
const STOPSPEED: Fixed = 0x1000;
const FRICTION: Fixed = 0xe800;
const FLOATSPEED: Fixed = 4 << 16;
const GRAVITY: Fixed = FRAC_UNIT;

/// `mobj_t`, with handles instead of pointers.
#[derive(Clone, Debug)]
pub struct Mobj {
    /// Link into [`Thinkers`].
    pub thinker: ThinkerId,

    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,

    /// Sector thing list (`snext`/`sprev`), owned by [`Mobjs::sector_heads`].
    pub snext: Option<MobjId>,
    pub sprev: Option<MobjId>,

    pub angle: u32,
    pub sprite: Sprite,
    /// May be ORed with `FF_FULLBRIGHT`.
    pub frame: i32,

    /// Blockmap links — reserved, wired with `p_map`.
    pub bnext: Option<MobjId>,
    pub bprev: Option<MobjId>,

    /// Index of the containing subsector.
    pub subsector: usize,
    /// Sector this mobj is currently linked into (`P_SetThingPosition`).
    pub sector: usize,

    pub floorz: Fixed,
    pub ceilingz: Fixed,

    pub radius: Fixed,
    pub height: Fixed,

    pub momx: Fixed,
    pub momy: Fixed,
    pub momz: Fixed,

    pub validcount: i32,

    pub ty: MobjType,

    pub tics: i32,
    pub state: StateId,
    pub flags: u32,
    pub health: i32,

    /// 0-7, or -1 for "no dir".
    pub movedir: i32,
    pub movecount: i32,

    pub target: Option<MobjId>,
    pub reactiontime: i32,
    pub threshold: i32,

    /// Index into the player array (only meaningful for `MT_PLAYER`).
    pub player: Option<usize>,
    pub lastlook: i32,

    pub spawnpoint: Thing,

    pub tracer: Option<MobjId>,
}

impl Mobj {
    /// `&mobjinfo[mobj->type]`.
    pub fn info(&self) -> &'static MobjInfo {
        &MOBJINFO[self.ty as usize]
    }

    /// A zeroed mobj of `ty` at `(x, y, z)`, with the info defaults applied
    /// (radius/height/flags/health/spawn state fields). No thinker or sector
    /// link is created.
    pub fn from_type(ty: MobjType, x: Fixed, y: Fixed, z: Fixed) -> Self {
        let info = &MOBJINFO[ty as usize];
        let spawn = &STATES[info.spawnstate as usize];
        Mobj {
            thinker: 0,
            x,
            y,
            z,
            snext: None,
            sprev: None,
            angle: 0,
            sprite: spawn.sprite,
            frame: spawn.frame,
            bnext: None,
            bprev: None,
            subsector: 0,
            sector: 0,
            floorz: 0,
            ceilingz: 0,
            radius: info.radius,
            height: info.height,
            momx: 0,
            momy: 0,
            momz: 0,
            validcount: 0,
            ty,
            tics: spawn.tics,
            state: info.spawnstate,
            flags: info.flags,
            health: info.spawnhealth,
            movedir: -1,
            movecount: 0,
            target: None,
            reactiontime: info.reactiontime,
            threshold: 0,
            player: None,
            lastlook: 0,
            spawnpoint: Thing::default(),
            tracer: None,
        }
    }
}

/// The mobj arena plus the per-sector and per-blockmap-cell thing-list heads.
#[derive(Default)]
pub struct Mobjs {
    slots: Vec<Option<Mobj>>,
    free: Vec<MobjId>,
    sector_heads: Vec<Option<MobjId>>,
    block_heads: Vec<Option<MobjId>>,
    bmap_orgx: Fixed,
    bmap_orgy: Fixed,
    bmap_w: usize,
    bmap_h: usize,
}

impl Mobjs {
    pub fn new(num_sectors: usize) -> Self {
        Mobjs {
            slots: Vec::new(),
            free: Vec::new(),
            sector_heads: vec![None; num_sectors],
            block_heads: Vec::new(),
            bmap_orgx: 0,
            bmap_orgy: 0,
            bmap_w: 0,
            bmap_h: 0,
        }
    }

    /// Resize the blockmap link table to match `level` and remember its
    /// geometry. Returns `false` when the level has no blockmap.
    fn sync_blockmap(&mut self, level: &Level) -> bool {
        let d = &level.blockmap.data;
        if d.len() < 4 {
            return false;
        }
        let orgx = (d[0] as i32) << 16;
        let orgy = (d[1] as i32) << 16;
        let w = d[2] as i32;
        let h = d[3] as i32;
        if w <= 0 || h <= 0 {
            return false;
        }
        let (w, h) = (w as usize, h as usize);
        let need = w * h;
        if self.block_heads.len() < need {
            self.block_heads.resize(need, None);
        }
        self.bmap_orgx = orgx;
        self.bmap_orgy = orgy;
        self.bmap_w = w;
        self.bmap_h = h;
        true
    }

    /// `MAPBLOCKSHIFT`.
    const MAPBLOCKSHIFT: u32 = 23;

    /// Blockmap cell containing `(x, y)`, or `None` when outside the map.
    fn block_of(&self, x: Fixed, y: Fixed) -> Option<usize> {
        if self.bmap_w == 0 || self.bmap_h == 0 {
            return None;
        }
        let bx = (x - self.bmap_orgx) >> Self::MAPBLOCKSHIFT;
        let by = (y - self.bmap_orgy) >> Self::MAPBLOCKSHIFT;
        if bx < 0 || by < 0 || bx as usize >= self.bmap_w || by as usize >= self.bmap_h {
            return None;
        }
        Some(by as usize * self.bmap_w + bx as usize)
    }

    /// Head of `block`'s thing list.
    pub fn block_head(&self, block: usize) -> Option<MobjId> {
        self.block_heads.get(block).copied().flatten()
    }

    /// Append the ids of every thing linked into `block` to `out`
    /// (`P_BlockThingsIterator`). The chain may change while `out` is used.
    pub fn collect_block(&self, block: usize, out: &mut Vec<MobjId>) {
        let mut cur = self.block_head(block);
        while let Some(id) = cur {
            out.push(id);
            cur = self.get(id).and_then(|m| m.bnext);
        }
    }

    /// Allocate a slot for `mobj` and return its handle.
    pub fn alloc(&mut self, mobj: Mobj) -> MobjId {
        let id = self.free.pop().unwrap_or_else(|| {
            self.slots.push(None);
            self.slots.len() - 1
        });
        self.slots[id] = Some(mobj);
        id
    }

    pub fn get(&self, id: MobjId) -> Option<&Mobj> {
        self.slots.get(id).and_then(|s| s.as_ref())
    }

    pub fn get_mut(&mut self, id: MobjId) -> Option<&mut Mobj> {
        self.slots.get_mut(id).and_then(|s| s.as_mut())
    }

    pub fn is_live(&self, id: MobjId) -> bool {
        self.get(id).is_some()
    }

    /// Number of live mobjs.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter_ids(&self) -> impl Iterator<Item = MobjId> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|_| i))
    }

    /// Head of `sector`'s thing list.
    pub fn sector_head(&self, sector: usize) -> Option<MobjId> {
        self.sector_heads.get(sector).copied().flatten()
    }

    fn set_sector_head(&mut self, sector: usize, head: Option<MobjId>) {
        if let Some(slot) = self.sector_heads.get_mut(sector) {
            *slot = head;
        }
    }

    fn set_block_head(&mut self, block: usize, head: Option<MobjId>) {
        if let Some(slot) = self.block_heads.get_mut(block) {
            *slot = head;
        }
    }

    /// `P_UnsetThingPosition`: unlinks from the sector and blockmap lists.
    pub fn unset_thing_position(&mut self, id: MobjId) {
        let (sector, sprev, snext, bprev, bnext, x, y, no_sector, no_block) = match self.get(id) {
            Some(m) => (
                m.sector,
                m.sprev,
                m.snext,
                m.bprev,
                m.bnext,
                m.x,
                m.y,
                m.flags & MF_NOSECTOR != 0,
                m.flags & MF_NOBLOCKMAP != 0,
            ),
            None => return,
        };

        if !no_sector {
            match sprev {
                Some(p) => {
                    if let Some(pm) = self.get_mut(p) {
                        pm.snext = snext;
                    }
                }
                None => self.set_sector_head(sector, snext),
            }
            if let Some(n) = snext {
                if let Some(nm) = self.get_mut(n) {
                    nm.sprev = sprev;
                }
            }
            if let Some(m) = self.get_mut(id) {
                m.snext = None;
                m.sprev = None;
            }
        }

        if !no_block {
            match bprev {
                Some(p) => {
                    if let Some(pm) = self.get_mut(p) {
                        pm.bnext = bnext;
                    }
                }
                None => {
                    if let Some(block) = self.block_of(x, y) {
                        self.set_block_head(block, bnext);
                    }
                }
            }
            if let Some(n) = bnext {
                if let Some(nm) = self.get_mut(n) {
                    nm.bprev = bprev;
                }
            }
            if let Some(m) = self.get_mut(id) {
                m.bnext = None;
                m.bprev = None;
            }
        }
    }

    /// Savegame restore: adopt a snapshot's mobj slots verbatim and relink
    /// every live thing into the sector/blockmap lists (`P_SetThingPosition`
    /// for each, over a fresh arena sized to the current level).
    pub fn restore_slots(
        &mut self,
        slots: Vec<Option<Mobj>>,
        level: &Level,
        render: &Render,
        num_sectors: usize,
    ) {
        *self = Mobjs::new(num_sectors.max(1));
        self.slots = slots;
        self.free.clear();
        let mut live: Vec<MobjId> = Vec::new();
        for (i, s) in self.slots.iter().enumerate() {
            if s.is_some() {
                live.push(i);
            }
        }
        // Slot recycle order matches `alloc` (a free list of the gaps).
        for i in 0..self.slots.len() {
            if self.slots[i].is_none() {
                self.free.push(i);
            }
        }
        for id in live {
            let sector = self.get(id).map(|m| m.sector).unwrap_or(0);
            self.set_thing_position(id, level, render, sector);
        }
    }

    /// `P_SetThingPosition`: links `id` into its subsector/sector and its
    /// blockmap cell based on `(x, y)`. Returns the sector it was linked to.
    pub fn set_thing_position(
        &mut self,
        id: MobjId,
        level: &Level,
        render: &Render,
        sector: usize,
    ) -> usize {
        let (x, y, no_sector, no_block) = match self.get(id) {
            Some(m) => (
                m.x,
                m.y,
                m.flags & MF_NOSECTOR != 0,
                m.flags & MF_NOBLOCKMAP != 0,
            ),
            None => return sector,
        };

        // Subsector from coordinates (`R_PointInSubsector`).
        let subsector = render.point_in_subsector(level, x, y);
        if let Some(m) = self.get_mut(id) {
            m.subsector = subsector;
            m.sector = sector;
        }

        if !no_sector {
            let head = self.sector_head(sector);
            if let Some(m) = self.get_mut(id) {
                m.snext = head;
                m.sprev = None;
            }
            if let Some(h) = head {
                if let Some(hm) = self.get_mut(h) {
                    hm.sprev = Some(id);
                }
            }
            self.set_sector_head(sector, Some(id));
        }

        if !no_block {
            self.sync_blockmap(level);
            if let Some(block) = self.block_of(x, y) {
                let head = self.block_head(block);
                if let Some(m) = self.get_mut(id) {
                    m.bnext = head;
                    m.bprev = None;
                }
                if let Some(h) = head {
                    if let Some(hm) = self.get_mut(h) {
                        hm.bprev = Some(id);
                    }
                }
                self.set_block_head(block, Some(id));
            } else if let Some(m) = self.get_mut(id) {
                m.bnext = None;
                m.bprev = None;
            }
        }

        sector
    }
}

/// `P_SetMobjState` (field updates only, no action invocation).
///
/// The full version that invokes action functions is in `p_tick.rs` as
/// `run_mobj_state`, which needs the monster dispatch (`p_enemy`).
pub fn set_mobj_state(
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    id: MobjId,
    mut state: StateId,
) -> bool {
    loop {
        if state == StateId::S_NULL {
            if let Some(m) = mobjs.get_mut(id) {
                m.state = StateId::S_NULL;
            }
            remove_mobj(mobjs, thinkers, id);
            return false;
        }

        let st = &STATES[state as usize];
        let (tics, sprite, frame, next) = (st.tics, st.sprite, st.frame, st.nextstate);
        match mobjs.get_mut(id) {
            Some(m) => {
                m.state = state;
                m.tics = tics;
                m.sprite = sprite;
                m.frame = frame;
            }
            None => return false,
        }

        if tics != 0 {
            break;
        }
        state = next;
    }
    true
}

/// `P_RemoveMobj`: unlink from the sector list, mark the thinker for lazy
/// removal and release the arena slot.
pub fn remove_mobj(mobjs: &mut Mobjs, thinkers: &mut Thinkers, id: MobjId) {
    let Some(m) = mobjs.get(id) else {
        return;
    };
    let thinker = m.thinker;
    mobjs.unset_thing_position(id);
    thinkers.remove(thinker);
    mobjs.slots[id] = None;
    mobjs.free.push(id);
}

/// `P_SpawnMobj`: create `ty` at `(x, y, z)`, link it into the world and add its
/// thinker. `z` may be [`ONFLOORZ`] or [`ONCEILINGZ`].
///
/// Deferred: `gameskill` gating of `reactiontime` and the `lastlook` RNG draw
/// (needs `P_Random`).
#[allow(clippy::too_many_arguments)]
pub fn spawn_mobj(
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    level: &Level,
    render: &Render,
    ty: MobjType,
    x: Fixed,
    y: Fixed,
    z: Fixed,
) -> MobjId {
    let id = mobjs.alloc(Mobj::from_type(ty, x, y, z));
    let thinker = thinkers.add(ThinkerKind::Mobj(id));
    if let Some(m) = mobjs.get_mut(id) {
        m.thinker = thinker;
    }

    let subsector = render.point_in_subsector(level, x, y);
    let sector = level.subsectors[subsector].sector;
    mobjs.set_thing_position(id, level, render, sector);

    let sec = &level.sectors[sector];
    if let Some(m) = mobjs.get_mut(id) {
        m.floorz = sec.floorheight;
        m.ceilingz = sec.ceilingheight;
        m.z = if z == ONFLOORZ {
            m.floorz
        } else if z == ONCEILINGZ {
            m.ceilingz - m.height
        } else {
            z
        };
    }
    id
}

/// `P_SpawnMapThing`: interpret a map `Thing` and spawn the corresponding
/// mobj. Returns `None` for player/deathmatch starts and filtered things.
///
/// `skill` is `0..=5` (`sk_baby`..`sk_nightmare`, with `-1` for "no items"
/// mapped by the caller); `netgame`/`deathmatch`/`nomonsters` mirror the
/// vanilla globals. Player starts are recorded in `player_starts` and
/// deathmatch starts in `deathmatch_starts`; the caller decides whether to
/// spawn the player mobj (needs the gameplay layer, Ease 3).
#[allow(clippy::too_many_arguments)]
pub fn spawn_map_thing(
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    level: &Level,
    render: &Render,
    rng: &mut Random,
    mthing: &Thing,
    skill: i32,
    netgame: bool,
    deathmatch: bool,
    nomonsters: bool,
    player_starts: &mut [Option<Thing>; 4],
    deathmatch_starts: &mut Vec<Thing>,
    counters: &mut LevelCounters,
) -> Option<MobjId> {
    // Count deathmatch start positions.
    if mthing.ty == 11 {
        if deathmatch_starts.len() < 10 {
            deathmatch_starts.push(*mthing);
        }
        return None;
    }
    if mthing.ty <= 0 {
        // "Player -1 start": accepted and ignored, like vanilla.
        return None;
    }

    if mthing.ty <= 4 {
        player_starts[(mthing.ty - 1) as usize] = Some(*mthing);
        // The player mobj itself is spawned by the game layer (`P_SpawnPlayer`).
        return None;
    }

    // Skill gating: bit 16 = deathmatch-only (dropped in single player), bits
    // 1/2/4 = skill classes.
    if !netgame && mthing.options & 16 != 0 {
        return None;
    }
    let bit = if skill == 0 {
        1
    } else if skill == 4 {
        4
    } else if skill < 0 {
        return None;
    } else {
        1 << (skill - 1)
    };
    if mthing.options & bit == 0 {
        return None;
    }

    let info_ty = mobj_type_for_doomednum(i32::from(mthing.ty))?;

    let info = &MOBJINFO[info_ty as usize];
    if deathmatch && info.flags & MF_NOTDMATCH != 0 {
        return None;
    }
    if nomonsters && (info_ty == MobjType::MT_SKULL || info.flags & MF_COUNTKILL != 0) {
        return None;
    }

    let x = i32::from(mthing.x) << FRAC_BITS;
    let y = i32::from(mthing.y) << FRAC_BITS;
    let z = if info.flags & MF_SPAWNCEILING != 0 {
        ONCEILINGZ
    } else {
        ONFLOORZ
    };

    let id = spawn_mobj(mobjs, thinkers, level, render, info_ty, x, y, z);
    if let Some(m) = mobjs.get_mut(id) {
        m.spawnpoint = *mthing;
        if m.tics > 0 {
            m.tics = 1 + (rng.p.next() as i32 % m.tics);
        }
        if m.flags & MF_COUNTKILL != 0 {
            counters.total_kills += 1;
        }
        if m.flags & MF_COUNTITEM != 0 {
            counters.total_items += 1;
        }
        m.angle = ANG45.wrapping_mul((i32::from(mthing.angle) as u32).wrapping_div(45) & 7);
        if mthing.options & MTF_AMBUSH != 0 {
            m.flags |= MF_AMBUSH;
        }
    }
    Some(id)
}

/// `ITEMQUESIZE` (`p_local.h`).
pub const ITEMQUESIZE: usize = 128;

/// `MTF_AMBUSH` (`doomdef.h`).
pub const MTF_AMBUSH: i16 = 8;

/// Per-level counters bumped by [`spawn_map_thing`] (`totalkills`…).
#[derive(Clone, Copy, Debug, Default)]
pub struct LevelCounters {
    pub total_kills: i32,
    pub total_items: i32,
    pub total_secrets: i32,
}

/// Spawn every thing in `level.things` (`P_LoadThings` placement loop).
/// Returns the recorded player/deathmatch starts and kill/item counters.
///
/// `commercial` mirrors `gamemode == commercial`: Doom II-only monsters are
/// skipped in non-commercial games (and loading stops at the first one, like
/// vanilla's `break`).
#[allow(clippy::too_many_arguments)]
pub fn spawn_map_things(
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    level: &Level,
    render: &Render,
    rng: &mut Random,
    skill: i32,
    netgame: bool,
    deathmatch: bool,
    nomonsters: bool,
    commercial: bool,
) -> SpawnedThings {
    let mut out = SpawnedThings::default();
    for t in &level.things {
        if !commercial {
            match t.ty {
                68 | 64 | 88 | 89 | 69 | 67 | 71 | 65 | 66 | 84 => break,
                _ => {}
            }
        }
        spawn_map_thing(
            mobjs,
            thinkers,
            level,
            render,
            rng,
            t,
            skill,
            netgame,
            deathmatch,
            nomonsters,
            &mut out.player_starts,
            &mut out.deathmatch_starts,
            &mut out.counters,
        );
    }
    out
}

/// `P_InitThinkers` placement call: reset the thinker list.
pub fn init_thinkers(thinkers: &mut Thinkers) {
    thinkers.clear();
}

/// Results of [`spawn_map_things`].
#[derive(Clone, Debug, Default)]
pub struct SpawnedThings {
    pub player_starts: [Option<Thing>; 4],
    pub deathmatch_starts: Vec<Thing>,
    pub counters: LevelCounters,
}

/// Structural parts of `P_SpawnSpecials`: timers and the line-special lists.
/// Sector light/door specials that spawn thinkers (light flash, doors) are
/// deferred to the gameplay layer (Fase 3), matching the README.
#[allow(clippy::too_many_arguments)]
pub fn spawn_specials(level: &Level, deathmatch: bool, timelimit: i32) -> LineSpecials {
    let mut out = LineSpecials::default();

    // -TIMER (deathmatch only).
    if timelimit > 0 && deathmatch {
        out.level_timer = true;
        out.level_time_count = timelimit * 60 * crate::doomdef::TICRATE as i32;
    }

    // Sector specials that only count totals (secret). Thinker-based specials
    // (light flash, strobes, doors) are deferred.
    for sec in &level.sectors {
        if sec.special == 9 {
            out.total_secrets += 1;
        }
    }

    // Line specials: 48 = scrolling wall (EFFECT FIRSTCOL SCROLL+).
    for i in 0..level.lines.len() {
        if level.lines[i].special == 48 && out.scroll_lines.len() < 64 {
            out.scroll_lines.push(i);
        }
    }

    out
}

/// `P_SpawnSpecials` state kept by the port.
#[derive(Clone, Debug, Default)]
pub struct LineSpecials {
    pub scroll_lines: Vec<usize>,
    pub total_secrets: i32,
    pub level_timer: bool,
    pub level_time_count: i32,
}

/// `P_NightmareRespawn`: respawn a dead monster at its spawn spot
/// (nightmare skill; driven from the thinker dispatch).
#[allow(clippy::too_many_arguments)]
pub fn nightmare_respawn(world: &mut super::p_world::World, mobj: MobjId) {
    use crate::p_map::check_position;

    let Some(m) = world.mobjs.get(mobj) else { return };
    let x = (m.spawnpoint.x as i32) << 16;
    let y = (m.spawnpoint.y as i32) << 16;

    // somthing is occupying its position?
    let vid = world.bump_validcount();
    if !check_position(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        mobj,
        x,
        y,
    )
    .is_some()
    {
        return; // no respawn
    }

    // spawn a teleport fog at old spot because of removal of the body
    let (ox, oy, floorz) = {
        let m = world.mobjs.get(mobj).unwrap();
        (m.x, m.y, world.level.sectors[m.sector].floorheight)
    };
    crate::p_mobj::spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        crate::info::MobjType::MT_TFOG,
        ox,
        oy,
        floorz,
    );
    world.play_sfx(crate::sounds::Sfx::sfx_telept);

    // spawn a teleport fog at the new spot
    let subsector = world.render.point_in_subsector(&world.level, x, y);
    let sector = world.level.subsectors[subsector].sector;
    let floorz = world.level.sectors[sector].floorheight;
    crate::p_mobj::spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        crate::info::MobjType::MT_TFOG,
        x,
        y,
        floorz,
    );
    world.play_sfx(crate::sounds::Sfx::sfx_telept);

    // spawn the new monster, inheriting attributes from the deceased one
    let (ty, spawnpoint, ceiling_spawn) = {
        let m = world.mobjs.get(mobj).unwrap();
        (
            m.ty,
            m.spawnpoint,
            m.info().flags & MF_SPAWNCEILING != 0,
        )
    };
    let z = if ceiling_spawn { ONCEILINGZ } else { ONFLOORZ };
    let mo = spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        ty,
        x,
        y,
        z,
    );
    if let Some(m) = world.mobjs.get_mut(mo) {
        m.spawnpoint = spawnpoint;
        m.angle = crate::tables::ANG45.wrapping_mul(spawnpoint.angle as u32 / 45);
        if spawnpoint.options & MTF_AMBUSH != 0 {
            m.flags |= MF_AMBUSH;
        }
        m.reactiontime = 18;
    }

    // remove the old monster
    remove_mobj(&mut world.mobjs, &mut world.thinkers, mobj);
}

/// `P_MobjThinker`: momentum movement, then state cycling.
///
/// Deferred: nightmare respawn, per-state action functions (invoked through the
/// gameplay layer) and `P_SlideMove`/`P_ExplodeMissile` inside the movement.
pub fn mobj_thinker(
    level: &Level,
    render: &Render,
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    line_vc: &mut [i32],
    validcount: i32,
    id: MobjId,
) -> bool {
    let (momx, momy, flags) = match mobjs.get(id) {
        Some(m) => (m.momx, m.momy, m.flags),
        None => return false,
    };

    if momx != 0 || momy != 0 || flags & MF_SKULLFLY != 0 {
        xy_movement(level, render, mobjs, thinkers, line_vc, validcount, id);
        if !mobjs.is_live(id) {
            return false;
        }
    }

    let (z, momz, floorz) = match mobjs.get(id) {
        Some(m) => (m.z, m.momz, m.floorz),
        None => return false,
    };
    if z != floorz || momz != 0 {
        z_movement(mobjs, id);
        if !mobjs.is_live(id) {
            return false;
        }
    }

    // Cycle through states.
    let tics = match mobjs.get(id) {
        Some(m) => m.tics,
        None => return false,
    };
    if tics != -1 {
        if let Some(m) = mobjs.get_mut(id) {
            m.tics -= 1;
        }
        if mobjs.get(id).map(|m| m.tics) == Some(0) {
            let next = STATES[mobjs.get(id).unwrap().state as usize].nextstate;
            return set_mobj_state(mobjs, thinkers, id, next);
        }
        true
    } else {
        // Nightmare respawn is deferred.
        true
    }
}

/// `P_XYMovement` (partial): momentum, blocking and friction.
///
/// Deferred: `P_SlideMove` for players, `P_ExplodeMissile` for missiles, the
/// sky-flat missile hack, player cheat/command handling and the walking-frame
/// state reset.
pub fn xy_movement(
    level: &Level,
    render: &Render,
    mobjs: &mut Mobjs,
    thinkers: &mut Thinkers,
    line_vc: &mut [i32],
    validcount: i32,
    id: MobjId,
) {
    let (mut momx, mut momy, mut flags) = match mobjs.get(id) {
        Some(m) => (m.momx, m.momy, m.flags),
        None => return,
    };

    if momx == 0 && momy == 0 {
        if flags & MF_SKULLFLY != 0 {
            if let Some(m) = mobjs.get_mut(id) {
                m.flags &= !MF_SKULLFLY;
                m.momx = 0;
                m.momy = 0;
                m.momz = 0;
            }
            let spawn = mobjs.get(id).unwrap().info().spawnstate;
            set_mobj_state(mobjs, thinkers, id, spawn);
        }
        return;
    }

    momx = momx.clamp(-MAXMOVE, MAXMOVE);
    momy = momy.clamp(-MAXMOVE, MAXMOVE);
    if let Some(m) = mobjs.get_mut(id) {
        m.momx = momx;
        m.momy = momy;
    }
    flags = mobjs.get(id).map(|m| m.flags).unwrap_or(flags);

    let mut xmove = momx;
    let mut ymove = momy;
    loop {
        let (x, y) = mobjs.get(id).map(|m| (m.x, m.y)).unwrap_or((0, 0));
        let (ptryx, ptryy);
        if xmove > MAXMOVE / 2 || ymove > MAXMOVE / 2 {
            ptryx = x + xmove / 2;
            ptryy = y + ymove / 2;
            xmove >>= 1;
            ymove >>= 1;
        } else {
            ptryx = x + xmove;
            ptryy = y + ymove;
            xmove = 0;
            ymove = 0;
        }

        if !try_move(level, render, mobjs, line_vc, validcount, id, ptryx, ptryy) {
            // Blocked: try to slide for players, explode missiles on impact.
            let is_player = mobjs.get(id).and_then(|m| m.player).is_some();
            let is_missile = flags & MF_MISSILE != 0;
            if is_player {
                slide_move(level, render, mobjs, line_vc, validcount, id);
            } else if is_missile {
                // Try to explode; if it hit sky in a hacky case we'd remove, but that's rare.
                if let Some(m) = mobjs.get_mut(id) {
                    m.momx = 0;
                    m.momy = 0;
                }
                let deathstate = mobjs.get(id).map(|m| m.info().deathstate).unwrap_or(StateId::S_NULL);
                set_mobj_state(mobjs, thinkers, id, deathstate);
            } else {
                if let Some(m) = mobjs.get_mut(id) {
                    m.momx = 0;
                    m.momy = 0;
                }
            }
        }

        if xmove == 0 && ymove == 0 {
            break;
        }
    }

    // Slow down. Player cheats/commands are deferred; this assumes no input.
    if flags & (MF_MISSILE | MF_SKULLFLY) != 0 {
        return;
    }

    let (z, floorz) = mobjs.get(id).map(|m| (m.z, m.floorz)).unwrap_or((0, 0));
    if z > floorz {
        return; // no friction when airborne
    }

    let m = mobjs.get(id).unwrap();
    if m.flags & MF_CORPSE != 0
        && (m.momx > FRAC_UNIT / 4
            || m.momx < -FRAC_UNIT / 4
            || m.momy > FRAC_UNIT / 4
            || m.momy < -FRAC_UNIT / 4)
    {
        let sec_floor = level.sectors[level.subsectors[m.subsector].sector].floorheight;
        if m.floorz != sec_floor {
            return; // halfway off a step
        }
    }

    let (momx, momy) = (m.momx, m.momy);
    if momx > -STOPSPEED && momx < STOPSPEED && momy > -STOPSPEED && momy < STOPSPEED {
        if let Some(m) = mobjs.get_mut(id) {
            m.momx = 0;
            m.momy = 0;
        }
    } else if let Some(m) = mobjs.get_mut(id) {
        m.momx = fixed_mul(momx, FRICTION);
        m.momy = fixed_mul(momy, FRICTION);
    }
}

/// `P_ZMovement` (partial): gravity, floating and floor/ceiling clipping.
///
/// Deferred: player viewheight/squat effects and sounds, `P_ExplodeMissile`,
/// and the version-dependent lost-soul floor bounce.
pub fn z_movement(mobjs: &mut Mobjs, id: MobjId) {
    let Some(m) = mobjs.get(id) else {
        return;
    };
    let (x, y, height, floorz, ceilingz, target, flags) = (
        m.x, m.y, m.height, m.floorz, m.ceilingz, m.target, m.flags,
    );
    let mut z = m.z;
    let mut momz = m.momz;

    z += momz;

    if flags & MF_FLOAT != 0 {
        if let Some(t) = target {
            if flags & (MF_SKULLFLY | MF_INFLOAT) == 0 {
                if let Some(tm) = mobjs.get(t) {
                    let dist = aprox_distance(x - tm.x, y - tm.y);
                    let delta = (tm.z + (height >> 1)) - z;
                    if delta < 0 && dist < -(delta * 3) {
                        z -= FLOATSPEED;
                    } else if delta > 0 && dist < delta * 3 {
                        z += FLOATSPEED;
                    }
                }
            }
        }
    }

    if z <= floorz {
        // Hit the floor.
        if momz < 0 {
            momz = 0;
        }
        z = floorz;
        // Deferred: P_ExplodeMissile for MF_MISSILE.
    } else if flags & MF_NOGRAVITY == 0 {
        if momz == 0 {
            momz = -(GRAVITY * 2);
        } else {
            momz -= GRAVITY;
        }
    }

    if z + height > ceilingz {
        // Hit the ceiling.
        if momz > 0 {
            momz = 0;
        }
        z = ceilingz - height;
        if flags & MF_SKULLFLY != 0 {
            momz = -momz;
        }
        // Deferred: P_ExplodeMissile for MF_MISSILE.
    }

    if let Some(m) = mobjs.get_mut(id) {
        m.z = z;
        m.momz = momz;
    }
}

/// `P_RespawnSpecials`: deathmatch item respawn (called from the game tic).
pub fn p_respawn_specials(world: &mut super::p_world::World) {
    // only respawn items in deathmatch
    if world.deathmatch != 2 {
        return;
    }

    // wait at least 30 seconds
    let due = world
        .item_respawn_queue
        .first()
        .map(|&(_, t)| world.leveltime - t >= 30 * crate::doomdef::TICRATE as i32)
        .unwrap_or(false);
    if !due {
        return;
    }
    let (mthing, _) = world.item_respawn_queue.remove(0);

    let x = (mthing.x as i32) << 16;
    let y = (mthing.y as i32) << 16;

    // spawn a teleport fog at the new spot
    let subsector = world.render.point_in_subsector(&world.level, x, y);
    let sector = world.level.subsectors[subsector].sector;
    let floorz = world.level.sectors[sector].floorheight;
    let fog = spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        MobjType::MT_IFOG,
        x,
        y,
        floorz,
    );
    let _ = fog;
    world.play_sfx(crate::sounds::Sfx::sfx_itmbk);

    // find which type to spawn
    let ty = crate::info::MOBJINFO
        .iter()
        .position(|i| i.doomednum == mthing.ty as i32)
        .map(|i| MobjType::from_index(i as i32))
        .unwrap_or(MobjType::MT_MISC13);

    // spawn it
    let ceiling_spawn = crate::info::MOBJINFO[ty as usize].flags & MF_SPAWNCEILING != 0;
    let z = if ceiling_spawn { ONCEILINGZ } else { ONFLOORZ };
    let mo = spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        ty,
        x,
        y,
        z,
    );
    if let Some(m) = world.mobjs.get_mut(mo) {
        m.spawnpoint = mthing;
        m.angle = crate::tables::ANG45.wrapping_mul(mthing.angle as u32 / 45);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Level, Subsector};
    use crate::p_tick::ThinkerKind;

    fn empty_level(sectors: usize) -> Level {
        Level {
            vertexes: Vec::new(),
            sectors: (0..sectors)
                .map(|_| crate::level::Sector::default())
                .collect(),
            sides: Vec::new(),
            lines: Vec::new(),
            subsectors: vec![Subsector {
                sector: 0,
                numlines: 0,
                firstline: 0,
            }],
            nodes: Vec::new(),
            segs: Vec::new(),
            blockmap: Default::default(),
            reject: Vec::new(),
            things: Vec::new(),
        }
    }

    fn level_with_blockmap() -> Level {
        let mut level = empty_level(4);
        // Header only: origin (0,0), 2x2 cells. Thing linking does not need
        // the per-cell line lists.
        level.blockmap.data = vec![0, 0, 2, 2];
        level
    }

    #[test]
    fn blockmap_links_are_lifo_and_unlink() {
        let level = level_with_blockmap();
        let render = Render::new();
        let mut mobjs = Mobjs::new(4);

        // Block (0,0) covers x/y in [0, MAPBLOCKSIZE); block (1,0) starts at
        // x = 1 << MAPBLOCKSHIFT.
        let a = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 0));
        let b = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 0));
        let c = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 1 << 23, 0, 0));
        mobjs.set_thing_position(a, &level, &render, 0);
        mobjs.set_thing_position(b, &level, &render, 0);
        mobjs.set_thing_position(c, &level, &render, 1);

        assert_eq!(mobjs.block_head(0), Some(b));
        assert_eq!(mobjs.get(b).unwrap().bnext, Some(a));
        assert_eq!(mobjs.get(a).unwrap().bprev, Some(b));
        assert_eq!(mobjs.block_head(1), Some(c));

        // Unlink the head; a becomes the head of block 0.
        mobjs.unset_thing_position(b);
        assert_eq!(mobjs.block_head(0), Some(a));
        assert_eq!(mobjs.get(a).unwrap().bprev, None);
        assert_eq!(mobjs.get(b).unwrap().bnext, None);

        // Unlink the tail directly.
        mobjs.unset_thing_position(a);
        assert_eq!(mobjs.block_head(0), None);
    }

    #[test]
    fn sector_links_are_lifo() {
        let level = empty_level(1);
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);

        let a = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 0));
        let b = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 0));
        mobjs.set_thing_position(a, &level, &render, 0);
        mobjs.set_thing_position(b, &level, &render, 0);

        assert_eq!(mobjs.sector_head(0), Some(b));
        assert_eq!(mobjs.get(b).unwrap().snext, Some(a));
        assert_eq!(mobjs.get(a).unwrap().sprev, Some(b));
        assert_eq!(mobjs.get(a).unwrap().snext, None);

        // Unlink the head; a becomes the head.
        mobjs.unset_thing_position(b);
        assert_eq!(mobjs.sector_head(0), Some(a));
        assert_eq!(mobjs.get(a).unwrap().sprev, None);
    }

    #[test]
    fn remove_mobj_unlinks_and_frees() {
        let level = empty_level(1);
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();

        let a = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 0));
        let tid = thinkers.add(ThinkerKind::Mobj(a));
        mobjs.get_mut(a).unwrap().thinker = tid;
        mobjs.set_thing_position(a, &level, &render, 0);

        remove_mobj(&mut mobjs, &mut thinkers, a);

        assert!(!mobjs.is_live(a));
        assert_eq!(mobjs.sector_head(0), None);
        assert!(thinkers.is_removed(tid));
    }

    #[test]
    fn set_state_follows_zero_tic_chain() {
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();
        let a = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 0));
        let tid = thinkers.add(ThinkerKind::Mobj(a));
        mobjs.get_mut(a).unwrap().thinker = tid;

        // S_CHAIN3 has duration 0 and jumps to S_CHAIN (duration > 0), so the
        // chain must collapse onto S_CHAIN. Guard the data assumption first.
        let from = StateId::S_CHAIN3;
        let to = StateId::S_CHAIN;
        assert_eq!(STATES[from as usize].tics, 0);
        assert_eq!(STATES[from as usize].nextstate, to);
        assert_ne!(STATES[to as usize].tics, 0);

        assert!(set_mobj_state(&mut mobjs, &mut thinkers, a, from));
        let m = mobjs.get(a).unwrap();
        assert_eq!(m.state, to);
        assert_eq!(m.tics, STATES[to as usize].tics);
    }

    #[test]
    fn set_state_null_removes() {
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();
        let a = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 0));
        let tid = thinkers.add(ThinkerKind::Mobj(a));
        mobjs.get_mut(a).unwrap().thinker = tid;

        assert!(!set_mobj_state(
            &mut mobjs,
            &mut thinkers,
            a,
            StateId::S_NULL
        ));
        assert!(!mobjs.is_live(a));
        assert!(thinkers.is_removed(tid));
    }

    #[test]
    fn spawn_places_on_floor_and_links() {
        let mut level = empty_level(1);
        level.sectors[0].floorheight = 16 << 16;
        level.sectors[0].ceilingheight = 96 << 16;
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();

        let id = spawn_mobj(
            &mut mobjs,
            &mut thinkers,
            &level,
            &render,
            MobjType::MT_PLAYER,
            0,
            0,
            ONFLOORZ,
        );

        let m = mobjs.get(id).unwrap();
        assert_eq!(m.floorz, 16 << 16);
        assert_eq!(m.ceilingz, 96 << 16);
        assert_eq!(m.z, 16 << 16);
        assert_eq!(mobjs.sector_head(0), Some(id));
        let head = thinkers.head().unwrap();
        assert_eq!(thinkers.kind(head), Some(ThinkerKind::Mobj(id)));
    }

    #[test]
    fn spawn_on_ceiling_subtracts_height() {
        let mut level = empty_level(1);
        level.sectors[0].floorheight = 0;
        level.sectors[0].ceilingheight = 100 << 16;
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();

        let id = spawn_mobj(
            &mut mobjs,
            &mut thinkers,
            &level,
            &render,
            MobjType::MT_PLAYER,
            0,
            0,
            ONCEILINGZ,
        );
        let m = mobjs.get(id).unwrap();
        assert_eq!(m.z, (100 << 16) - m.height);
    }

    #[test]
    fn z_movement_applies_gravity_and_lands() {
        let mut mobjs = Mobjs::new(1);
        let id = mobjs.alloc(Mobj::from_type(MobjType::MT_PLAYER, 0, 0, 80 << 16));
        mobjs.get_mut(id).unwrap().floorz = 0;
        mobjs.get_mut(id).unwrap().ceilingz = 1000 << 16;

        z_movement(&mut mobjs, id);
        assert_eq!(mobjs.get(id).unwrap().momz, -(FRAC_UNIT * 2));

        for _ in 0..1000 {
            z_movement(&mut mobjs, id);
        }
        let m = mobjs.get(id).unwrap();
        assert_eq!(m.z, 0);
        assert_eq!(m.momz, 0);
    }

    #[test]
    fn mobj_thinker_decrements_tics() {
        let mut level = empty_level(1);
        level.sectors[0].floorheight = 0;
        level.sectors[0].ceilingheight = 128 << 16;
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();
        let mut line_vc = vec![0i32; level.lines.len().max(1)];

        let id = spawn_mobj(
            &mut mobjs,
            &mut thinkers,
            &level,
            &render,
            MobjType::MT_PLAYER,
            0,
            0,
            ONFLOORZ,
        );
        mobjs.get_mut(id).unwrap().tics = 2;
        let state = mobjs.get(id).unwrap().state;

        assert!(mobj_thinker(
            &level,
            &render,
            &mut mobjs,
            &mut thinkers,
            &mut line_vc,
            1,
            id
        ));
        let m = mobjs.get(id).unwrap();
        assert_eq!(m.tics, 1);
        assert_eq!(m.state, state);
    }

    #[test]
    fn spawn_map_thing_spawns_monster_and_counts_kill() {
        let mut level = empty_level(1);
        level.sectors[0].floorheight = 0;
        level.sectors[0].ceilingheight = 200 << 16;
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();
        let mut rng = Random::new();

        // Sergeant (POSS) is doomednum 9; flags include MF_COUNTKILL.
        let t = Thing {
            x: 32,
            y: 48,
            angle: 90,
            ty: 9,
            options: 7, // all skills
        };

        let mut player_starts: [Option<Thing>; 4] = [None; 4];
        let mut dm_starts = Vec::new();
        let mut counters = LevelCounters::default();
        let id = spawn_map_thing(
            &mut mobjs,
            &mut thinkers,
            &level,
            &render,
            &mut rng,
            &t,
            1,
            false,
            false,
            false,
            &mut player_starts,
            &mut dm_starts,
            &mut counters,
        );
        let id = id.expect("should spawn");
        let m = mobjs.get(id).unwrap();
        assert_eq!(m.info().doomednum, 9);
        assert_eq!(m.spawnpoint.ty, 9);
        assert_eq!(m.angle, ANG45 * 2);
        assert!(m.tics >= 1);
        assert_eq!(counters.total_kills, 1);
        assert!(player_starts.iter().all(Option::is_none));
    }

    #[test]
    fn spawn_map_thing_records_player_and_dm_starts() {
        let mut level = empty_level(1);
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();
        let mut rng = Random::new();
        level.things = vec![
            Thing { x: 1, y: 2, angle: 90, ty: 1, options: 7 },
            Thing { x: 3, y: 4, angle: 180, ty: 11, options: 7 },
        ];
        let out = spawn_map_things(
            &mut mobjs,
            &mut thinkers,
            &level,
            &render,
            &mut rng,
            1,
            false,
            false,
            false,
            true,
        );
        assert_eq!(out.player_starts[0].map(|t| t.ty), Some(1));
        assert_eq!(out.deathmatch_starts.len(), 1);
        assert!(mobjs.is_empty());
    }

    #[test]
    fn spawn_map_thing_skill_gating() {
        let mut level = empty_level(1);
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();
        let mut rng = Random::new();

        // options=2 = "medium only". bit = 1<<(skill-1) -> spawns at skill 2.
        level.things.push(Thing { x: 0, y: 0, angle: 0, ty: 9, options: 2 });
        for skill in [1, 2, 3] {
            let before = mobjs.len();
            spawn_map_things(
                &mut mobjs, &mut thinkers, &level, &render, &mut rng,
                skill, false, false, false, true,
            );
            let spawned = mobjs.len() > before;
            if spawned {
                // remove so the next iteration starts fresh
                let ids: Vec<_> = mobjs.iter_ids().collect();
                for id in ids {
                    remove_mobj(&mut mobjs, &mut thinkers, id);
                }
            }
            assert_eq!(spawned, skill == 2, "skill {skill}");
        }
    }

    #[test]
    fn spawn_map_thing_non_commercial_skips_doom2() {
        let mut level = empty_level(1);
        let render = Render::new();
        let mut mobjs = Mobjs::new(1);
        let mut thinkers = Thinkers::new();
        let mut rng = Random::new();

        // 64 = Archvile (Doom II only); spawn must stop before it.
        level.things.push(Thing { x: 0, y: 0, angle: 0, ty: 64, options: 7 });
        level.things.push(Thing { x: 0, y: 0, angle: 0, ty: 9, options: 7 });
        let out = spawn_map_things(
            &mut mobjs, &mut thinkers, &level, &render, &mut rng,
            1, false, false, false, false,
        );
        assert!(mobjs.is_empty());
        assert_eq!(out.counters.total_kills, 0);
    }

    #[test]
    fn spawn_specials_counts_secrets_and_scrollers() {
        let mut level = empty_level(2);
        level.sectors[0].special = 9;
        level.lines.push(crate::level::Line {
            v1: 0,
            v2: 0,
            special: 48,
            ..Default::default()
        });
        let sp = spawn_specials(&level, true, 5);
        assert_eq!(sp.total_secrets, 1);
        assert_eq!(sp.scroll_lines, vec![0]);
        assert!(sp.level_timer);
        assert_eq!(sp.level_time_count, 5 * 60 * 35);

        let sp = spawn_specials(&level, false, 5);
        assert!(!sp.level_timer);
    }

    #[test]
    fn init_thinkers_clears() {
        let mut thinkers = Thinkers::new();
        thinkers.add(ThinkerKind::Mobj(0));
        init_thinkers(&mut thinkers);
        assert!(thinkers.head().is_none());
    }
}
