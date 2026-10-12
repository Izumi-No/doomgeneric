//! The running world: level + mobjs + thinkers + players + specials, port of
//! the collection of globals in `doomstat.c`/`p_*` modules.
//!
//! A `World` owns everything the play simulation touches and provides the
//! action dispatch that links `info::STATES` action functions to the code in
//! `p_enemy`/`p_pspr`/`p_inter`.

use alloc::vec::Vec;

use crate::d_player::Player;
use crate::doomdef::MAX_PLAYERS;
use crate::fixed::{fixed_mul, Fixed, FRAC_BITS, FRAC_UNIT};
use crate::info::{Action, MobjType, StateId, STATES};
use crate::level::Level;
use crate::p_inter::Interaction;
use crate::p_map::{try_move, MAXRADIUS};
use crate::p_maputl::{aprox_distance, blockmap_dims, MAPBLOCKSHIFT};
use crate::p_mobj::{spawn_mobj, Mobjs, MobjId, MF_MISSILE, MF_SHADOW, MF_SHOOTABLE};
use crate::p_spec::Specials;
use crate::s_sound::SoundSystem;
use crate::p_tick::{ThinkerId, ThinkerKind, Thinkers};
use crate::random::Random;
use crate::r_main::{point_to_angle_from, Render};
use crate::sounds::Sfx;
use crate::tables::{finecosine_index, finesine_index, ANGLETOFINESHIFT};

/// Maximum number of on-screen sprites that a single frame can list.
pub struct World {
    pub level: Level,
    pub mobjs: Mobjs,
    pub thinkers: Thinkers,
    pub rng: Random,
    pub render: Render,
    /// Door/floor/ceiling/plat mover arenas (`p_spec.c` state).
    pub specials: Specials,

    /// Line validcount filter buffer for `block_lines`.
    pub line_vc: Vec<i32>,
    pub validcount: i32,

    pub players: [Player; MAX_PLAYERS],
    pub player_in_game: [bool; MAX_PLAYERS],
    pub totalkills: i32,
    pub totalitems: i32,
    pub totalsecret: i32,
    pub leveltime: i32,
    pub nomonsters: bool,
    pub respawnmonsters: bool,
    pub commercial: bool,
    /// Shareware episode-1 IWAD (`gamemode == shareware`).
    pub shareware: bool,
    pub skill: i32,
    pub netgame: bool,
    pub deathmatch: i32,
    pub consoleplayer: usize,
    pub fastparm: bool,

    /// `P_SpawnSpecials` line bookkeeping (scrolling lines, level timer).
    pub line_specials: crate::p_mobj::LineSpecials,
    /// Wall texture heights by name (`textureheight[]`), filled by the host
    /// from `RenderData` (needed by `EV_DoFloor(raiseToTexture)`).
    pub texture_heights: Vec<(crate::wad::LumpName, i32)>,
    /// The switch texture pairs (`switchlist`), filled by the host.
    pub switches: crate::p_switch::SwitchList,
    /// Exit request from line specials: `Some(secret)` = `G_ExitLevel` /
    /// `G_SecretExitLevel`, consumed by the game ticker.
    pub pending_exit: Option<bool>,
    /// `ga_loadlevel` request from a single-player reborn.
    pub pending_reborn_reload: bool,

    /// Deathmatch `-timer` minutes (`timelimit`).
    pub timelimit: i32,

    /// The corpse-flush queue from `G_CheckSpot` (`bodyque`).
    pub bodyque: Vec<Option<MobjId>>,

    /// `itemrespawnque`/`itemrespawntime`: picked-up `MF_SPECIAL` spots
    /// awaiting the deathmatch respawn (`(mapthing, leveltime)`).
    pub item_respawn_queue: Vec<(crate::level::Thing, i32)>,

    /// Current episode/map (`gameepisode`/`gamemap`).
    pub gameepisode: i32,
    pub gamemap: i32,

    /// The sound/music request queue (`s_sound.c` globals).
    pub sounds: SoundSystem,

