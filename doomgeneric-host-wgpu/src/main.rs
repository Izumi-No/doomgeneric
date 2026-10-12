//! A playable, windowed host: winit for the window and input, wgpu for
//! presentation. It drives the same [`Game`] session the headless host uses.
//!
//! Usage:
//! ```text
//! doomgeneric-host-wgpu [IWAD.wad] [-warp E,M | -warp M] [-skill N]
//! ```
//!
//! Controls: arrows / WASD move, A/D strafe, mouse turns, Shift runs,
//! Ctrl or LMB fires, Space/E opens, 1-7 selects a weapon, Esc quits.

mod game;
mod gfx;

use std::sync::Arc;
use std::time::{Duration, Instant};

use doomgeneric_core::d_loop::{keys, InputState};
use doomgeneric_core::f_finale::Finale;
use doomgeneric_core::g_game::FlowEvent;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use game::Game;
use gfx::Gfx;

/// One 35 Hz game tic.
const TIC_DT: Duration = Duration::from_nanos(1_000_000_000 / 35);
/// Don't try to run more than this many tics to catch up after a stall.
const MAX_CATCHUP: usize = 5;
/// How long the finale plays before it holds on the last frame.
const FINALE_TICS: i32 = 35 * 8;

struct State {
    window: Arc<Window>,
    gfx: Gfx,
    game: Game,
    input: InputState,
    last: Instant,
    acc: Duration,
    finale: Option<Finale>,
    finale_ticks: i32,
    /// Mouse device events are global; only feed them to the game while the
    /// window is focused, and grab the pointer meanwhile.
    focused: bool,
}

struct App {
    wad_path: Option<String>,
    skill: i32,
    episode: i32,
    map: i32,
    state: Option<State>,
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

impl App {
    fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let wad_path = args
            .iter()
            .find(|a| a.to_ascii_lowercase().ends_with(".wad"))
            .cloned()
            .or_else(|| {
                ["DOOM2.WAD", "DOOM (2).WAD", "doom1.wad", "DOOM.WAD"]
                    .iter()
                    .find(|p| std::path::Path::new(p).exists())
                    .map(|p| p.to_string())
            });

        let mut skill = 2;
        if let Some(i) = args.iter().position(|a| a == "-skill") {
            if let Some(v) = args.get(i + 1).and_then(|v| v.parse().ok()) {
                skill = v;
            }
        }

        let (mut episode, mut map) = (1, 1);
        if let Some((e, m)) = parse_warp(&args) {
            episode = e;
            map = m;
        }

        App {
            wad_path,
            skill,
            episode,
            map,
            state: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        event_loop.set_control_flow(ControlFlow::Poll);

        let attrs = Window::default_attributes()
            .with_title("doomgeneric (wgpu)")
            .with_inner_size(LogicalSize::new(960u32, 600u32));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let gfx = Gfx::new(window.clone());

        let wad_path = match &self.wad_path {
            Some(p) => p.clone(),
            None => {
                eprintln!("no IWAD found: pass one on the command line");
                event_loop.exit();
                return;
            }
        };
        let data = match std::fs::read(&wad_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("cannot read {wad_path}: {e}");
                event_loop.exit();
                return;
            }
        };

