//! The reference host: a playable, headless driver.
//!
//! Everything the engine needs comes through the seams (`FileSystem` for the
//! WAD, `Renderer` for frames, `Input` for keys, `Audio` for the sound
//! queue): this host feeds a scripted `TicCmd` stream to `G_Ticker`, renders
//! the 3D view plus the status bar through the same palette path vanilla
//! uses, and dumps a PPM every second for inspection.

use std::convert::Infallible;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use doomgeneric_core::d_loop::{g_build_ticcmd, keys, InputState, TurnState};
use doomgeneric_core::d_main::{self, GameMode};
use doomgeneric_core::f_finale::Finale;
use doomgeneric_core::m_demo;
use doomgeneric_core::g_game::{self, FlowEvent, GameFlow, PlayerStarts};
use doomgeneric_core::level::Thing;
use doomgeneric_core::p_switch::SwitchList;
use doomgeneric_core::r_frame::FrameRenderer;
use doomgeneric_core::r_things::{init_sprite_defs, SpriteDefs};
use doomgeneric_core::s_sound;
use doomgeneric_core::st_stuff;
use doomgeneric_core::v_video;
use doomgeneric_core::wad::WadManager;
use doomgeneric_core::{Audio, Clock, FileSystem, Frame, Input, KeyEvent, Pixel, Renderer};

const WIDTH: usize = v_video::SCREENWIDTH;
const HEIGHT: usize = v_video::SCREENHEIGHT;
const VIEW_HEIGHT: usize = HEIGHT - v_video::ST_HEIGHT;

struct HeadlessHost {
    start: Instant,
    frame: Vec<Pixel>,
    frames_drawn: u32,
    sounds_played: u32,
}

impl HeadlessHost {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            frame: vec![0; WIDTH * HEIGHT],
            frames_drawn: 0,
            sounds_played: 0,
        }
    }

    fn write_ppm(&self, path: &str) -> io::Result<()> {
        let mut f = File::create(path)?;
        writeln!(f, "P6\n{} {}\n255", WIDTH, HEIGHT)?;
        for px in &self.frame {
            let r = ((px >> 16) & 0xff) as u8;
            let g = ((px >> 8) & 0xff) as u8;
            let b = (*px & 0xff) as u8;
            f.write_all(&[r, g, b])?;
        }
        Ok(())
    }
}

impl Renderer for HeadlessHost {
    type Error = Infallible;

    fn draw_frame(&mut self, frame: Frame<'_>) -> Result<(), Self::Error> {
        self.frame.copy_from_slice(&frame.pixels[..WIDTH * HEIGHT]);
        self.frames_drawn += 1;
        Ok(())
    }

    fn set_window_title(&mut self, title: &str) -> Result<(), Self::Error> {
        eprintln!("[headless] window title: {title}");
        Ok(())
    }
}

impl Input for HeadlessHost {
    fn poll_key(&mut self) -> Option<KeyEvent> {
        None
    }
}

impl Audio for HeadlessHost {
    fn submit_samples(&mut self, _samples: &[i16]) {}
}

impl Clock for HeadlessHost {
    fn sleep_ms(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }

    fn ticks_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }
}

impl doomgeneric_core::FileSystem for HeadlessHost {
    type Error = io::Error;

    fn read(&mut self, path: &str) -> Result<Vec<u8>, Self::Error> {
        fs::read(path)
    }

    fn exists(&mut self, path: &str) -> bool {
        Path::new(path).exists()
    }
}

/// The scripted player: an [`InputState`] fed through `G_BuildTiccmd`, the
/// same path a real window host would take with raw keyboard/mouse events.
fn scripted_input(tic: i32) -> InputState {
    if std::env::var("DG_STILL").is_ok() {
        return InputState::default();
    }
    let phase = tic % (35 * 4);
    let mut st = InputState {
        keys: keys::UP | keys::SPEED,
        ..InputState::default()
    };
    // Sweep the view left/right over four seconds.
    if phase / 35 == 1 {
        st.keys |= keys::LEFT;
    } else if phase / 35 == 3 {
        st.keys |= keys::RIGHT;
    }
    // Fire in bursts during the second half of each sweep.
    if (phase / 18) % 2 == 0 {
        st.keys |= keys::FIRE;
    }
    st
}

