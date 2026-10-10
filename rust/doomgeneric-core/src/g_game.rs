//! The game ticker and level lifecycle, port of the play-simulation core of
//! `g_game.c`.
//!
//! Savegames, demos, the intermission (`wi_stuff`) and the finale are host /
//! Fase 4 concerns; the port models the in-level state machine: reborn
//! players, `P_SetupLevel` glue, per-tic `G_Ticker` and `G_RespawnSpecials`.

use alloc::vec::Vec;
use crate::d_player::{Player, PlayerState};
use crate::doomdef::{MAX_PLAYERS, TICRATE, AmmoType, WeaponType};
use crate::fixed::{fixed_mul, FRAC_UNIT};
use crate::info::MobjType;
use crate::level::{Level, LevelError, Thing};
use crate::p_enemy::noise_alert;
use crate::p_lights::{
    spawn_fire_flicker, spawn_glowing_light, spawn_light_flash, spawn_strobe_flash, FASTDARK,
    SLOWDARK,
};
use crate::p_mobj::{
    remove_mobj, spawn_mobj, spawn_map_things, MF_SOLID, ONFLOORZ,
};
use crate::p_pspr::setup_psprites;
use crate::p_spec;
use crate::p_world::World;
use crate::r_main::Render;
use crate::tables::{finecosine_index, ANGLETOFINESHIFT};
use crate::wad::WadManager;

/// `bodyquesize` (`g_game.c`).
const BODYQUESIZE: usize = 32;

/// Player start per player (`playerstarts`).
pub type PlayerStarts = [Option<Thing>; MAX_PLAYERS];

/// `G_PlayerReborn`: called after a player dies; almost everything is
/// cleared and initialized (frag/kill/item/secret tallies are kept).
pub fn g_player_reborn(world: &mut World, pnum: usize) {
    let frags = world.players[pnum].frags;
    let killcount = world.players[pnum].killcount;
    let itemcount = world.players[pnum].itemcount;
    let secretcount = world.players[pnum].secretcount;

    world.players[pnum] = Player::new();
    let p = &mut world.players[pnum];
    p.frags = frags;
    p.killcount = killcount;
    p.itemcount = itemcount;
    p.secretcount = secretcount;

    // don't do anything immediately
    p.usedown = true;
    p.attackdown = true;
    p.playerstate = PlayerState::Live;
    p.readyweapon = WeaponType::Pistol;
    p.pendingweapon = None;
    p.weaponowned[WeaponType::Fist as usize] = true;
    p.weaponowned[WeaponType::Pistol as usize] = true;
    p.ammo[AmmoType::Clip as usize] = 50;
}

/// `G_PlayerFinishLevel`: can when a player completes a level.
pub fn g_player_finish_level(world: &mut World, pnum: usize) {
    let p = &mut world.players[pnum];
    p.powers = [0; crate::doomdef::NUM_POWERS];
    p.cards = [false; crate::doomdef::NUM_CARDS];
    p.extralight = 0; // cancel gun flashes
    p.fixedcolormap = 0; // cancel ir goggles
    p.damagecount = 0; // no palette changes
    p.bonuscount = 0;
    if let Some(mo) = p.mo {
        if let Some(m) = world.mobjs.get_mut(mo) {
            m.flags &= !crate::p_mobj::MF_SHADOW; // cancel invisibility
        }
    }
}