        let game = match Game::new(data, self.skill, self.episode, self.map) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("cannot start game: {e}");
                event_loop.exit();
                return;
            }
        };
        let title = format!(
            "doomgeneric (wgpu) - {:?} E{}M{} skill {}",
            game.mode(),
            game.episode(),
            game.map(),
            game.skill()
        );
        window.set_title(&title);
        eprintln!("[wgpu] {title}");

        self.state = Some(State {
            window,
            gfx,
            game,
            input: InputState::default(),
            last: Instant::now(),
            acc: Duration::ZERO,
            finale: None,
            finale_ticks: 0,
            focused: false,
        });
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if let Some(s) = &mut self.state {
                if s.focused {
                    s.input.mouse.dx += delta.0 as i32;
                    s.input.mouse.dy += delta.1 as i32;
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(s) = &mut self.state else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => s.gfx.resize(size.width, size.height),
            WindowEvent::Focused(focused) => {
                s.focused = focused;
                let grab = if focused {
                    s.window
                        .set_cursor_grab(winit::window::CursorGrabMode::Locked)
                        .or_else(|_| {
                            s.window
                                .set_cursor_grab(winit::window::CursorGrabMode::Confined)
                        })
                } else {
                    s.window.set_cursor_grab(winit::window::CursorGrabMode::None)
                };
                let _ = grab;
                s.window.set_cursor_visible(!focused);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.logical_key == Key::Named(NamedKey::Escape)
                    && event.state == ElementState::Pressed
                {
                    event_loop.exit();
                    return;
                }
                if let Some(bit) = key_bit(&event.logical_key) {
                    if event.state == ElementState::Pressed {
                        s.input.keys |= bit;
                    } else {
                        s.input.keys &= !bit;
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let mask = match button {
                    MouseButton::Left => 0x01,
                    MouseButton::Right => 0x02,
                    MouseButton::Middle => 0x04,
                    _ => 0,
                };
                if mask != 0 {
                    if state == ElementState::Pressed {
                        s.input.mouse.buttons |= mask;
                    } else {
                        s.input.mouse.buttons &= !mask;
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                s.acc += now - s.last;
                s.last = now;
                let cap = TIC_DT * MAX_CATCHUP as u32;
                if s.acc > cap {
                    s.acc = cap;
                }

                let mut ticks = 0;
                while s.acc >= TIC_DT && ticks < MAX_CATCHUP {
                    s.acc -= TIC_DT;
                    ticks += 1;
                    if let Some(fin) = &mut s.finale {
                        if s.finale_ticks < FINALE_TICS {
                            fin.ticker(false);
                            s.finale_ticks += 1;
                        }
                    } else {
                        let events = s.game.tick_input(&mut s.input);
                        if events.iter().any(|e| matches!(e, FlowEvent::Victory)) {
                            s.game.start_finale_music();
                            s.finale = Some(s.game.start_finale());
                            s.finale_ticks = 0;
                        }
                    }
                }

                if let Some(fin) = &mut s.finale {
                    s.game.draw_finale(fin);
                } else {
                    s.game.render();
                }
                s.gfx.upload(s.game.pixels());
                s.gfx.present();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(s) = &self.state {
            s.window.request_redraw();
        }
    }
}

/// Map a physical key to an engine key bit.
fn key_bit(key: &Key) -> Option<u32> {
    match key {
        Key::Named(named) => named_bit(*named),
        Key::Character(c) => char_bit(c.as_str()),
        _ => None,
    }
}

fn named_bit(key: NamedKey) -> Option<u32> {
    Some(match key {
        NamedKey::ArrowRight => keys::RIGHT,
        NamedKey::ArrowLeft => keys::LEFT,
        NamedKey::ArrowUp => keys::UP,
        NamedKey::ArrowDown => keys::DOWN,
        NamedKey::Space => keys::USE,
        NamedKey::Shift => keys::SPEED,
        NamedKey::Control => keys::FIRE,
        NamedKey::Alt => keys::STRAFE,
        _ => return None,
    })
}

fn char_bit(c: &str) -> Option<u32> {
    Some(match c.to_ascii_lowercase().as_str() {
        "w" => keys::UP,
        "s" => keys::DOWN,
        "a" => keys::STRAFE_LEFT,
        "d" => keys::STRAFE_RIGHT,
        "e" => keys::USE,
        "f" => keys::FIRE,
        "1" => keys::WEAPON_1,
        "2" => keys::WEAPON_2,
        "3" => keys::WEAPON_3,
        "4" => keys::WEAPON_4,
        "5" => keys::WEAPON_5,
        "6" => keys::WEAPON_6,
        "7" => keys::WEAPON_7,
        _ => return None,
    })
}

fn main() {
    let event_loop = EventLoop::new().expect("create event loop");
    let mut app = App::from_args();
    if app.wad_path.is_none() {
        eprintln!("usage: doomgeneric-host-wgpu [IWAD.wad] [-warp E,M | -warp M] [-skill N]");
        return;
    }
    event_loop.run_app(&mut app).expect("run winit event loop");
}