    /// DOOM II boss-brain state (`braintargets`/`braintargeton`).
    pub brain_targets: Vec<MobjId>,
    pub brain_target_on: usize,
    /// The `easy` toggle from `A_BrainSpit`.
    pub brain_easy_toggle: bool,
}

impl World {
    /// Create a world from a loaded level.
    pub fn new(level: Level, render: Render) -> Self {
        let n_lines = level.lines.len();
        let n_sectors = level.sectors.len();
        World {
            mobjs: Mobjs::new(n_sectors.max(1)),
            thinkers: Thinkers::new(),
            rng: Random::new(),
            render,
            specials: Specials::new(),
            line_vc: alloc::vec![0; n_lines.max(1)],
            validcount: 0,
            players: core::array::from_fn(|_| Player::new()),
            player_in_game: [false; MAX_PLAYERS],
            totalkills: 0,
            totalitems: 0,
            totalsecret: 0,
            leveltime: 0,
            nomonsters: false,
            respawnmonsters: false,
            commercial: false,
            shareware: false,
            skill: 2,
            netgame: false,
            deathmatch: 0,
            consoleplayer: 0,
            fastparm: false,
            line_specials: crate::p_mobj::LineSpecials::default(),
            texture_heights: Vec::new(),
            switches: crate::p_switch::SwitchList::default(),
            pending_exit: None,
            pending_reborn_reload: false,
            timelimit: 0,
            bodyque: Vec::new(),
            item_respawn_queue: Vec::new(),
            sounds: SoundSystem::new(),
            gameepisode: 1,
            gamemap: 1,
            brain_targets: Vec::new(),
            brain_target_on: 0,
            brain_easy_toggle: false,
            level,
        }
    }

    #[inline]
    pub fn bump_validcount(&mut self) -> i32 {
        self.validcount += 1;
        self.validcount
    }

    /// `P_SetMobjState` with action dispatch — calls the `A_*` action on
    /// every transition with a tics value (same timing as vanilla).
    pub fn set_mobj_state(&mut self, id: MobjId, state: StateId) -> bool {
        self.run_mobj_state(id, state)
    }

    /// Continue the state chain starting at `state`, invoking actions.
    pub fn run_mobj_state(&mut self, id: MobjId, mut state: StateId) -> bool {
        loop {
            if state == StateId::S_NULL {
                if let Some(m) = self.mobjs.get_mut(id) {
                    m.state = StateId::S_NULL;
                }
                crate::p_mobj::remove_mobj(&mut self.mobjs, &mut self.thinkers, id);
                return false;
            }

            let st = STATES[state as usize];
            if let Some(m) = self.mobjs.get_mut(id) {
                m.state = state;
                m.tics = st.tics;
                m.sprite = st.sprite;
                m.frame = st.frame;
            } else {
                return false;
            }

            if st.action != Action::Null {
                self.run_action(id, st.action);
                if !self.mobjs.is_live(id) {
                    return false;
                }
            }

            state = st.nextstate;
            let tics = self.mobjs.get(id).map(|m| m.tics).unwrap_or(0);
            if tics != 0 {
                break;
            }
        }
        true
    }

