//! Savegames, the port-native snapshot format.
//!
//! Vanilla `G_DoSaveGame` archives the dynamic world over a freshly loaded
//! level; the port does the same with a compact binary blob: geometry is
//! NOT stored (the host reloads the same map lump), everything that moves
//! is. The byte buffers travel through the host's `FileSystem` seam.
//!
//! Format (`RDSV` v1): a fixed magic, then each subsystem in the order the
//! writer emits it below. Enums are stored as raw discriminants so old
//! snapshots break loudly on incompatible builds (the version bumps).

use alloc::vec::Vec;

use crate::d_player::{Player, PlayerState, Psprite};
use crate::d_ticcmd::TicCmd;
use crate::doomdef::WeaponType;
use crate::info::{MobjType, Sprite, StateId};
use crate::level::Thing;
use crate::p_mobj::Mobj;
use crate::p_world::World;

/// Snapshot magic + version.
const MAGIC: &[u8; 8] = b"RDSV\0\x01\0\0";

/// Save/load failure modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveError {
    Truncated,
    BadMagic,
    BadVersion,
}

// ---------------------------------------------------------------------------
// Byte writer/reader
// ---------------------------------------------------------------------------

struct Wtr {
    out: Vec<u8>,
}

impl Wtr {
    fn new() -> Self {
        Wtr { out: Vec::new() }
    }
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn b(&mut self, v: bool) {
        self.out.push(v as u8);
    }
    fn i16(&mut self, v: i16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn usize(&mut self, v: usize) {
        self.u32(v as u32);
    }
    fn opt_id(&mut self, v: Option<usize>) {
        match v {
            Some(id) => {
                self.b(true);
                self.usize(id);
            }
            None => self.b(false),
        }
    }
}

struct Rdr<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Rdr<'a> {
    fn new(data: &'a [u8]) -> Self {
        Rdr { data, pos: 0 }
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], SaveError> {
        if self.pos + n > self.data.len() {
            return Err(SaveError::Truncated);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, SaveError> {
        Ok(self.bytes(1)?[0])
    }
    fn b(&mut self) -> Result<bool, SaveError> {
        Ok(self.u8()? != 0)
    }
    fn i16(&mut self) -> Result<i16, SaveError> {
        let b = self.bytes(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Result<i32, SaveError> {
        let b = self.bytes(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u32(&mut self) -> Result<u32, SaveError> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn usize(&mut self) -> Result<usize, SaveError> {
        Ok(self.u32()? as usize)
    }
    fn opt_id(&mut self) -> Result<Option<usize>, SaveError> {
        if self.b()? {
            Ok(Some(self.usize()?))
        } else {
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Subsystem serializers
// ---------------------------------------------------------------------------

fn save_thing(w: &mut Wtr, t: &Thing) {
    w.i16(t.x);
    w.i16(t.y);
    w.i16(t.angle);
    w.i16(t.ty);
    w.i16(t.options);
}

fn load_thing(r: &mut Rdr) -> Result<Thing, SaveError> {
    Ok(Thing {
        x: r.i16()?,
        y: r.i16()?,
        angle: r.i16()?,
        ty: r.i16()?,
        options: r.i16()?,
    })
}

fn save_ticcmd(w: &mut Wtr, c: &TicCmd) {
    w.u8(c.forwardmove as u8);
    w.u8(c.sidemove as u8);
    w.i16(c.angleturn);
    w.u8(c.chatchar);
    w.u8(c.buttons);
    w.u8(c.consistancy);
}

fn load_ticcmd(r: &mut Rdr) -> Result<TicCmd, SaveError> {
    let forwardmove = r.u8()? as i8;
    let sidemove = r.u8()? as i8;
    let angleturn = r.i16()?;
    let chatchar = r.u8()?;
    let buttons = r.u8()?;
    let consistancy = r.u8()?;
    Ok(TicCmd {
        forwardmove,
        sidemove,
        angleturn,
        chatchar,
        buttons,
        consistancy,
        ..TicCmd::default()
    })
}

fn save_psprite(w: &mut Wtr, p: &Psprite) {
    w.opt_id(p.state.map(|s| s as usize));
    w.i32(p.tics);
    w.i32(p.sx);
    w.i32(p.sy);
}

fn load_psprite(r: &mut Rdr) -> Result<Psprite, SaveError> {
    let state = r.opt_id()?.map(|i| StateId::from_index(i as i32));
    Ok(Psprite {
        state,
        tics: r.i32()?,
        sx: r.i32()?,
        sy: r.i32()?,
    })
}

fn save_player(w: &mut Wtr, p: &Player) {
    w.opt_id(p.mo);
    w.u8(match p.playerstate {
        PlayerState::Live => 0,
        PlayerState::Dead => 1,
        PlayerState::Reborn => 2,
    });
    save_ticcmd(w, &p.cmd);
    w.i32(p.viewz);
    w.i32(p.viewheight);
    w.i32(p.deltaviewheight);
    w.i32(p.bob);
    w.i32(p.health);
    w.i32(p.armorpoints);
    w.i32(p.armortype);
    for pw in p.powers.iter() {
        w.i32(*pw);
    }
    for c in p.cards.iter() {
        w.b(*c);
    }
    w.b(p.backpack);
    for f in p.frags.iter() {
        w.i32(*f);
    }
    w.u8(p.readyweapon as u8);
    w.opt_id(p.pendingweapon.map(|x| x as usize));
    for wo in p.weaponowned.iter() {
        w.b(*wo);
    }
    for a in p.ammo.iter() {
        w.i32(*a);
    }
    for a in p.maxammo.iter() {
        w.i32(*a);
    }
    w.b(p.attackdown);
    w.b(p.usedown);
    w.u32(p.cheats);
    w.i32(p.refire);
    w.i32(p.killcount);
    w.i32(p.itemcount);
    w.i32(p.secretcount);
    w.i32(p.damagecount);
    w.i32(p.bonuscount);
    w.opt_id(p.attacker);
    w.i32(p.extralight);
    w.i32(p.fixedcolormap);
    w.i32(p.colormap);
    for psp in p.psprites.iter() {
        save_psprite(w, psp);
    }
    w.b(p.didsecret);
    w.b(p.message.is_some());
}

fn load_player(r: &mut Rdr) -> Result<Player, SaveError> {
    let mut p = Player::new();
    p.mo = r.opt_id()?;
    p.playerstate = match r.u8()? {
        0 => PlayerState::Live,
        1 => PlayerState::Dead,
        _ => PlayerState::Reborn,
    };
    p.cmd = load_ticcmd(r)?;
    p.viewz = r.i32()?;
    p.viewheight = r.i32()?;
    p.deltaviewheight = r.i32()?;
    p.bob = r.i32()?;
    p.health = r.i32()?;
    p.armorpoints = r.i32()?;
    p.armortype = r.i32()?;
    for pw in p.powers.iter_mut() {
        *pw = r.i32()?;
    }
    for c in p.cards.iter_mut() {
        *c = r.b()?;
    }
    p.backpack = r.b()?;
    for f in p.frags.iter_mut() {
        *f = r.i32()?;
    }
    p.readyweapon = WeaponType::from_index(r.u8()? as i32);
    p.pendingweapon = r.opt_id()?.map(|i| WeaponType::from_index(i as i32));
    for wo in p.weaponowned.iter_mut() {
        *wo = r.b()?;
    }
    for a in p.ammo.iter_mut() {
        *a = r.i32()?;
    }
    for a in p.maxammo.iter_mut() {
        *a = r.i32()?;
    }
    p.attackdown = r.b()?;
    p.usedown = r.b()?;
    p.cheats = r.u32()?;
    p.refire = r.i32()?;
    p.killcount = r.i32()?;
    p.itemcount = r.i32()?;
    p.secretcount = r.i32()?;
    p.damagecount = r.i32()?;
    p.bonuscount = r.i32()?;
    p.attacker = r.opt_id()?;
    p.extralight = r.i32()?;
    p.fixedcolormap = r.i32()?;
    p.colormap = r.i32()?;
    for psp in p.psprites.iter_mut() {
        *psp = load_psprite(r)?;
    }
    p.didsecret = r.b()?;
    let _has_message = r.b()?;
    Ok(p)
}

fn save_mobj(w: &mut Wtr, m: &Mobj) {
    w.usize(m.thinker);
    w.i32(m.x);
    w.i32(m.y);
    w.i32(m.z);
    w.opt_id(m.snext);
    w.opt_id(m.sprev);
    w.u32(m.angle);
    w.out.extend_from_slice(&(m.sprite as u16).to_le_bytes());
    w.i32(m.frame);
    w.opt_id(m.bnext);
    w.opt_id(m.bprev);
    w.usize(m.subsector);
    w.usize(m.sector);
    w.i32(m.floorz);
    w.i32(m.ceilingz);
    w.i32(m.radius);
    w.i32(m.height);
    w.i32(m.momx);
    w.i32(m.momy);
    w.i32(m.momz);
    w.i32(m.validcount);
    w.u8(m.ty as u8);
    w.i32(m.tics);
    w.i32(m.state as i32);
    w.u32(m.flags);
    w.i32(m.health);
    w.i32(m.movedir);
    w.i32(m.movecount);
    w.opt_id(m.target);
    w.i32(m.reactiontime);
    w.i32(m.threshold);
    w.opt_id(m.player);
    w.i32(m.lastlook);
    save_thing(w, &m.spawnpoint);
    w.opt_id(m.tracer);
}

fn load_mobj(r: &mut Rdr) -> Result<Mobj, SaveError> {
    let thinker = r.usize()?;
    let x = r.i32()?;
    let y = r.i32()?;
    let z = r.i32()?;
    let snext = r.opt_id()?;
    let sprev = r.opt_id()?;
    let angle = r.u32()?;
    let sprite_raw = r.i16()?;
    let frame = r.i32()?;
    let bnext = r.opt_id()?;
    let bprev = r.opt_id()?;
    let subsector = r.usize()?;
    let sector = r.usize()?;
    let floorz = r.i32()?;
    let ceilingz = r.i32()?;
    let radius = r.i32()?;
    let height = r.i32()?;
    let momx = r.i32()?;
    let momy = r.i32()?;
    let momz = r.i32()?;
    let validcount = r.i32()?;
    let ty_raw = r.u8()?;
    let tics = r.i32()?;
    let state_raw = r.i32()?;
    let flags = r.u32()?;
    let health = r.i32()?;
    let movedir = r.i32()?;
    let movecount = r.i32()?;
    let target = r.opt_id()?;
    let reactiontime = r.i32()?;
    let threshold = r.i32()?;
    let player = r.opt_id()?;
    let lastlook = r.i32()?;
    let spawnpoint = load_thing(r)?;
    let tracer = r.opt_id()?;

    let ty = if (ty_raw as usize) < crate::info::NUMMOBJTYPES {
        MobjType::from_index(ty_raw as i32)
    } else {
        MobjType::MT_MISC13
    };
    let mut m = Mobj::from_type(ty, x, y, z);
    m.thinker = thinker;
    m.snext = snext;
    m.sprev = sprev;
    m.angle = angle;
    m.sprite = Sprite::from_index(sprite_raw as i32);
    m.frame = frame;
    m.bnext = bnext;
    m.bprev = bprev;
    m.subsector = subsector;
    m.sector = sector;
    m.floorz = floorz;
    m.ceilingz = ceilingz;
    m.radius = radius;
    m.height = height;
    m.momx = momx;
    m.momy = momy;
    m.momz = momz;
    m.validcount = validcount;
    m.tics = tics;
    m.state = if (0..crate::info::NUMSTATES as i32).contains(&state_raw) {
        StateId::from_index(state_raw)
    } else {
        StateId::S_NULL
    };
    m.flags = flags;
    m.health = health;
    m.movedir = movedir;
    m.movecount = movecount;
    m.target = target;
    m.reactiontime = reactiontime;
    m.threshold = threshold;
    m.player = player;
    m.lastlook = lastlook;
    m.spawnpoint = spawnpoint;
    m.tracer = tracer;
    Ok(m)
}

// ---------------------------------------------------------------------------
// World snapshot
// ---------------------------------------------------------------------------

/// `G_DoSaveGame` core: snapshot every dynamic bit of the world.
pub fn save(world: &World) -> Vec<u8> {
    let mut w = Wtr::new();
    w.out.extend_from_slice(MAGIC);

    // game variables
    w.i32(world.leveltime);
    w.i32(world.validcount);
    w.i32(world.totalkills);
    w.i32(world.totalitems);
    w.i32(world.totalsecret);
    w.b(world.nomonsters);
    w.b(world.respawnmonsters);
    w.b(world.commercial);
    w.b(world.shareware);
    w.i32(world.skill);
    w.b(world.netgame);
    w.i32(world.deathmatch);
    w.usize(world.consoleplayer);
    w.b(world.fastparm);
    w.i32(world.gameepisode);
    w.i32(world.gamemap);
    w.i32(world.timelimit);
    w.opt_id(world.pending_exit.map(|s| s as usize));
    w.b(world.pending_reborn_reload);
    w.u8(world.rng.m.state());
    w.u8(world.rng.p.state());

    // players
    for ingame in world.player_in_game.iter() {
        w.b(*ingame);
    }
    for p in world.players.iter() {
        save_player(&mut w, p);
    }

    // level dynamic state
    w.usize(world.level.sectors.len());
    for s in world.level.sectors.iter() {
        w.i32(s.floorheight);
        w.i32(s.ceilingheight);
        w.i16(s.lightlevel);
        w.i16(s.special);
        w.opt_id(s.specialdata);
        w.i32(s.soundtraversed);
        w.opt_id(s.soundtarget);
        w.i32(s.validcount);
    }
    w.usize(world.level.lines.len());
    for l in world.level.lines.iter() {
        w.i16(l.special);
    }
    w.usize(world.level.sides.len());
    for s in world.level.sides.iter() {
        w.i32(s.textureoffset);
    }

    // movers think from the mobj state, so mobjs come next
    w.usize(world.mobjs.len());
    for i in 0..world.mobjs.len() {
        let live = world.mobjs.get(i).is_some();
        w.b(live);
        if live {
            save_mobj(&mut w, world.mobjs.get(i).unwrap());
        }
    }

    // thinkers
    let thinkers = world.thinkers.snapshot();
    w.usize(thinkers.len());
    for (id, kind) in &thinkers {
        w.usize(*id);
        w.u8(match kind {
            crate::p_tick::ThinkerKind::Mobj(_) => 0,
            crate::p_tick::ThinkerKind::Door(_) => 1,
            crate::p_tick::ThinkerKind::Ceiling(_) => 2,
            crate::p_tick::ThinkerKind::Plat(_) => 3,
            crate::p_tick::ThinkerKind::Floor(_) => 4,
            crate::p_tick::ThinkerKind::Light(_) => 5,
            crate::p_tick::ThinkerKind::Other(_) => 6,
        });
        w.u8(match kind {
            crate::p_tick::ThinkerKind::Mobj(m) => *m as u8,
            crate::p_tick::ThinkerKind::Door(d) => *d as u8,
            crate::p_tick::ThinkerKind::Ceiling(c) => *c as u8,
            crate::p_tick::ThinkerKind::Plat(p) => *p as u8,
            crate::p_tick::ThinkerKind::Floor(f) => *f as u8,
            crate::p_tick::ThinkerKind::Light(l) => *l as u8,
            crate::p_tick::ThinkerKind::Other(t) => *t as u8,
        });
    }

    save_specials(&mut w, world);

    // misc queues
    w.usize(world.bodyque.len());
    for b in world.bodyque.iter() {
        w.opt_id(*b);
    }
    w.usize(world.item_respawn_queue.len());
    for (t, time) in world.item_respawn_queue.iter() {
        save_thing(&mut w, t);
        w.i32(*time);
    }
    w.usize(world.brain_targets.len());
    for t in world.brain_targets.iter() {
        w.opt_id(Some(*t));
    }
    w.usize(world.brain_target_on);
    w.b(world.brain_easy_toggle);

    // line specials bookkeeping
    w.b(world.line_specials.level_timer);
    w.i32(world.line_specials.level_time_count);
    w.usize(world.line_specials.scroll_lines.len());
    for l in world.line_specials.scroll_lines.iter() {
        w.usize(*l);
    }
    w.i32(world.line_specials.total_secrets);

    w.out
}

/// `G_DoLoadGame` core: adopt a snapshot over a freshly loaded world of the
/// same map.
pub fn restore(world: &mut World, data: &[u8]) -> Result<(), SaveError> {
    let mut r = Rdr::new(data);
    let magic = r.bytes(8)?;
    if magic[..4] != MAGIC[..4] {
        return Err(SaveError::BadMagic);
    }
    if magic[4..] != MAGIC[4..] {
        return Err(SaveError::BadVersion);
    }

    world.leveltime = r.i32()?;
    world.validcount = r.i32()?;
    world.totalkills = r.i32()?;
    world.totalitems = r.i32()?;
    world.totalsecret = r.i32()?;
    world.nomonsters = r.b()?;
    world.respawnmonsters = r.b()?;
    world.commercial = r.b()?;
    world.shareware = r.b()?;
    world.skill = r.i32()?;
    world.netgame = r.b()?;
    world.deathmatch = r.i32()?;
    world.consoleplayer = r.usize()?;
    world.fastparm = r.b()?;
    world.gameepisode = r.i32()?;
    world.gamemap = r.i32()?;
    world.timelimit = r.i32()?;
    world.pending_exit = r.opt_id()?.map(|i| i != 0);
    world.pending_reborn_reload = r.b()?;
    world.rng.m.set_state(r.u8()?);
    world.rng.p.set_state(r.u8()?);

    for ing in world.player_in_game.iter_mut() {
        *ing = r.b()?;
    }
    for p in world.players.iter_mut() {
        *p = load_player(&mut r)?;
    }

    // level dynamic state
    let n_sectors = r.usize()?;
    if n_sectors != world.level.sectors.len() {
        return Err(SaveError::BadVersion); // wrong map loaded
    }
    for s in world.level.sectors.iter_mut() {
        s.floorheight = r.i32()?;
        s.ceilingheight = r.i32()?;
        s.lightlevel = r.i16()?;
        s.special = r.i16()?;
        s.specialdata = r.opt_id()?;
        s.soundtraversed = r.i32()?;
        s.soundtarget = r.opt_id()?;
        s.validcount = r.i32()?;
    }
    let n_lines = r.usize()?;
    if n_lines != world.level.lines.len() {
        return Err(SaveError::BadVersion);
    }
    for l in world.level.lines.iter_mut() {
        l.special = r.i16()?;
    }
    let n_sides = r.usize()?;
    if n_sides != world.level.sides.len() {
        return Err(SaveError::BadVersion);
    }
    for s in world.level.sides.iter_mut() {
        s.textureoffset = r.i32()?;
    }

    // mobjs
    let mlen = r.usize()?;
    let mut slots: Vec<Option<Mobj>> = Vec::with_capacity(mlen);
    for _ in 0..mlen {
        if r.b()? {
            slots.push(Some(load_mobj(&mut r)?));
        } else {
            slots.push(None);
        }
    }

    // thinkers (needed before relinking: restore into a scratch list)
    let tlen = r.usize()?;
    let mut pairs = Vec::with_capacity(tlen);
    for _ in 0..tlen {
        let id = r.usize()?;
        let kind_tag = r.u8()?;
        let payload = r.u8()?;
        let kind = match kind_tag {
            0 => crate::p_tick::ThinkerKind::Mobj(payload as usize),
            1 => crate::p_tick::ThinkerKind::Door(payload as usize),
            2 => crate::p_tick::ThinkerKind::Ceiling(payload as usize),
            3 => crate::p_tick::ThinkerKind::Plat(payload as usize),
            4 => crate::p_tick::ThinkerKind::Floor(payload as usize),
            5 => crate::p_tick::ThinkerKind::Light(payload as usize),
            _ => crate::p_tick::ThinkerKind::Other(payload as u32),
        };
        pairs.push((id, kind));
    }

    // specials
    load_specials(&mut r, world)?;

    // Adopt the arenas now (specialdata/ThinkerIds in movers reference these).
    let level = core::mem::take(&mut world.level);
    world
        .mobjs
        .restore_slots(slots, &level, &world.render, level.sectors.len().max(1));
    world.level = level;
    world.thinkers.restore(&pairs);

    // misc queues
    let blen = r.usize()?;
    world.bodyque.clear();
    for _ in 0..blen {
        world.bodyque.push(r.opt_id()?);
    }
    let qlen = r.usize()?;
    world.item_respawn_queue.clear();
    for _ in 0..qlen {
        let t = load_thing(&mut r)?;
        let time = r.i32()?;
        world.item_respawn_queue.push((t, time));
    }
    let btlen = r.usize()?;
    world.brain_targets.clear();
    for _ in 0..btlen {
        if let Some(t) = r.opt_id()? {
            world.brain_targets.push(t);
        }
    }
    world.brain_target_on = r.usize()?;
    world.brain_easy_toggle = r.b()?;

    // line specials bookkeeping
    world.line_specials.level_timer = r.b()?;
    world.line_specials.level_time_count = r.i32()?;
    let scrlen = r.usize()?;
    world.line_specials.scroll_lines.clear();
    for _ in 0..scrlen {
        world.line_specials.scroll_lines.push(r.usize()?);
    }
    world.line_specials.total_secrets = r.i32()?;

    Ok(())
}

fn save_specials(w: &mut Wtr, world: &World) {
    use crate::p_spec::Specials;
    let s: &Specials = &world.specials;

    w.usize(s.doors.len());
    for d in &s.doors {
        w.usize(d.thinker);
        w.usize(d.sector);
        w.u8(d.ty as u8);
        w.i32(d.topheight);
        w.i32(d.speed);
        w.i32(d.direction);
        w.i32(d.topwait);
        w.i32(d.topcountdown);
    }
    w.usize(s.ceilings.len());
    for c in &s.ceilings {
        w.usize(c.thinker);
        w.usize(c.sector);
        w.b(c.crush);
        w.i32(c.bottomheight);
        w.i32(c.topheight);
        w.i32(c.speed);
        w.i32(c.direction);
        w.i16(c.tag);
        w.i32(c.olddirection);
        w.u8(c.ty as u8);
    }
    w.usize(s.floors.len());
    for f in &s.floors {
        w.usize(f.thinker);
        w.u8(f.ty as u8);
        w.b(f.crush);
        w.usize(f.sector);
        w.i32(f.direction);
        w.i16(f.newspecial);
        for b in f.texture.0.iter() {
            w.u8(*b);
        }
        w.i32(f.floordestheight);
        w.i32(f.speed);
    }
    w.usize(s.plats.len());
    for p in &s.plats {
        w.usize(p.thinker);
        w.usize(p.sector);
        w.i32(p.speed);
        w.i32(p.low);
        w.i32(p.high);
        w.i32(p.wait);
        w.i32(p.count);
        w.u8(p.status as u8);
        w.u8(p.oldstatus as u8);
        w.b(p.crush);
        w.i16(p.tag);
        w.u8(p.ty as u8);
    }
    w.usize(s.lights.len());
    for l in &s.lights {
        w.usize(l.thinker);
        w.usize(l.sector);
        w.i32(l.count);
        w.u8(match l.kind {
            crate::p_lights::LightKind::FireFlicker { .. } => 0,
            crate::p_lights::LightKind::Flash { .. } => 1,
            crate::p_lights::LightKind::Strobe { .. } => 2,
            crate::p_lights::LightKind::Glow { .. } => 3,
        });
        match l.kind {
            crate::p_lights::LightKind::FireFlicker { minlight, maxlight } => {
                w.i32(minlight);
                w.i32(maxlight);
            }
            crate::p_lights::LightKind::Flash { minlight, maxlight, maxtime, mintime } => {
                w.i32(minlight);
                w.i32(maxlight);
                w.i32(maxtime);
                w.i32(mintime);
            }
            crate::p_lights::LightKind::Strobe { minlight, maxlight, darktime, brighttime } => {
                w.i32(minlight);
                w.i32(maxlight);
                w.i32(darktime);
                w.i32(brighttime);
            }
            crate::p_lights::LightKind::Glow { minlight, maxlight, direction } => {
                w.i32(minlight);
                w.i32(maxlight);
                w.i32(direction);
            }
        }
    }
    w.usize(s.buttons.len());
    for b in &s.buttons {
        w.usize(b.line);
        w.u8(b.where_ as u8);
        for byte in b.texture.0.iter() {
            w.u8(*byte);
        }
        w.i32(b.timer);
    }
    w.usize(s.active_ceilings.len());
    for a in &s.active_ceilings {
        w.opt_id(*a);
    }
    w.usize(s.active_plats.len());
    for a in &s.active_plats {
        w.opt_id(*a);
    }
}

fn load_specials(r: &mut Rdr, world: &mut World) -> Result<(), SaveError> {
    use crate::p_lights::{Light, LightKind};
    use crate::p_spec::Specials;

    let doors = r.usize()?;
    let mut dvec = Vec::with_capacity(doors);
    for _ in 0..doors {
        dvec.push(crate::p_doors::VlDoor {
            thinker: r.usize()?,
            sector: r.usize()?,
            ty: match r.u8()? {
                0 => crate::p_doors::VlDoorType::Normal,
                1 => crate::p_doors::VlDoorType::Close30ThenOpen,
                2 => crate::p_doors::VlDoorType::Close,
                3 => crate::p_doors::VlDoorType::Open,
                4 => crate::p_doors::VlDoorType::RaiseIn5Mins,
                5 => crate::p_doors::VlDoorType::BlazeRaise,
                6 => crate::p_doors::VlDoorType::BlazeOpen,
                _ => crate::p_doors::VlDoorType::BlazeClose,
            },
            topheight: r.i32()?,
            speed: r.i32()?,
            direction: r.i32()?,
            topwait: r.i32()?,
            topcountdown: r.i32()?,
        });
    }
    let ceilings = r.usize()?;
    let mut cvec = Vec::with_capacity(ceilings);
    for _ in 0..ceilings {
        cvec.push(crate::p_ceilng::Ceiling {
            thinker: r.usize()?,
            sector: r.usize()?,
            crush: r.b()?,
            bottomheight: r.i32()?,
            topheight: r.i32()?,
            speed: r.i32()?,
            direction: r.i32()?,
            tag: r.i16()?,
            olddirection: r.i32()?,
            ty: match r.u8()? {
                0 => crate::p_ceilng::CeilingType::RaiseToHighest,
                1 => crate::p_ceilng::CeilingType::LowerAndCrush,
                2 => crate::p_ceilng::CeilingType::SilentCrushAndRaise,
                3 => crate::p_ceilng::CeilingType::FastCrushAndRaise,
                4 => crate::p_ceilng::CeilingType::CrushAndRaise,
                _ => crate::p_ceilng::CeilingType::LowerToFloor,
            },
        });
    }
    let floors = r.usize()?;
    let mut fvec = Vec::with_capacity(floors);
    for _ in 0..floors {
        let thinker = r.usize()?;
        let ty_raw = r.u8()?;
        let crush = r.b()?;
        let sector = r.usize()?;
        let direction = r.i32()?;
        let newspecial = r.i16()?;
        let mut name = [0u8; 8];
        for b in name.iter_mut() {
            *b = r.u8()?;
        }
        let floordestheight = r.i32()?;
        let speed = r.i32()?;
        fvec.push(crate::p_floor::FloorMove {
            thinker,
            ty: crate::p_floor::FloorType::from_index(ty_raw as i32),
            crush,
            sector,
            direction,
            newspecial,
            texture: crate::wad::LumpName(name),
            floordestheight,
            speed,
        });
    }
    let plats = r.usize()?;
    let mut pvec = Vec::with_capacity(plats);
    for _ in 0..plats {
        let thinker = r.usize()?;
        let sector = r.usize()?;
        let speed = r.i32()?;
        let low = r.i32()?;
        let high = r.i32()?;
        let wait = r.i32()?;
        let count = r.i32()?;
        let status_raw = r.u8()?;
        let oldstatus_raw = r.u8()?;
        let crush = r.b()?;
        let tag = r.i16()?;
        let ty_raw = r.u8()?;
        pvec.push(crate::p_plats::Plat {
            thinker,
            sector,
            speed,
            low,
            high,
            wait,
            count,
            status: if status_raw == 0 {
                crate::p_plats::PlatStatus::Up
            } else if status_raw == 1 {
                crate::p_plats::PlatStatus::Down
            } else if status_raw == 2 {
                crate::p_plats::PlatStatus::Waiting
            } else {
                crate::p_plats::PlatStatus::InStasis
            },
            oldstatus: if oldstatus_raw == 0 {
                crate::p_plats::PlatStatus::Up
            } else if oldstatus_raw == 1 {
                crate::p_plats::PlatStatus::Down
            } else if oldstatus_raw == 2 {
                crate::p_plats::PlatStatus::Waiting
            } else {
                crate::p_plats::PlatStatus::InStasis
            },
            crush,
            tag,
            ty: crate::p_plats::PlatType::from_index(ty_raw as i32),
        });
    }
    let lights = r.usize()?;
    let mut lvec = Vec::with_capacity(lights);
    for _ in 0..lights {
        let thinker = r.usize()?;
        let sector = r.usize()?;
        let count = r.i32()?;
        let kind_raw = r.u8()?;
        let kind = match kind_raw {
            0 => {
                let minlight = r.i32()?;
                let maxlight = r.i32()?;
                LightKind::FireFlicker { minlight, maxlight }
            }
            1 => {
                let minlight = r.i32()?;
                let maxlight = r.i32()?;
                let maxtime = r.i32()?;
                let mintime = r.i32()?;
                LightKind::Flash { minlight, maxlight, maxtime, mintime }
            }
            2 => {
                let minlight = r.i32()?;
                let maxlight = r.i32()?;
                let darktime = r.i32()?;
                let brighttime = r.i32()?;
                LightKind::Strobe { minlight, maxlight, darktime, brighttime }
            }
            _ => {
                let minlight = r.i32()?;
                let maxlight = r.i32()?;
                let direction = r.i32()?;
                LightKind::Glow { minlight, maxlight, direction }
            }
        };
        lvec.push(Light { thinker, sector, count, kind });
    }
    let buttons = r.usize()?;
    let mut bvec = Vec::with_capacity(buttons);
    for _ in 0..buttons {
        let line = r.usize()?;
        let where_raw = r.u8()?;
        let mut name = [0u8; 8];
        for b in name.iter_mut() {
            *b = r.u8()?;
        }
        let timer = r.i32()?;
        bvec.push(crate::p_switch::Button {
            line,
            where_: if where_raw == 0 {
                crate::p_switch::ButtonWhere::Top
            } else if where_raw == 1 {
                crate::p_switch::ButtonWhere::Middle
            } else {
                crate::p_switch::ButtonWhere::Bottom
            },
            texture: crate::wad::LumpName(name),
            timer,
        });
    }
    let n_active_ceil = r.usize()?;
    let mut ac = Vec::with_capacity(n_active_ceil);
    for _ in 0..n_active_ceil {
        ac.push(r.opt_id()?);
    }
    let n_active_plats = r.usize()?;
    let mut ap = Vec::with_capacity(n_active_plats);
    for _ in 0..n_active_plats {
        ap.push(r.opt_id()?);
    }

    world.specials = Specials {
        doors: dvec,
        ceilings: cvec,
        floors: fvec,
        plats: pvec,
        lights: lvec,
        buttons: bvec,
        active_ceilings: ac,
        active_plats: ap,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doomdef::MAX_PLAYERS;
    use crate::level::Level;
    use crate::r_main::Render;
    use crate::g_game::{g_ticker, PlayerStarts};
    use crate::d_ticcmd::{TicCmd, BT_ATTACK};
    use crate::level::{Line, Sector, Subsector};

    fn mini_level() -> Level {
        let mut level = Level::default();
        for (floor, ceil) in [(0, 128), (0, 96)] {
            level.sectors.push(Sector {
                floorheight: floor << 16,
                ceilingheight: ceil << 16,
                tag: if ceil == 96 { 7 } else { 0 },
                ..Sector::default()
            });
        }
        level.lines.push(Line {
            flags: crate::doomdef::ML_TWOSIDED,
            special: 4,
            tag: 7,
            frontsector: Some(0),
            backsector: Some(1),
            ..Line::default()
        });
        level.sectors[0].lines = alloc::vec![0];
        level.sectors[1].lines = alloc::vec![0];
        level.subsectors.push(Subsector {
            sector: 0,
            numlines: 0,
            firstline: 0,
        });
        level
    }

    fn world_with_player() -> World {
        let mut world = World::new(mini_level(), Render::new());
        world.player_in_game[0] = true;
        let mut starts: PlayerStarts = [None; MAX_PLAYERS];
        starts[0] = Some(Thing {
            x: 0,
            y: 0,
            angle: 0,
            ty: 1,
            options: 0,
        });
        crate::g_game::p_spawn_player(&mut world, 0, &starts[0].unwrap(), false);
        world
    }

    #[test]
    fn save_load_resumes_identically() {
        // Play A for a while, save; restore into a fresh world B and keep
        // playing both with the same commands: they must stay in lockstep.
        let mut a = world_with_player();
        let starts: PlayerStarts = [None; MAX_PLAYERS];

        let fire = TicCmd {
            forwardmove: 40,
            buttons: BT_ATTACK,
            ..TicCmd::default()
        };
        for _ in 0..20 {
            a.players[0].cmd = fire;
            g_ticker(&mut a, &starts, &[]);
        }
        let snap = save(&a);

        let mut b = world_with_player();
        assert!(restore(&mut b, &snap).is_ok());

        for _ in 0..20 {
            a.players[0].cmd = fire;
            b.players[0].cmd = fire;
            g_ticker(&mut a, &starts, &[]);
            g_ticker(&mut b, &starts, &[]);
        }

        let (ax, ay) = a
            .players[0]
            .mo
            .map(|m| (a.mobjs.get(m).unwrap().x, a.mobjs.get(m).unwrap().y))
            .unwrap_or((0, 0));
        let (bx, by) = b
            .players[0]
            .mo
            .map(|m| (b.mobjs.get(m).unwrap().x, b.mobjs.get(m).unwrap().y))
            .unwrap_or((0, 0));
        assert_eq!((ax, ay), (bx, by));
        assert_eq!(a.leveltime, b.leveltime);
        assert_eq!(a.mobjs.len(), b.mobjs.len());
        assert_eq!(
            a.players[0].ammo[0], b.players[0].ammo[0],
            "both burned the same clips"
        );
    }

    #[test]
    fn rejects_foreign_blobs() {
        let mut b = world_with_player();
        assert_eq!(restore(&mut b, b"garbage!!"), Err(SaveError::BadMagic));
        assert_eq!(
            restore(&mut b, &[0, 0, 0, 0, 9, 9, 9, 9]),
            Err(SaveError::BadMagic)
        );
        assert_eq!(
            restore(&mut b, b"RDSV\0\x02\0\0"),
            Err(SaveError::BadVersion)
        );
        assert_eq!(
            restore(&mut b, b"RDSV\0\x01\0\0"),
            Err(SaveError::Truncated)
        );
    }

}