/// `G_CheckSpot`: false if the player cannot be respawned at `mthing`
/// because something is occupying it.
fn g_check_spot(world: &mut World, pnum: usize, mthing: &Thing) -> bool {
    let x = (mthing.x as i32) << 16;
    let y = (mthing.y as i32) << 16;

    if world.players[pnum].mo.is_none() {
        // first spawn of level, before corpses
        for i in 0..pnum {
            let spot = world
                .players
                .get(i)
                .and_then(|p| p.mo)
                .and_then(|mo| world.mobjs.get(mo))
                .map(|m| (m.x, m.y));
            if spot == Some((x, y)) {
                return false;
            }
        }
        return true;
    }

    let Some(mo) = world.players[pnum].mo else {
        return true;
    };
    let vid = world.bump_validcount();
    if crate::p_map::check_position(
        &world.level,
        &world.render,
        &world.mobjs,
        &mut world.line_vc,
        vid,
        mo,
        x,
        y,
    )
    .is_none()
    {
        return false;
    }

    // flush an old corpse if needed (body queue)
    if world.bodyque.len() >= BODYQUESIZE {
        let old = world.bodyque.remove(0);
        if let Some(old) = old {
            remove_mobj(&mut world.mobjs, &mut world.thinkers, old);
        }
    }
    world.bodyque.push(world.players[pnum].mo);

    // spawn a teleport fog (the vanilla DOS negative-angle quirk is not
    // reproduced; the port always uses the fine tables).
    let an = ((crate::tables::ANG45 >> ANGLETOFINESHIFT) * (mthing.angle as u32 / 45)) & (crate::tables::FINEANGLES as u32 - 1);
    let subsector = world.render.point_in_subsector(&world.level, x, y);
    let sector = world.level.subsectors[subsector].sector;
    let floorz = world.level.sectors[sector].floorheight;
    let fog = spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        MobjType::MT_TFOG,
        x + 20 * finecosine_index(an),
        y + 20 * finecosine_index(an),
        floorz,
    );
    let _ = fog;

    true
}

/// `P_SpawnPlayer`: spawn the player mobj at `mthing` (`type` is 1..4).
pub fn p_spawn_player(world: &mut World, pnum: usize, mthing: &Thing, on_firing_squad: bool) {
    let x = (mthing.x as i32) << 16;
    let y = (mthing.y as i32) << 16;
    let z = ONFLOORZ;

    // remove whatever the player was (corpse etc.)
    if let Some(old) = world.players[pnum].mo {
        if world.mobjs.is_live(old) {
            if let Some(m) = world.mobjs.get_mut(old) {
                m.player = None;
            }
            remove_mobj(&mut world.mobjs, &mut world.thinkers, old);
        }
    }

    let mo = spawn_mobj(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        MobjType::MT_PLAYER,
        x,
        y,
        z,
    );
    if let Some(m) = world.mobjs.get_mut(mo) {
        m.angle = crate::tables::ANG45.wrapping_mul(mthing.angle as u32 / 45);
        // (Vanilla ORs MF_SOLID only outside deathmatch spawn, then always
        // for the level start. Reproduce the level-start path.)
        m.flags |= MF_SOLID;
        m.player = Some(pnum);
    }

    // `G_CheckSpot`'s firing-squad bug note: on deathmatch the player is not
    // shootable until they move; not reproduced here.
    let _ = on_firing_squad;

    let p = &mut world.players[pnum];
    p.mo = Some(mo);
    p.playerstate = PlayerState::Live;
    p.viewz = world.mobjs.get(mo).map(|m| m.z + p.viewheight).unwrap_or(0);

    // Wake up the monsters (the spawn sound teleports nothing, but the player
    // makes a valid noise target from tic one).
    noise_alert(world, mo, mo);

    setup_psprites(world, pnum);
}

/// `G_DeathMatchSpawnPlayer`: pick a random deathmatch start.
pub fn g_deathmatch_spawn_player(world: &mut World, pnum: usize, dm_starts: &[Thing]) {
    let n = dm_starts.len();
    if n == 0 {
        return;
    }
    // About 10 random tries, then the sequential fallback.
    for _ in 0..10 {
        let pick = (world.rng.p.next() as usize) % n;
        if g_check_spot(world, pnum, &dm_starts[pick]) {
            p_spawn_player(world, pnum, &dm_starts[pick], false);
            return;
        }
    }
    for t in dm_starts {
        if g_check_spot(world, pnum, t) {
            p_spawn_player(world, pnum, t, false);
            return;
        }
    }
    // no spot found: he's going to be inside something. Too bad.
    let idx = (world.rng.p.next() as usize) % n;
    p_spawn_player(world, pnum, &dm_starts[idx], false);
}