    /// State action dispatcher (`A_*` functions). `id` is the acting mobj.
    pub fn run_action(&mut self, id: MobjId, action: Action) {
        match action {
            Action::A_Look => crate::p_enemy::a_look(self, id),
            Action::A_Chase => crate::p_enemy::a_chase(self, id),
            Action::A_FaceTarget => crate::p_enemy::a_face_target(self, id),
            Action::A_SargAttack => crate::p_enemy::a_sarg_attack(self, id),
            Action::A_TroopAttack => crate::p_enemy::a_troop_attack(self, id),
            Action::A_HeadAttack => crate::p_enemy::a_head_attack(self, id),
            Action::A_BruisAttack => crate::p_enemy::a_bruis_attack(self, id),
            Action::A_SkullAttack => crate::p_enemy::a_skull_attack(self, id),
            Action::A_PosAttack => crate::p_enemy::a_pos_attack(self, id),
            Action::A_SPosAttack => crate::p_enemy::a_spos_attack(self, id),
            Action::A_CPosAttack => crate::p_enemy::a_cpos_attack(self, id),
            Action::A_CPosRefire => crate::p_enemy::a_cpos_refire(self, id),
            Action::A_SpidRefire => crate::p_enemy::a_spid_refire(self, id),
            Action::A_Fall => crate::p_enemy::a_fall(self, id),
            Action::A_Pain => crate::p_enemy::a_pain(self, id),
            Action::A_Scream => crate::p_enemy::a_scream(self, id),
            Action::A_XScream => crate::p_enemy::a_xscream(self, id),
            Action::A_Explode => crate::p_enemy::a_explode(self, id),
            Action::A_Metal => crate::p_enemy::a_metal(self, id),
            Action::A_BabyMetal => crate::p_enemy::a_baby_metal(self, id),
            Action::A_Hoof => crate::p_enemy::a_hoof(self, id),
            Action::A_BspiAttack => crate::p_enemy::a_bspi_attack(self, id),
            Action::A_CyberAttack => crate::p_enemy::a_cyber_attack(self, id),
            Action::A_SkelWhoosh => crate::p_enemy::a_skel_whoosh(self, id),
            Action::A_SkelFist => crate::p_enemy::a_skel_fist(self, id),
            Action::A_SkelMissile => crate::p_enemy::a_skel_missile(self, id),
            Action::A_Tracer => crate::p_enemy::a_tracer(self, id),
            Action::A_BFGSpray => crate::p_pspr::a_bfg_spray(self, id),
            Action::A_PlayerScream => crate::p_enemy::a_player_scream(self, id),
            Action::A_VileChase => crate::p_enemy::a_vile_chase(self, id),
            Action::A_VileStart => crate::p_enemy::a_vile_start(self, id),
            Action::A_VileTarget => crate::p_enemy::a_vile_target(self, id),
            Action::A_VileAttack => crate::p_enemy::a_vile_attack(self, id),
            Action::A_StartFire => crate::p_enemy::a_start_fire(self, id),
            Action::A_Fire => crate::p_enemy::a_fire(self, id),
            Action::A_FireCrackle => crate::p_enemy::a_fire_crackle(self, id),
            Action::A_FatRaise => crate::p_enemy::a_fat_raise(self, id),
            Action::A_FatAttack1 => crate::p_enemy::a_fat_attack1(self, id),
            Action::A_FatAttack2 => crate::p_enemy::a_fat_attack2(self, id),
            Action::A_FatAttack3 => crate::p_enemy::a_fat_attack3(self, id),
            Action::A_BossDeath => crate::p_enemy::a_boss_death(self, id),
            Action::A_PainAttack => crate::p_enemy::a_pain_attack(self, id),
            Action::A_PainDie => crate::p_enemy::a_pain_die(self, id),
            Action::A_KeenDie => crate::p_enemy::a_keen_die(self, id),
            Action::A_BrainAwake => crate::p_enemy::a_brain_awake(self, id),
            Action::A_BrainPain => crate::p_enemy::a_brain_pain(self, id),
            Action::A_BrainScream => crate::p_enemy::a_brain_scream(self, id),
            Action::A_BrainDie => crate::p_enemy::a_brain_die(self, id),
            Action::A_BrainSpit => crate::p_enemy::a_brain_spit(self, id),
            Action::A_SpawnSound => crate::p_enemy::a_spawn_sound(self, id),
            Action::A_SpawnFly => crate::p_enemy::a_spawn_fly(self, id),
            Action::A_BrainExplode => crate::p_enemy::a_brain_explode(self, id),
            _ => {} // psprite-side weapon actions live in `p_pspr`
        }
    }

    /// Sight-check helper: bumps `validcount`, then `P_CheckSight`.
    pub fn sight_between(&mut self, a: MobjId, b: MobjId) -> bool {
        let vid = self.bump_validcount();
        crate::p_sight::check_sight(&self.level, &self.mobjs, &mut self.line_vc, vid, a, b)
    }

