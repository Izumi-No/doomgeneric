//! The play session: IWAD bootstrap plus the per-tic engine drive.
//!
//! This is the host-side counterpart of vanilla's `D_DoomLoop` body. It owns
//! the [`WadManager`], the [`World`], the per-level renderer and every screen
//! the engine draws into. A host only has to:
//!
//! 1. build one with the IWAD bytes,
//! 2. feed an [`InputState`] once per tic ([`Game::tick_input`]),
//! 3. upload [`Game::render`]'s indexed-to-ARGB pixels to its display.

use doomgeneric_core::d_loop::{g_build_ticcmd, InputState, TurnState};
use doomgeneric_core::d_main::{self, GameMode};
use doomgeneric_core::d_ticcmd::TicCmd;
use doomgeneric_core::doomdef::MAX_PLAYERS;
use doomgeneric_core::f_finale::Finale;
use doomgeneric_core::g_game::{self, FlowEvent, GameFlow, PlayerStarts};
use doomgeneric_core::hu_stuff::Hud;
use doomgeneric_core::level::{Level, Thing};
use doomgeneric_core::p_switch::SwitchList;
use doomgeneric_core::p_world::World;
use doomgeneric_core::r_frame::FrameRenderer;
use doomgeneric_core::r_main::Render;
use doomgeneric_core::r_things::{init_sprite_defs, SpriteDefs};
use doomgeneric_core::s_sound;
use doomgeneric_core::st_stuff::{self, StatusBar};
use doomgeneric_core::v_video;
use doomgeneric_core::wad::{LumpName, WadManager};

pub const WIDTH: usize = v_video::SCREENWIDTH;
pub const HEIGHT: usize = v_video::SCREENHEIGHT;
pub const VIEW_HEIGHT: usize = HEIGHT - v_video::ST_HEIGHT;

/// One running game, from IWAD to the current level.
pub struct Game {
    world: World,
    manager: WadManager,
    mode: GameMode,
    commercial: bool,
    sprite_defs: SpriteDefs,
    fr: FrameRenderer,
    status: StatusBar,
    hud: Hud,
    flow: GameFlow,
    starts: PlayerStarts,
    dm_starts: Vec<Thing>,
    palette: [u8; 768],
    screen: [u8; WIDTH * HEIGHT],
    pixels: Vec<u32>,
    turn: TurnState,
    tic: i32,
    skill: i32,
    episode: i32,
    map: i32,
}

impl Game {
    /// Bootstrap an IWAD and warp straight into `(episode, map)` / `MAPxx`.
    pub fn new(wad: Vec<u8>, skill: i32, episode: i32, map: i32) -> Result<Self, String> {
        let mut manager = WadManager::new();
        manager.add(wad).map_err(|e| format!("{e:?}"))?;

        let mode = d_main::identify_iwad(&manager);
        if mode == GameMode::Indetermined {
            return Err(
                "unrecognized IWAD: no E1M1/E3M1/E4M1/MAP01 lumps (pre-release \
                 alpha IWADs, with M_AREAS/M_LINES maps, are not supported)"
                    .into(),
            );
        }
        let commercial = mode == GameMode::Commercial;
        let map_name = d_main::map_name(mode, episode, map).ok_or("bad map name")?;
        let map_name = String::from_utf8_lossy(&map_name).into_owned();
        let map_lump = d_main::find_map_lump(&manager, &map_name)
            .ok_or_else(|| format!("map lump {map_name} not found"))?;

        let first_sprite = manager
            .check_num_for_name(&LumpName::from_bytes(b"S_START"))
            .map(|i| i + 1)
            .unwrap_or(0);
        let last_sprite = manager
            .check_num_for_name(&LumpName::from_bytes(b"S_END"))
            .map(|i| i.saturating_sub(1))
            .unwrap_or_else(|| manager.num_lumps());
        let sprite_defs = init_sprite_defs(&manager, first_sprite, last_sprite)
            .map_err(|e| format!("{e:?}"))?;

        let mut world = World::new(Level::default(), Render::new());
        world.render.set_view_size(10, 0);
        world.render.execute_set_view_size();
        g_game::g_init_new(&mut world, skill, episode, map);
        world.switches = SwitchList::init(commercial, mode == GameMode::Registered);
        world.shareware = mode == GameMode::Shareware;

        let mut starts: PlayerStarts = [None; MAX_PLAYERS];
        let mut dm_starts: Vec<Thing> = Vec::new();
        g_game::g_do_load_level(
            &mut world,
            &manager,
            map_lump,
            commercial,
            &mut starts,
            &mut dm_starts,
        )
        .map_err(|e| format!("{e:?}"))?;

        let fr = FrameRenderer::new(&manager, &world.level, sprite_defs.clone())
            .map_err(|e| format!("{e:?}"))?;
        world.texture_heights = fr
            .data
            .textures
            .iter()
            .enumerate()
            .map(|(i, t)| (LumpName(t.name), fr.data.texture_height[i]))
            .collect();
        world.sounds.start_music(s_sound::s_level_music(commercial, episode, map));

        let palette = v_video::load_palette(&manager).ok_or("PLAYPAL missing")?;

        Ok(Game {
            world,
            manager,
            mode,
            commercial,
            sprite_defs,
            fr,
            status: StatusBar::new(0),
            hud: Hud::new(),
            flow: GameFlow::new(),
            starts,
            dm_starts,
            palette,
            screen: [0u8; WIDTH * HEIGHT],
            pixels: vec![0u32; WIDTH * HEIGHT],
            turn: TurnState::default(),
            tic: 0,
            skill,
            episode,
            map,
        })
    }