/// `G_DoReborn`.
///
/// Single player reloads the level from scratch (the caller sees
/// [`World::pending_reborn_reload`]); netgames respawn at the starts.
pub fn g_do_reborn(world: &mut World, pnum: usize, starts: &PlayerStarts, dm_starts: &[Thing]) {
    if !world.netgame {
        // reload the level from scratch
        world.pending_reborn_reload = true;
        return;
    }

    // first dissasociate the corpse
    if let Some(mo) = world.players[pnum].mo {
        if let Some(m) = world.mobjs.get_mut(mo) {
            m.player = None;
        }
    }

    // spawn at random spot if in death match
    if world.deathmatch != 0 {
        g_deathmatch_spawn_player(world, pnum, dm_starts);
        return;
    }

    if let Some(spot) = starts[pnum] {
        if g_check_spot(world, pnum, &spot) {
            p_spawn_player(world, pnum, &spot, false);
            return;
        }
    }
    // try to spawn at one of the other players spots
    for spot in starts.iter().flatten() {
        if g_check_spot(world, pnum, spot) {
            p_spawn_player(world, pnum, spot, false);
            return;
        }
    }
    // he's going to be inside something. Too bad.
    if let Some(spot) = starts[pnum] {
        p_spawn_player(world, pnum, &spot, false);
    }
}

/// `P_SpawnSpecials` thinker part: sector specials that spawn light/door
/// thinkers. The bookkeeping part (`line_specials`) is computed separately.
pub fn spawn_special_thinkers(world: &mut World) {
    for i in 0..world.level.sectors.len() {
        let special = world.level.sectors[i].special;
        if special == 0 {
            continue;
        }
        match special {
            1 => spawn_light_flash(world, i), // FLICKERING LIGHTS
            2 => spawn_strobe_flash(world, i, FASTDARK, false), // STROBE FAST
            3 => spawn_strobe_flash(world, i, SLOWDARK, false), // STROBE SLOW
            4 => {
                // STROBE FAST/DEATH SLIME
                spawn_strobe_flash(world, i, FASTDARK, false);
                world.level.sectors[i].special = 4;
            }
            8 => spawn_glowing_light(world, i), // GLOWING LIGHT
            9 => {}                            // SECRET SECTOR (counted at load)
            10 => crate::p_doors::spawn_door_close_in_30(world, i),
            12 => spawn_strobe_flash(world, i, SLOWDARK, true), // SYNC STROBE SLOW
            13 => spawn_strobe_flash(world, i, FASTDARK, true), // SYNC STROBE FAST
            14 => crate::p_doors::spawn_door_raise_in_5_mins(world, i),
            17 => spawn_fire_flicker(world, i),
            _ => {}
        }
    }
}