    /// The `p_inter` context for the current game state.
    pub fn interaction(&self) -> Interaction {
        Interaction {
            netgame: self.netgame,
            deathmatch: self.deathmatch,
            skill: self.skill,
            commercial: self.commercial,
            consoleplayer: self.consoleplayer,
            wminfo_frags: 0,
            totalkills: self.totalkills,
            totalitems: self.totalitems,
            leveltime: self.leveltime,
        }
    }

    /// `P_DamageMobj` bridge.
    pub fn damage_mobj(
        &mut self,
        target: MobjId,
        inflictor: Option<MobjId>,
        source: Option<MobjId>,
        damage: i32,
    ) -> bool {
        let ctx = self.interaction();
        crate::p_inter::damage_mobj(
            &self.level,
            &mut self.mobjs,
            &mut self.thinkers,
            &mut self.rng,
            &mut self.players,
            &ctx,
            target,
            inflictor,
            None,
            source,
            damage,
        )
    }

    /// Sound hook: a UI/global sound at full volume (vanilla
    /// `S_StartSound(NULL, ..)`); the host drains the queue each tic.
    pub fn play_sfx(&mut self, sfx: Sfx) {
        self.sounds.start_sound(sfx);
    }

    /// A positioned sound (`S_StartSound(origin, ..)`), attenuated against
    /// the display player.
    pub fn play_sfx_at(&mut self, sfx: Sfx, x: Fixed, y: Fixed) {
        let listener = self.players[self.consoleplayer]
            .mo
            .and_then(|mo| self.mobjs.get(mo))
            .map(|m| (m.x, m.y, m.angle));
        let map8 = self.gamemap == 8 && !self.commercial;
        self.sounds.start_sound_at(sfx, (x, y), listener, map8);
    }

    /// `P_SpawnMissile`: fire `ty` from `source` toward `dest`.
    pub fn spawn_missile(&mut self, source: MobjId, dest: MobjId, ty: MobjType) -> Option<MobjId> {
        let (sx, sy, sz) = {
            let s = self.mobjs.get(source)?;
            (s.x, s.y, s.z + 4 * 8 * FRAC_UNIT)
        };
        let (dx, dy, dz, dshadow) = {
            let d = self.mobjs.get(dest)?;
            (d.x, d.y, d.z, d.flags & MF_SHADOW != 0)
        };

        let id = spawn_mobj(&mut self.mobjs, &mut self.thinkers, &self.level, &self.render, ty, sx, sy, sz);
        let speed = self.mobjs.get(id).map(|m| m.info().speed).unwrap_or(0);
        if let Some(m) = self.mobjs.get_mut(id) {
            m.target = Some(source);
        }

        let mut an = point_to_angle_from(sx, sy, dx, dy);
        // Fuzzy player.
        if dshadow {
            let r1 = self.rng.p.next() as i32;
            let r2 = self.rng.p.next() as i32;
            an = an.wrapping_add(((r1 - r2) << 20) as u32);
        }
        if let Some(m) = self.mobjs.get_mut(id) {
            m.angle = an;
        }
        let a = an >> ANGLETOFINESHIFT;
        let momx = fixed_mul(speed, finecosine_index(a));
        let momy = fixed_mul(speed, finesine_index(a));
        if let Some(m) = self.mobjs.get_mut(id) {
            m.momx = momx;
            m.momy = momy;
        }

        let mut dist = aprox_distance(dx - sx, dy - sy);
        dist /= speed.max(1);
        if dist < 1 {
            dist = 1;
        }
        let momz = (dz - sz) / dist;
        if let Some(m) = self.mobjs.get_mut(id) {
            m.momz = momz;
        }

        self.check_missile_spawn(id);
        Some(id)
    }