/// `-warp M`, `-warp E,M`, `-warpM`, `-warpE,M`, or the vanilla separate
/// forms `-warp M` / `-warp E M`.
fn parse_warp(args: &[String]) -> Option<(i32, i32)> {
    let i = args.iter().position(|a| a.starts_with("-warp"))?;
    let nums: Vec<i32> = args[i][5..]
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    match nums.as_slice() {
        [e, m] => Some((*e, *m)),
        [m] => Some((1, *m)),
        _ => {
            let mut rest: Vec<i32> = args[i + 1..]
                .iter()
                .map(|a| a.trim().parse::<i32>())
                .map_while(Result::ok)
                .take(2)
                .collect();
            match rest.len() {
                2 => Some((rest[0], rest[1])),
                1 => Some((1, rest.remove(0))),
                _ => None,
            }
        }
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let mut host = HeadlessHost::new();
    host.set_window_title("doomgeneric-headless")
        .map_err(|e: Infallible| e.to_string())?;

    // ---- WAD via the FileSystem seam ------------------------------------
    // Prefer an explicitly passed .wad; otherwise pick a known retail IWAD
    // (the checked-in DOOM.WAD is a pre-release alpha with M_* lumps).
    let wad_path = args
        .iter()
        .find(|a| a.to_ascii_lowercase().ends_with(".wad"))
        .cloned()
        .or_else(|| {
            ["DOOM2.WAD", "DOOM (2).WAD", "doom1.wad"]
                .iter()
                .find(|p| Path::new(p).exists())
                .map(|p| p.to_string())
        })
        .ok_or("no IWAD found")?;
    let data = host.read(&wad_path).map_err(|e| e.to_string())?;
    eprintln!("[headless] {} is {} bytes", wad_path, data.len());

    let mut manager = WadManager::new();
    manager.add(data).map_err(|e| format!("{e:?}"))?;

    // ---- engine bootstrap ------------------------------------------------
    let mode = d_main::identify_iwad(&manager);
    let commercial = mode == GameMode::Commercial;
    eprintln!("[headless] IWAD identified as {mode:?} (commercial={commercial})");
    let (episode, map, skill) = parse_warp(&args).map_or((1, 1, 2), |(e, m)| (e, m, 2));

    let map_bytes = d_main::map_name(mode, episode, map).ok_or("bad map name")?;
    let map_name = String::from_utf8_lossy(&map_bytes);
    let map_lump = d_main::find_map_lump(&manager, &map_name)
        .ok_or_else(|| format!("map lump {map_name} not found"))?;
    eprintln!("[headless] starting at {map_name} (lump {map_lump})");

    // Sprite lumps live between the S_START/S_END markers (vanilla
    // R_InitSpriteDefs range).
    use doomgeneric_core::wad::LumpName;
    let first_sprite = manager
        .check_num_for_name(&LumpName::from_bytes(b"S_START"))
        .map(|i| i + 1)
        .unwrap_or(0);
    let last_sprite = manager
        .check_num_for_name(&LumpName::from_bytes(b"S_END"))
        .map(|i| i.saturating_sub(1))
        .unwrap_or_else(|| manager.num_lumps());
    let sprite_defs: SpriteDefs =
        init_sprite_defs(&manager, first_sprite, last_sprite).map_err(|e| format!("{e:?}"))?;

    let mut world = doomgeneric_core::p_world::World::new(
        doomgeneric_core::level::Level::default(),
        doomgeneric_core::r_main::Render::new(),
    );
    world.render.set_view_size(10, 0);
    world.render.execute_set_view_size();
    g_game::g_init_new(&mut world, skill, episode, map);
    world.switches = SwitchList::init(commercial, mode == GameMode::Registered);
    world.shareware = mode == GameMode::Shareware;

    let mut starts: PlayerStarts = [None; doomgeneric_core::doomdef::MAX_PLAYERS];
    let mut dm_starts: Vec<Thing> = Vec::new();
    g_game::g_do_load_level(&mut world, &manager, map_lump, commercial, &mut starts, &mut dm_starts)
        .map_err(|e| format!("{e:?}"))?;
    eprintln!(
        "[headless] level loaded: {} sectors, {} things, {} kills / {} items / {} secrets",
        world.level.sectors.len(),
        world.level.things.len(),
        world.totalkills,
        world.totalitems,
        world.totalsecret
    );

    let mut fr = FrameRenderer::new(&manager, &world.level, sprite_defs.clone())
        .map_err(|e| format!("{e:?}"))?;
    // The host feeds the engine's name->height table from RenderData.
    world.texture_heights = fr
        .data
        .textures
        .iter()
        .enumerate()
        .map(|(i, t)| (doomgeneric_core::wad::LumpName(t.name), fr.data.texture_height[i]))
        .collect();
    world.sounds.start_music(s_sound::s_level_music(commercial, episode, map));

    let mut status = st_stuff::StatusBar::new(0);
    let mut hud = doomgeneric_core::hu_stuff::Hud::new();

    // Demo record/playback over the same byte format vanilla .lmp files use.
    let mut demo_out: Option<std::fs::File> = None;
    let mut demo_in: Option<Vec<u8>> = None;
    let mut demo_pos = 0usize;
    let demo_players = 1usize;
    if let Some(i) = args.iter().position(|a| a == "-record") {
        if let Some(path) = args.get(i + 1) {
            let header = m_demo::DemoHeader {
                skill,
                episode,
                map,
                deathmatch: 0,
                respawn: false,
                fast: false,
                nomonsters: false,
                consoleplayer: 0,
                playeringame: [true, false, false, false],
            };
            // full-resolution turns: playbacks replay exactly
            let mut bytes = header.to_bytes();
            bytes[0] = m_demo::DEMO_VERSION_LONGTICS;
            std::fs::write(path, bytes).map_err(|e| e.to_string())?;
            demo_out = Some(
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(path)
                        .map_err(|e| e.to_string())?,
                );
            eprintln!("[headless] recording demo to {path}");
        }
    }
    if let Some(i) = args.iter().position(|a| a == "-playdemo") {
        if let Some(path) = args.get(i + 1) {
            let data = std::fs::read(path).map_err(|e| e.to_string())?;
            if let Some(header) = m_demo::DemoHeader::parse(&data) {
                eprintln!(
                    "[headless] playing demo: skill {} E{}M{}",
                    header.skill, header.episode, header.map
                );
                demo_pos = 13;
                demo_in = Some(data);
            } else {
                return Err("not a v1.9 demo".to_string());
            }
        }
    }
    let mut flow = GameFlow::new();
    let mut screen = [0u8; WIDTH * HEIGHT];
    let palette = v_video::load_palette(&manager).ok_or("PLAYPAL missing")?;

    // ---- optional finale preview (the Victory flow uses the same code) ----
    if args.iter().any(|a| a == "-finale") {
        let mut fin = Finale::start(commercial, world.gameepisode, world.gamemap);
        for _ in 0..35 * 6 {
            fin.ticker(false);
            fin.draw(&manager, &mut screen);
            let mut pixels = vec![0u32; WIDTH * HEIGHT];
            v_video::present_screen(&screen, &palette, &mut pixels);
            host.draw_frame(Frame {
                width: WIDTH,
                height: HEIGHT,
                pixels: &pixels,
            })
            .map_err(|e: Infallible| e.to_string())?;
        }
        host.write_ppm("finale.ppm").map_err(|e| e.to_string())?;
        eprintln!("[headless] finale preview written ({} chars typed)", fin.text_shown());
        return Ok(());
    }

    // ---- the title screen --------------------------------------------------
    if let Ok(data) = manager.lump_data_by_name(&doomgeneric_core::wad::LumpName(*b"TITLEPIC")) {
        let patch = doomgeneric_core::v_video::Patch::from_lump(data);
        screen.fill(0);
        doomgeneric_core::v_video::draw_patch(
            &mut screen,
            WIDTH as i32,
            HEIGHT as i32,
            &patch,
            0,
            0,
        );
        let mut pixels = vec![0u32; WIDTH * HEIGHT];
        v_video::present_screen(&screen, &palette, &mut pixels);
        host.draw_frame(Frame {
            width: WIDTH,
            height: HEIGHT,
            pixels: &pixels,
        })
        .map_err(|e: Infallible| e.to_string())?;
        host.write_ppm("title.ppm").map_err(|e| e.to_string())?;
    }

    let mut turn = TurnState::default();

    // ---- the game loop ----------------------------------------------------
    let demo_tics: i32 = args
        .iter()
        .position(|a| a == "-tics")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(35 * 14);
    for tic in 0..demo_tics {
        world.players[0].cmd = match &demo_in {
            Some(data) => {
                let long = m_demo::DemoHeader::longtics(data);
                m_demo::read_ticcmd_stride(data, &mut demo_pos, demo_players, long)
                    .map(|cmds| cmds[0])
                    .unwrap_or_default()
            }
            None => {
                let mut st = scripted_input(tic);
                g_build_ticcmd(&mut st, &mut turn, 1)
            }
        };
        if let Some(f) = demo_out.as_mut() {
            let mut buf = Vec::new();
            m_demo::write_ticcmd_long(&mut buf, core::slice::from_ref(&world.players[0].cmd));
            use std::io::Write;
            f.write_all(&buf).map_err(|e| e.to_string())?;
        }
        let skip = (tic % 35) == 34; // a "use" tap per second for the flow demo
        for ev in flow.ticker(&mut world, &starts, &dm_starts, skip) {
            match ev {
                FlowEvent::LoadLevel { episode, map } => {
                    let name_bytes =
                        d_main::map_name(mode, episode, map).ok_or("bad map name")?;
                    let name = String::from_utf8_lossy(&name_bytes);
                    match d_main::find_map_lump(&manager, &name) {
                        Some(lump) => {
                            g_game::g_do_load_level(
                                &mut world,
                                &manager,
                                lump,
                                commercial,
                                &mut starts,
                                &mut dm_starts,
                            )
                            .map_err(|e| format!("{e:?}"))?;
                            fr = FrameRenderer::new(&manager, &world.level, sprite_defs.clone())
                                .map_err(|e| format!("{e:?}"))?;
                            fr.displayplayer = 0;
                            eprintln!("[headless] advanced to {name}");
                        }
                        None => {
                            eprintln!("[headless] {name} missing, ending demo");
                            return Ok(());
                        }
                    }
                }
                FlowEvent::Victory => {
                    eprintln!("[headless] episode complete!");
                    // Run the finale for a few seconds of typing, then a
                    // skip, then the art screen.
                    let mut fin = Finale::start(
                        commercial,
                        world.gameepisode,
                        world.gamemap,
                    );
                    world
                        .sounds
                        .start_music(doomgeneric_core::sounds::Music::mus_victor);
                    for _ in 0..35 * 6 {
                        fin.ticker(false);
                        fin.draw(&manager, &mut screen);
                        let mut pixels = vec![0u32; WIDTH * HEIGHT];
                        v_video::present_screen(&screen, &palette, &mut pixels);
                        host.draw_frame(Frame {
                            width: WIDTH,
                            height: HEIGHT,
                            pixels: &pixels,
                        })
                        .map_err(|e: Infallible| e.to_string())?;
                    }
                    host.write_ppm("finale.ppm").map_err(|e| e.to_string())?;
                    for sfx in fin.drain_sounds() {
                        world.sounds.start_sound(sfx);
                    }
                }
            }
        }

        // HUD + status tickers.
        let rng_face = world.rng.p.next();
        status.ticker(
            &world.players[0],
            &world.mobjs,
            world.deathmatch != 0,
            rng_face,
        );
        hud.ticker(&mut world.players);

        // Drain the audio seam.
        for _ in world.sounds.drain_sounds() {
            host.sounds_played += 1;
        }

        // Render one frame per second (35 tics) into the indexed screen,
        // then present through the palette seam.
        let every = std::env::var("DUMPALL").is_ok();
        if tic % 35 == 0 || every {
            screen[..].fill(0);
            fr.render_view(&mut world, &manager, &mut screen);
            st_stuff::st_draw(&manager, &status.state, &mut screen[WIDTH * VIEW_HEIGHT..]);
            let mut pixels = vec![0u32; WIDTH * HEIGHT];
            v_video::present_screen(&screen, &palette, &mut pixels);
            host.draw_frame(Frame {
                width: WIDTH,
                height: HEIGHT,
                pixels: &pixels,
            })
            .map_err(|e: Infallible| e.to_string())?;
            host.write_ppm(&format!("frame-{:02}.ppm", tic / 35))
                .map_err(|e| e.to_string())?;
            if std::env::var("DUMPIDX").is_ok() || every {
                let mut v = format!("P5\n{} {}\n255\n", WIDTH, HEIGHT).into_bytes();
                v.extend_from_slice(&screen);
                std::fs::write(format!("port-{:03}.pgm", tic), v)
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    eprintln!(
        "[headless] demo done: {} tics, {} frames, {} sounds",
        demo_tics, host.frames_drawn, host.sounds_played
    );
    eprintln!(
        "[headless] player: health={}, clip={}/200, pos=({},{}), angle={}deg",
        world.players[0].health,
        world.players[0].ammo[doomgeneric_core::doomdef::AmmoType::Clip as usize],
        world.players[0].mo.map(|m| world.mobjs.get(m).unwrap().x >> 16).unwrap_or(0),
        world.players[0].mo.map(|m| world.mobjs.get(m).unwrap().y >> 16).unwrap_or(0),
        world.players[0].mo.map(|m| (world.mobjs.get(m).unwrap().angle as u64 * 360) >> 32).unwrap_or(0),
    );
    println!(
        "headless host: {WIDTH}x{HEIGHT}, frames={}, sounds={}, ticks={}ms",
        host.frames_drawn,
        host.sounds_played,
        host.ticks_ms()
    );
    Ok(())
}