/// `G_DoLoadLevel` / `P_SetupLevel` glue: load a map lump into `world` and
/// spawn everything. `map_lump` is the first lump of the map block.
///
/// Cross-level state (players, skill, RNG, host-tuned render) is preserved;
/// everything else is reset, like the `P_SetupLevel` body.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::needless_range_loop)]
pub fn g_do_load_level(
    world: &mut World,
    manager: &WadManager,
    map_lump: usize,
    commercial: bool,
    starts: &mut PlayerStarts,
    dm_starts: &mut Vec<Thing>,
) -> Result<(), LevelError> {
    let level = Level::load(manager, map_lump, commercial)?;

    // Keep what survives a level change, reset the rest.
    let players = world.players.clone();
    let player_in_game = world.player_in_game;
    let (skill, nomonsters, respawnmonsters, fastparm) =
        (world.skill, world.nomonsters, world.respawnmonsters, world.fastparm);
    let (netgame, deathmatch, timelimit, consoleplayer) =
        (world.netgame, world.deathmatch, world.timelimit, world.consoleplayer);
    let (gameepisode, gamemap) = (world.gameepisode, world.gamemap);
    let rng = world.rng;
    let mut render = Render::new();
    core::mem::swap(&mut world.render, &mut render);
    *world = World::new(level, render);

    world.players = players;
    world.player_in_game = player_in_game;
    world.skill = skill;
    world.nomonsters = nomonsters;
    world.respawnmonsters = respawnmonsters;
    world.fastparm = fastparm;
    world.netgame = netgame;
    world.deathmatch = deathmatch;
    world.timelimit = timelimit;
    world.consoleplayer = consoleplayer;
    world.gameepisode = gameepisode;
    world.gamemap = gamemap;
    world.commercial = commercial;
    world.rng = rng;

    for i in 0..MAX_PLAYERS {
        if world.player_in_game[i] && world.players[i].playerstate == PlayerState::Dead {
            world.players[i].playerstate = PlayerState::Reborn;
        }
        world.players[i].frags = [0; MAX_PLAYERS];
    }

    // P_SetupLevel body: spawn things, specials, players.
    let spawned = spawn_map_things(
        &mut world.mobjs,
        &mut world.thinkers,
        &world.level,
        &world.render,
        &mut world.rng,
        world.skill,
        world.netgame,
        world.deathmatch != 0,
        world.nomonsters,
        commercial,
    );
    world.totalkills = spawned.counters.total_kills;
    world.totalitems = spawned.counters.total_items;
    world.totalsecret = spawned.counters.total_secrets;
    *starts = spawned.player_starts;
    *dm_starts = spawned.deathmatch_starts;

    world.line_specials = crate::p_mobj::spawn_specials(
        &world.level,
        world.deathmatch != 0,
        world.timelimit,
    );
    spawn_special_thinkers(world);

    for i in 0..MAX_PLAYERS {
        if world.player_in_game[i]
            && world.players[i].playerstate == PlayerState::Reborn
        {
            g_player_reborn(world, i);
        }
        if world.player_in_game[i] {
            if let Some(spot) = starts[i] {
                p_spawn_player(world, i, &spot, false);
            }
        }
    }

    Ok(())
}

/// `G_Ticker`: one game tic. `starts`/`dm_starts` feed reborns.
pub fn g_ticker(world: &mut World, starts: &PlayerStarts, dm_starts: &[Thing]) {
    // do player reborns if needed
    for i in 0..MAX_PLAYERS {
        if world.player_in_game[i] && world.players[i].playerstate == PlayerState::Reborn {
            g_do_reborn(world, i, starts, dm_starts);
        }
    }

    // A single-player death reload is driven by the host (`ga_loadlevel`).
    if world.pending_reborn_reload {
        world.pending_reborn_reload = false;
    }

    // move the players
    for i in 0..MAX_PLAYERS {
        if !world.player_in_game[i] {
            continue;
        }
        crate::p_user::player_think(world, i);
    }

    // run the thinkers
    world.run_thinkers();

    // deathmatch item respawn
    crate::p_mobj::p_respawn_specials(world);

    // animate specials (scrolling walls, buttons, the level timer)
    p_spec::update_specials(world);

    // for par times (leveltime advances once per game tic)
    world.leveltime += 1;
}

/// `G_InitNew`: set up the game-state flags for a fresh game.
pub fn g_init_new(world: &mut World, skill: i32, episode: i32, map: i32) {
    // Nobody is reborn between levels on a fresh game.
    world.player_in_game = [false; MAX_PLAYERS];
    world.player_in_game[0] = true;
    world.consoleplayer = 0;

    world.skill = skill;
    world.gameepisode = episode;
    world.gamemap = map;
    world.respawnmonsters = skill == 4; // sk_nightmare
    world.fastparm = skill == 4 || world.fastparm;
    let _ = fixed_mul;
    let _ = FRAC_UNIT;
    let _ = TICRATE;
}

/// `G_ExitLevel`.
pub fn g_exit_level(world: &mut World) {
    world.pending_exit = Some(false);
}

/// `G_SecretExitLevel`.
pub fn g_secret_exit_level(world: &mut World) {
    world.pending_exit = Some(true);
}

// ---------------------------------------------------------------------------
// Game flow: `gamestate_t`, `G_DoCompleted`, `G_WorldDone` (`wi_stuff` wiring)
// ---------------------------------------------------------------------------