    /// Build the local player's command from an input snapshot, then run one
    /// game tic.
    pub fn tick_input(&mut self, input: &mut InputState) -> Vec<FlowEvent> {
        let cmd = g_build_ticcmd(input, &mut self.turn, 1);
        self.tick(cmd)
    }

    /// Run one game tic with an explicit command (demo playback, netgames).
    /// `FlowEvent::LoadLevel` is handled internally; `Victory` is returned so
    /// the host can show the finale.
    pub fn tick(&mut self, cmd: TicCmd) -> Vec<FlowEvent> {
        self.world.players[0].cmd = cmd;
        self.tic += 1;

        let mut forward = Vec::new();
        let events = self
            .flow
            .ticker(&mut self.world, &self.starts, &self.dm_starts, false);
        for ev in events {
            match ev {
                FlowEvent::LoadLevel { episode, map } => self.load_level(episode, map),
                FlowEvent::Victory => forward.push(FlowEvent::Victory),
            }
        }

        let rng_face = self.world.rng.p.next();
        self.status.ticker(
            &self.world.players[0],
            &self.world.mobjs,
            self.world.deathmatch != 0,
            rng_face,
        );
        self.hud.ticker(&mut self.world.players);
        for _ in self.world.sounds.drain_sounds() {}
        forward
    }

    fn load_level(&mut self, episode: i32, map: i32) {
        let Some(name) = d_main::map_name(self.mode, episode, map) else {
            return;
        };
        let name = String::from_utf8_lossy(&name).into_owned();
        let Some(lump) = d_main::find_map_lump(&self.manager, &name) else {
            return;
        };
        if g_game::g_do_load_level(
            &mut self.world,
            &self.manager,
            lump,
            self.commercial,
            &mut self.starts,
            &mut self.dm_starts,
        )
        .is_err()
        {
            return;
        }
        if let Ok(mut fr) =
            FrameRenderer::new(&self.manager, &self.world.level, self.sprite_defs.clone())
        {
            fr.displayplayer = 0;
            self.world.texture_heights = fr
                .data
                .textures
                .iter()
                .enumerate()
                .map(|(i, t)| (LumpName(t.name), fr.data.texture_height[i]))
                .collect();
            self.fr = fr;
        }
        self.world
            .sounds
            .start_music(s_sound::s_level_music(self.commercial, episode, map));
        self.episode = episode;
        self.map = map;
    }

    /// Draw the 3D view plus the status bar and return the ARGB frame.
    pub fn render(&mut self) -> &[u32] {
        self.screen[..].fill(0);
        self.fr
            .render_view(&mut self.world, &self.manager, &mut self.screen);
        st_stuff::st_draw(
            &self.manager,
            &self.status.state,
            &mut self.screen[WIDTH * VIEW_HEIGHT..],
        );
        v_video::present_screen(&self.screen, &self.palette, &mut self.pixels);
        &self.pixels
    }

    /// Start the end-of-episode finale for the level just finished.
    pub fn start_finale(&self) -> Finale {
        Finale::start(self.commercial, self.world.gameepisode, self.world.gamemap)
    }

    /// Draw one frame of the finale into the shared pixel buffer.
    pub fn draw_finale(&mut self, fin: &mut Finale) {
        self.screen[..].fill(0);
        fin.draw(&self.manager, &mut self.screen);
        v_video::present_screen(&self.screen, &self.palette, &mut self.pixels);
    }

    pub fn start_finale_music(&mut self) {
        self.world
            .sounds
            .start_music(doomgeneric_core::sounds::Music::mus_victor);
    }

    pub fn pixels(&self) -> &[u32] {
        self.pixels.as_slice()
    }

    pub fn mode(&self) -> GameMode {
        self.mode
    }

    pub fn skill(&self) -> i32 {
        self.skill
    }

    pub fn episode(&self) -> i32 {
        self.episode
    }

    pub fn map(&self) -> i32 {
        self.map
    }
}