    /// `P_CheckMissileSpawn`.
    pub fn check_missile_spawn(&mut self, id: MobjId) {
        let Some(m) = self.mobjs.get(id) else { return };
        let mut tics = m.tics - (self.rng.p.next() as i32 & 3);
        if tics < 1 {
            tics = 1;
        }
        let (x, y, z, momx, momy, momz) = (m.x, m.y, m.z, m.momx, m.momy, m.momz);
        if let Some(m) = self.mobjs.get_mut(id) {
            m.tics = tics;
            m.x = x + (momx >> 1);
            m.y = y + (momy >> 1);
            m.z = z + (momz >> 1);
        }
        let vid = self.bump_validcount();
        let (nx, ny) = {
            let m = self.mobjs.get(id).unwrap();
            (m.x, m.y)
        };
        if !try_move(&self.level, &self.render, &mut self.mobjs, &mut self.line_vc, vid, id, nx, ny) {
            self.explode_missile(id);
        }
    }

    /// `P_ExplodeMissile`.
    pub fn explode_missile(&mut self, id: MobjId) {
        if let Some(m) = self.mobjs.get_mut(id) {
            m.momx = 0;
            m.momy = 0;
            m.momz = 0;
        }
        let deathstate = self.mobjs.get(id).map(|m| m.info().deathstate).unwrap_or(StateId::S_NULL);
        self.run_mobj_state(id, deathstate);

        let Some(m) = self.mobjs.get(id) else { return };
        let mut tics = m.tics - (self.rng.p.next() as i32 & 3);
        if tics < 1 {
            tics = 1;
        }
        if let Some(m) = self.mobjs.get_mut(id) {
            m.tics = tics;
            m.flags &= !MF_MISSILE;
        }
        let deathsound = self.mobjs.get(id).map(|m| m.info().deathsound).unwrap_or(Sfx::sfx_None);
        self.play_sfx(deathsound);
    }

    /// `P_RunThinkers`: visit every thinker once, running live ones and
    /// unlinking marked ones (the `p_tick.c` loop specialised for `World`).
    ///
    /// Thinkers appended by a handler during the run are visited in the same
    /// pass, like the C loop re-reading `->next`.
    pub fn run_thinkers(&mut self) {
        let mut current = self.thinkers.head();
        while let Some(id) = current {
            match self.thinkers.kind(id) {
                None => {
                    current = self.thinkers.next_of(id);
                    self.thinkers.unlink(id);
                }
                Some(kind) => {
                    self.dispatch_thinker(id, kind);
                    current = self.thinkers.next_of(id);
                }
            }
        }
    }

    /// Run one thinker node (`thinker->function.acp1` dispatch).
    fn dispatch_thinker(&mut self, _id: ThinkerId, kind: ThinkerKind) {
        match kind {
            ThinkerKind::Mobj(mo) => {
                let (oldx, oldy) = self
                    .mobjs
                    .get(mo)
                    .map(|m| (m.x, m.y))
                    .unwrap_or((0, 0));
                let vid = self.bump_validcount();
                crate::p_mobj::mobj_thinker(
                    &self.level,
                    &self.render,
                    &mut self.mobjs,
                    &mut self.thinkers,
                    &mut self.line_vc,
                    vid,
                    mo,
                );
                // crossed special lines + item pickups (vanilla P_TryMove)
                if self.mobjs.is_live(mo) {
                    self.post_move_hooks(mo, oldx, oldy);
                }
                // check for nightmare respawn
                if self.respawnmonsters
                    && self
                        .mobjs
                        .get(mo)
                        .map(|m| m.tics == -1 && m.flags & crate::p_mobj::MF_COUNTKILL != 0)
                        .unwrap_or(false)
                {
                    let movecount = self.mobjs.get(mo).map(|m| m.movecount).unwrap_or(0) + 1;
                    if let Some(m) = self.mobjs.get_mut(mo) {
                        m.movecount = movecount;
                    }
                    if movecount >= 12 * crate::doomdef::TICRATE as i32
                        && self.leveltime & 31 == 0
                        && (self.rng.p.next() as i32) <= 4
                    {
                        crate::p_mobj::nightmare_respawn(self, mo);
                    }
                }
            }
            ThinkerKind::Door(door) => crate::p_doors::t_vertical_door(self, door),
            ThinkerKind::Ceiling(ceiling) => crate::p_ceilng::t_move_ceiling(self, ceiling),
            ThinkerKind::Floor(floor) => crate::p_floor::t_move_floor(self, floor),
            ThinkerKind::Plat(plat) => crate::p_plats::t_plat_raise(self, plat),
            ThinkerKind::Light(light) => crate::p_lights::t_light(self, light),
            // Misc thinkers come with later phases.
            ThinkerKind::Other(_) => {}
        }
    }