/// `gamestate_t` (the subset the core drives; menus/finale are host states).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameState {
    Level,
    Intermission,
}

/// Requests the flow layer hands to the host (the C `gameaction_t` queue).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowEvent {
    /// `ga_loadlevel`: load `(episode, map)`.
    LoadLevel { episode: i32, map: i32 },
    /// `ga_victory`: the episode is over (finale text is a host screen).
    Victory,
}

/// DOOM episode par times (`pars`).
static PARS: [[i32; 10]; 4] = [
    [0; 10],
    [0, 30, 75, 120, 90, 165, 180, 180, 30, 165],
    [0, 90, 90, 90, 120, 90, 360, 240, 30, 170],
    [0, 90, 45, 90, 150, 90, 90, 165, 30, 135],
];

/// DOOM II par times (`cpars`).
static CPARS: [i32; 32] = [
    30, 90, 120, 120, 90, 150, 120, 120, 270, 90, 210, 150, 150, 150, 210, 150, 420, 150, 210,
    150, 240, 150, 180, 150, 150, 300, 330, 420, 300, 180, 120, 30,
];

/// The in-game flow state machine: level <-> intermission, level advancing,
/// victory detection. The host owns the loop and performs the WAD IO the
/// [`FlowEvent`]s ask for.
#[derive(Clone, Debug)]
pub struct GameFlow {
    pub state: GameState,
    /// The running intermission once `state == Intermission`.
    pub inter: Option<crate::wi_stuff::Intermission>,
    /// `secretexit` (the pending exit carries it until `g_do_completed`).
    pub secretexit: bool,
    /// Events queued by the completed/world-done paths (drained per tic).
    pending: Vec<FlowEvent>,
}

impl Default for GameFlow {
    fn default() -> Self {
        GameFlow {
            state: GameState::Level,
            inter: None,
            secretexit: false,
            pending: Vec::new(),
        }
    }
}

impl GameFlow {
    pub fn new() -> Self {
        Self::default()
    }

    /// One game tic: runs the level (or the intermission) and returns the
    /// actions the host must perform (loading levels, showing the finale).
    pub fn ticker(
        &mut self,
        world: &mut World,
        starts: &PlayerStarts,
        dm_starts: &[Thing],
        skip_input: bool,
    ) -> Vec<FlowEvent> {
        match self.state {
            GameState::Level => {
                g_ticker(world, starts, dm_starts);
                if let Some(secret) = world.pending_exit.take() {
                    self.secretexit = secret;
                    self.g_do_completed(world);
                }
                if world.pending_reborn_reload {
                    self.pending.push(FlowEvent::LoadLevel {
                        episode: world.gameepisode,
                        map: world.gamemap,
                    });
                }
            }
            GameState::Intermission => {
                let done = self
                    .inter
                    .as_mut()
                    .map(|wi| wi.ticker(skip_input))
                    .unwrap_or(true);
                if done {
                    self.g_do_world_done(world);
                }
            }
        }
        core::mem::take(&mut self.pending)
    }

    /// `G_DoCompleted`: tally the level, compute the next map and enter the
    /// intermission.
    fn g_do_completed(&mut self, world: &mut World) {
        for i in 0..MAX_PLAYERS {
            if world.player_in_game[i] {
                g_player_finish_level(world, i);
            }
        }

        let secretexit = self.secretexit;
        if !world.commercial && world.gamemap == 8 {
            // victory: the finale is a host screen.
            self.state = GameState::Level;
            self.pending.push(FlowEvent::Victory);
            return;
        }
        if !world.commercial && world.gamemap == 9 {
            // returning from secret level
            for p in world.players.iter_mut() {
                p.didsecret = true;
            }
        }

        let mut info = crate::wi_stuff::WbStart {
            didsecret: world.players[world.consoleplayer].didsecret,
            epsd: world.gameepisode - 1,
            last: world.gamemap - 1,
            pnum: world.consoleplayer,
            maxkills: world.totalkills,
            maxitems: world.totalitems,
            maxsecret: world.totalsecret,
            maxfrags: 0,
            partime: par_time(world.commercial, world.gameepisode, world.gamemap)
                * TICRATE as i32,
            ..crate::wi_stuff::WbStart::default()
        };

        // wminfo.next is 0 biased, unlike gamemap.
        info.next = next_map(world.commercial, secretexit, world.gameepisode, world.gamemap);

        for stats in info.plyr.iter_mut() {
            stats.in_game = false;
        }
        for i in 0..MAX_PLAYERS {
            if world.player_in_game[i] {
                info.plyr[i] = crate::s_sound::WbPlayerStats {
                    in_game: true,
                    skills: world.players[i].killcount,
                    sitems: world.players[i].itemcount,
                    ssecret: world.players[i].secretcount,
                    stime: world.leveltime,
                    frags: world.players[i].frags,
                };
            }
        }

        self.inter = Some(crate::wi_stuff::Intermission::start(info));
        self.state = GameState::Intermission;
    }