    /// `P_RadiusAttack`: `spot` is the impact point, `source` the attacker.
    pub fn radius_attack(&mut self, spot: MobjId, source: Option<MobjId>, damage: i32) {
        let (sx, sy) = {
            let Some(s) = self.mobjs.get(spot) else { return };
            (s.x, s.y)
        };

        let (orgx, orgy, bw, bh) = blockmap_dims(&self.level).unwrap_or((0, 0, 0, 0));
        if bw == 0 || bh == 0 {
            return;
        }
        let dist = (damage + MAXRADIUS) << FRAC_BITS;
        let xl = (sx - dist - orgx) >> MAPBLOCKSHIFT;
        let xh = (sx + dist - orgx) >> MAPBLOCKSHIFT;
        let yl = (sy - dist - orgy) >> MAPBLOCKSHIFT;
        let yh = (sy + dist - orgy) >> MAPBLOCKSHIFT;

        let mut buf: Vec<MobjId> = Vec::new();
        for y in yl..=yh {
            for x in xl..=xh {
                if x < 0 || y < 0 || x as usize >= bw || y as usize >= bh {
                    continue;
                }
                let block = y as usize * bw + x as usize;
                buf.clear();
                self.mobjs.collect_block(block, &mut buf);
                for t in buf.iter().copied() {
                    if !self.mobjs.is_live(spot) {
                        return;
                    }
                    let Some(thing) = self.mobjs.get(t) else { continue };
                    if thing.flags & MF_SHOOTABLE == 0 {
                        continue;
                    }
                    // Boss spider and cyborg take no concussive damage.
                    if thing.ty == MobjType::MT_CYBORG || thing.ty == MobjType::MT_SPIDER {
                        continue;
                    }
                    let dx = (thing.x - sx).abs();
                    let dy = (thing.y - sy).abs();
                    let mut d = if dx > dy { dx } else { dy };
                    d = (d - thing.radius) >> FRAC_BITS;
                    if d < 0 {
                        d = 0;
                    }
                    if d >= damage {
                        continue; // out of range
                    }
                    if !self.sight_between(t, spot) {
                        continue;
                    }
                    self.damage_mobj(t, Some(spot), source, damage - d);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl World {
        fn level_one() -> Self {
            let mut level = Level::default();
            // One big room sector.
            level.sectors.push(crate::level::Sector::default());
            level.sectors[0].floorheight = 0;
            level.sectors[0].ceilingheight = 200 << 16;
            // one subsector containing the whole area
            level.subsectors.push(crate::level::Subsector {
                sector: 0,
                numlines: 0,
                firstline: 0,
            });
            World::new(level, Render::new())
        }
    }

    #[test]
    fn world_spawn_and_look_for_player() {
        let mut w = World::level_one();
        // Add a player mobj.
        let mut p = Player::new();
        let pmid = crate::p_mobj::spawn_mobj(
            &mut w.mobjs, &mut w.thinkers, &w.level, &w.render,
            crate::info::MobjType::MT_PLAYER, 10 << 16, 10 << 16, crate::p_mobj::ONFLOORZ,
        );
        p.mo = Some(pmid);
        w.players[0] = p;
        w.player_in_game[0] = true;

        // A monster within sight.
        let mid = crate::p_mobj::spawn_mobj(
            &mut w.mobjs, &mut w.thinkers, &w.level, &w.render,
            crate::info::MobjType::MT_POSSESSED, 30 << 16, 10 << 16, crate::p_mobj::ONFLOORZ,
        );

        // Run a look tick — the zombie should pick the player.
        w.run_action(mid, Action::A_Look);
        assert_eq!(w.mobjs.get(mid).unwrap().target, Some(pmid));
    }
}