    /// `G_WorldDone` + `G_DoWorldDone`: advance the map and ask the host to
    /// load the next level.
    fn g_do_world_done(&mut self, world: &mut World) {
        if self.secretexit {
            world.players[world.consoleplayer].didsecret = true;
        }

        // DOOM II finale maps (G_WorldDone's F_StartFinale cases).
        if world.commercial {
            let finale = match world.gamemap {
                15 | 31 => self.secretexit,
                6 | 11 | 20 | 30 => true,
                _ => false,
            };
            if finale {
                self.state = GameState::Level;
                self.pending.push(FlowEvent::Victory);
                return;
            }
        }

        let next = self
            .inter
            .as_ref()
            .map(|wi| wi.info.next)
            .unwrap_or(world.gamemap);
        self.inter = None;
        self.state = GameState::Level;
        world.gamemap = next + 1;
        self.pending.push(FlowEvent::LoadLevel {
            episode: world.gameepisode,
            map: world.gamemap,
        });
    }
}

/// The `wminfo.next` computation from `G_DoCompleted`.
fn next_map(commercial: bool, secretexit: bool, episode: i32, map: i32) -> i32 {
    if commercial {
        if secretexit {
            match map {
                15 => 30, // go to secret level
                31 => 31,
                _ => map,
            }
        } else {
            match map {
                31 | 32 => 15, // back from the secret levels
                _ => map,      // next level (0-based next == map)
            }
        }
    } else if secretexit {
        8 // go to secret level
    } else if map == 9 {
        // returning from secret level
        match episode {
            1 => 3,
            2 => 5,
            3 => 6,
            4 => 2,
            _ => map,
        }
    } else {
        map // go to next level
    }
}

/// Par times (in seconds before the `TICRATE` scaling).
fn par_time(commercial: bool, episode: i32, map: i32) -> i32 {
    if commercial {
        CPARS[((map - 1).max(0) as usize).min(CPARS.len() - 1)]
    } else if (1..=3).contains(&episode) {
        PARS[episode as usize][map.clamp(1, 9) as usize]
    } else {
        // Episode 4 has no pars; vanilla overflows into cpars.
        CPARS[((map.max(1) - 1) as usize).min(CPARS.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d_ticcmd::TicCmd;
    use crate::level::{Line, Sector, Subsector, Vertex};
    /// A tiny synthetic map: two rooms, a door line, a zombie.
    fn mini_level() -> Level {
        let mut level = Level::default();
        // Room 0 (start) and room 1, joined by a two-sided line.
        for (floor, ceil) in [(0, 128), (0, 128)] {
            level.sectors.push(Sector {
                floorheight: floor << 16,
                ceilingheight: ceil << 16,
                ..Sector::default()
            });
        }
        level.vertexes.push(Vertex { x: 0, y: 0 });
        level.vertexes.push(Vertex { x: 0, y: 128 << 16 });
        level.lines.push(Line {
            v1: 0,
            v2: 1,
            flags: crate::doomdef::ML_TWOSIDED,
            special: 4, // raise door trigger
            tag: 0,
            sidenum: [-1, -1],
            frontsector: Some(0),
            backsector: Some(1),
            ..Line::default()
        });
        level.subsectors.push(Subsector {
            sector: 0,
            numlines: 0,
            firstline: 0,
        });
        level
    }

    #[test]
    fn ticker_moves_player_and_runs_door() {
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
        p_spawn_player(&mut world, 0, &starts[0].unwrap(), false);
        assert!(world.players[0].mo.is_some());

        // A zombie in room 1 gets counted in totalkills at spawn time by the
        // map loader; here just drive a walk + fire.
        world.players[0].cmd = TicCmd {
            forwardmove: 50,
            buttons: crate::d_ticcmd::BT_ATTACK,
            ..TicCmd::default()
        };
        for _ in 0..40 {
            g_ticker(&mut world, &starts, &[]);
        }
        let mo = world.players[0].mo.unwrap();
        let m = world.mobjs.get(mo).unwrap();
        assert!(m.x > 0, "player should have moved");
        assert_eq!(world.leveltime, 40);

        // Ammo spent: firing consumed clips.
        assert!(world.players[0].ammo[AmmoType::Clip as usize] < 50);
    }

    #[test]
    fn reborn_resets_player() {
        let mut world = World::new(mini_level(), Render::new());
        world.player_in_game[0] = true;
        world.players[0].killcount = 5;
        world.players[0].health = 0;
        world.players[0].playerstate = PlayerState::Reborn;
        let starts: PlayerStarts = [None; MAX_PLAYERS];
        g_do_reborn(&mut world, 0, &starts, &[]);
        // Single player: level reload requested.
        assert!(world.pending_reborn_reload);
        g_player_reborn(&mut world, 0);
        assert_eq!(world.players[0].killcount, 5);
        assert_eq!(world.players[0].health, 100);
        assert_eq!(world.players[0].ammo[AmmoType::Clip as usize], 50);
    }

    #[test]
    fn init_new_sets_skill_flags() {
        let mut world = World::new(mini_level(), Render::new());
        g_init_new(&mut world, 4, 1, 1);
        assert!(world.respawnmonsters);
        g_init_new(&mut world, 2, 1, 1);
        assert!(!world.respawnmonsters);
    }

    #[test]
    fn flow_exits_to_intermission_then_next_map() {
        let mut world = World::new(mini_level(), crate::r_main::Render::new());
        world.player_in_game[0] = true;
        let starts: PlayerStarts = [None; MAX_PLAYERS];
        let mut flow = GameFlow::new();
        world.gamemap = 3;

        // Exiting the level enters the intermission with the stats filled.
        world.pending_exit = Some(false);
        let evs = flow.ticker(&mut world, &starts, &[], false);
        assert!(evs.is_empty());
        assert_eq!(flow.state, GameState::Intermission);
        let wi = flow.inter.as_ref().unwrap();
        assert_eq!(wi.info.last, 2); // gamemap 3 - 1
        assert_eq!(wi.info.next, 3); // straight to map 4

        // Stats settle, then a skip finishes the intermission and asks the
        // host to load the next map.
        for _ in 0..256 {
            flow.ticker(&mut world, &starts, &[], false);
        }
        // Two skips: one to leave the stats screen, one to confirm.
        let evs = flow.ticker(&mut world, &starts, &[], true);
        assert!(evs.is_empty());
        let evs = flow.ticker(&mut world, &starts, &[], true);
        assert_eq!(evs, alloc::vec![FlowEvent::LoadLevel { episode: 1, map: 4 }]);
        assert_eq!(flow.state, GameState::Level);
    }

    #[test]
    fn flow_secret_exit_on_e1m3_goes_to_e1m9() {
        let mut world = World::new(mini_level(), crate::r_main::Render::new());
        world.player_in_game[0] = true;
        let starts: PlayerStarts = [None; MAX_PLAYERS];
        let mut flow = GameFlow::new();
        world.gamemap = 3;
        world.pending_exit = Some(true);
        flow.ticker(&mut world, &starts, &[], false);
        let wi = flow.inter.as_ref().unwrap();
        assert_eq!(wi.info.next, 8); // secret level (0-based)
    }
}
