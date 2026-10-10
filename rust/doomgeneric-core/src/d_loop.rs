//! Event loop and ticcmd building, port of `d_loop.c`/`d_event.c` plus
//! vanilla `G_BuildTiccmd` (the keyboard/mouse/joystick to `ticcmd_t` glue).
//!
//! The core never touches real input devices: the host feeds an
//! [`InputState`] snapshot (accumulated keys/mouse/joystick deltas, exactly
//! what vanilla's `gamekeydown`/`mousex`/`joyxmove` globals held) and gets
//! the [`TicCmd`] the player meant, with the vanilla two-stage turn
//! acceleration and speed tables.

use crate::d_ticcmd::{TicCmd, BT_ATTACK, BT_CHANGE, BT_USE, BT_WEAPONMASK, BT_WEAPONSHIFT};
use crate::platform::{JoystickState, MouseDelta};

/// `SLOWTURNTICS`: keyboard turning accelerates after this many held tics.
pub const SLOWTURNTICS: i32 = 6;

/// `forwardmove[2]` (walk/run).
pub const FORWARDMOVE: [i32; 2] = [25, 50];
/// `sidemove[2]` (walk/run).
pub const SIDEMOVE: [i32; 2] = [24, 40];
/// `angleturn[3]`: slow, fast, run-adjust turn speeds.
pub const ANGLETURN: [i32; 3] = [640, 1280, 320];

/// The portable key codes the engine understands (`keys_e`); bits in
/// [`InputState::keys`].
pub mod keys {
    pub const RIGHT: u32 = 1;
    pub const LEFT: u32 = 1 << 1;
    pub const UP: u32 = 1 << 2;
    pub const DOWN: u32 = 1 << 3;
    pub const STRAFE: u32 = 1 << 4;
    pub const SPEED: u32 = 1 << 5;
    pub const FIRE: u32 = 1 << 6;
    pub const USE: u32 = 1 << 7;
    pub const STRAFE_LEFT: u32 = 1 << 8;
    pub const STRAFE_RIGHT: u32 = 1 << 9;
    /// Weapon slot keys 1..=7 (`key_weapon` blocks).
    pub const WEAPON_1: u32 = 1 << 16;
    pub const WEAPON_2: u32 = 2 << 16;
    pub const WEAPON_3: u32 = 3 << 16;
    pub const WEAPON_4: u32 = 4 << 16;
    pub const WEAPON_5: u32 = 5 << 16;
    pub const WEAPON_6: u32 = 6 << 16;
    pub const WEAPON_7: u32 = 7 << 16;
    pub const WEAPON_MASK: u32 = 0xf << 16;
    pub const WEAPON_SHIFT: u32 = 16;
}

/// `d_event.h`: event types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvType {
    KeyDown,
    KeyUp,
    Mouse,
    Joy,
    Quit,
}

/// `event_t`.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub ty: EvType,
    /// Key code for key events; button index for mouse/joy.
    pub data1: i32,
    /// Mouse/joystick axis deltas.
    pub data2: i32,
    pub data3: i32,
}

/// `D_PostEvent`/`D_ProcessEvents`: a small event queue the responders
/// (menu, finale, ...) consume each tic.
#[derive(Default)]
pub struct EventQueue {
    queue: alloc::vec::Vec<Event>,
}

impl EventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// `D_PostEvent`.
    pub fn post(&mut self, ev: Event) {
        self.queue.push(ev);
    }

    /// `D_ProcessEvents`: drain, handing each event to `responder`.
    /// Responder returns `true` when the event was consumed.
    pub fn process<F: FnMut(&Event) -> bool>(&mut self, mut responder: F) {
        while !self.queue.is_empty() {
            let ev = self.queue.remove(0);
            if !responder(&ev) {
                // Unhandled events are simply dropped, like vanilla's
                // `D_ProcessEvents` ignoring non-claimed input.
            }
        }
    }
}

/// The `G_BuildTiccmd` world view: keys held down plus accumulated
/// mouse/joystick deltas since the last built tic.
#[derive(Clone, Copy, Debug)]
pub struct InputState {
    /// Held keys, one bit per [`keys`] constant.
    pub keys: u32,
    pub mouse: MouseDelta,
    pub joystick: JoystickState,
    /// `-autorun`: always run.
    pub always_run: bool,
    /// `mouseSensitivity` (default 5).
    pub mouse_sensitivity: i32,
}

impl Default for InputState {
    fn default() -> Self {
        InputState {
            keys: 0,
            mouse: MouseDelta::default(),
            joystick: JoystickState::default(),
            always_run: false,
            mouse_sensitivity: 5,
        }
    }
}

impl InputState {
    /// `D_ProcessEvents` applied: fold one event into the snapshot.
    pub fn apply_event(&mut self, ev: &Event) {
        match ev.ty {
            EvType::KeyDown | EvType::KeyUp => {
                let down = ev.ty == EvType::KeyDown;
                let bit = match ev.data1 {
                    k if k < 32 && k > 0 => Some(1u32 << k),
                    _ => None,
                };
                if let Some(bit) = bit {
                    if down {
                        self.keys |= bit;
                    } else {
                        self.keys &= !bit;
                    }
                }
            }
            EvType::Mouse => {
                // `mousex = ev->data2 * (mouseSensitivity + 5) / 10`
                let sens = (self.mouse_sensitivity + 5) / 10;
                self.mouse.dx += ev.data2 * sens;
                self.mouse.dy += ev.data3 * sens;
                self.mouse.buttons = ev.data1 as u8;
            }
            EvType::Joy => {
                self.joystick.x = ev.data2;
                self.joystick.y = ev.data3;
                self.joystick.buttons = ev.data1 as u8;
                self.joystick.strafe = 0;
            }
            EvType::Quit => {}
        }
    }

    /// Consume the accumulated mouse x delta (the turn source).
    fn take_mouse_turn(&mut self) -> i32 {
        let dx = self.mouse.dx;
        self.mouse.dx = 0;
        dx
    }

    /// Consume the accumulated mouse y delta (the forward source).
    fn take_mouse_forward(&mut self) -> i32 {
        let dy = self.mouse.dy;
        self.mouse.dy = 0;
        dy
    }
}

/// The mutable side of `G_BuildTiccmd` (`turnheld`).
#[derive(Clone, Copy, Debug, Default)]
pub struct TurnState {
    pub held: i32,
}

/// `G_BuildTiccmd`: fold the input snapshot into one player command.
pub fn g_build_ticcmd(state: &mut InputState, turn: &mut TurnState, ticdup: i32) -> TicCmd {
    use keys::*;

    let mut cmd = TicCmd::default();

    let strafe = state.keys & STRAFE != 0
        || state.mouse.buttons & 0x02 != 0
        || state.joystick.buttons & 0x02 != 0;
    let speed = state.always_run
        || state.keys & SPEED != 0
        || state.joystick.buttons & 0x01 != 0;
    let speed = speed as usize;

    let mut forward = 0;
    let mut side = 0;

    // use two stage accelerative turning on the keyboard and joystick
    if state.joystick.x != 0 || state.keys & (RIGHT | LEFT) != 0 {
        turn.held += ticdup;
    } else {
        turn.held = 0;
    }

    let tspeed = if turn.held < SLOWTURNTICS { 2 } else { speed };

    // let movement keys cancel each other out
    if strafe {
        if state.keys & RIGHT != 0 {
            side += SIDEMOVE[speed];
        }
        if state.keys & LEFT != 0 {
            side -= SIDEMOVE[speed];
        }
        if state.joystick.x > 0 {
            side += SIDEMOVE[speed];
        }
        if state.joystick.x < 0 {
            side -= SIDEMOVE[speed];
        }
    } else {
        if state.keys & RIGHT != 0 {
            cmd.angleturn -= ANGLETURN[tspeed] as i16;
        }
        if state.keys & LEFT != 0 {
            cmd.angleturn += ANGLETURN[tspeed] as i16;
        }
        if state.joystick.x > 0 {
            cmd.angleturn -= ANGLETURN[tspeed] as i16;
        }
        if state.joystick.x < 0 {
            cmd.angleturn += ANGLETURN[tspeed] as i16;
        }
    }

    if state.keys & UP != 0 {
        forward += FORWARDMOVE[speed];
    }
    if state.keys & DOWN != 0 {
        forward -= FORWARDMOVE[speed];
    }
    if state.joystick.y < 0 {
        forward += FORWARDMOVE[speed];
    }
    if state.joystick.y > 0 {
        forward -= FORWARDMOVE[speed];
    }

    if state.keys & STRAFE_LEFT != 0 || state.joystick.strafe < 0 {
        side -= SIDEMOVE[speed];
    }
    if state.keys & STRAFE_RIGHT != 0 || state.joystick.strafe > 0 {
        side += SIDEMOVE[speed];
    }

    // mouse: `mousey` feeds forward, `mousex` turns or strafes (the deltas
    // are already sensitivity-scaled by `apply_event`).
    let mouse_dx = state.take_mouse_turn();
    let mouse_dy = state.take_mouse_forward();
    forward += mouse_dy;
    if strafe {
        side += mouse_dx * 2;
    } else {
        cmd.angleturn -= (mouse_dx * 0x8) as i16;
    }

    // `MAXPLMOVE` clamp on both axes.
    const MAXPLMOVE: i32 = 50;
    cmd.forwardmove = forward.clamp(-MAXPLMOVE, MAXPLMOVE) as i8;
    cmd.sidemove = side.clamp(-MAXPLMOVE, MAXPLMOVE) as i8;

    // buttons
    if state.keys & FIRE != 0
        || state.mouse.buttons & 0x01 != 0
        || state.joystick.buttons & 0x04 != 0
    {
        cmd.buttons |= BT_ATTACK;
    }
    if state.keys & USE != 0
        || state.mouse.buttons & 0x04 != 0
        || state.joystick.buttons & 0x08 != 0
    {
        cmd.buttons |= BT_USE;
    }

    // weapon change buttons: keys 1..7
    let weapon = (state.keys & WEAPON_MASK) >> WEAPON_SHIFT;
    if weapon != 0 {
        cmd.buttons |= BT_CHANGE | ((weapon as u8 & BT_WEAPONMASK) << BT_WEAPONSHIFT);
        state.keys &= !WEAPON_MASK;
    }

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_down(bits: u32) -> InputState {
        InputState {
            keys: bits,
            ..InputState::default()
        }
    }

    #[test]
    fn walk_and_run_speeds() {
        let mut st = keys_down(keys::UP);
        let mut t = TurnState::default();
        let cmd = g_build_ticcmd(&mut st, &mut t, 1);
        assert_eq!(cmd.forwardmove, FORWARDMOVE[0] as i8);

        let mut st = keys_down(keys::UP | keys::SPEED);
        let cmd = g_build_ticcmd(&mut st, &mut t, 1);
        assert_eq!(cmd.forwardmove, FORWARDMOVE[1] as i8);
    }

    #[test]
    fn turning_accelerates_after_slowturntics() {
        let mut st = keys_down(keys::LEFT);
        let mut t = TurnState::default();
        let first = g_build_ticcmd(&mut st, &mut t, 1).angleturn;
        for _ in 0..SLOWTURNTICS {
            g_build_ticcmd(&mut st, &mut t, 1);
        }
        let fast = g_build_ticcmd(&mut st, &mut t, 1).angleturn;
        assert_eq!(first, ANGLETURN[2] as i16); // slow turn
        assert_eq!(fast, ANGLETURN[0] as i16); // full speed
    }

    #[test]
    fn strafe_key_converts_turn_to_side() {
        let mut st = keys_down(keys::RIGHT | keys::STRAFE);
        let mut t = TurnState::default();
        let cmd = g_build_ticcmd(&mut st, &mut t, 1);
        assert_eq!(cmd.angleturn, 0);
        assert_eq!(cmd.sidemove, SIDEMOVE[0] as i8);
    }

    #[test]
    fn mouse_turn_is_converted_and_consumed() {
        // Event scaling: dx * (sens+5)/10 = 500 * 1 = 500.
        let mut st = InputState::default();
        st.apply_event(&Event {
            ty: EvType::Mouse,
            data1: 0,
            data2: 500,
            data3: 0,
        });
        let mut t = TurnState::default();
        let cmd = g_build_ticcmd(&mut st, &mut t, 1);
        assert_eq!(cmd.angleturn, -(500 * 0x8) as i16);
        // The delta is consumed.
        let cmd = g_build_ticcmd(&mut st, &mut t, 1);
        assert_eq!(cmd.angleturn, 0);
    }

    #[test]
    fn weapon_and_fire_buttons() {
        let mut st = keys_down(keys::FIRE | keys::WEAPON_2);
        let mut t = TurnState::default();
        let cmd = g_build_ticcmd(&mut st, &mut t, 1);
        assert_ne!(cmd.buttons & BT_ATTACK, 0);
        assert_ne!(cmd.buttons & BT_CHANGE, 0);
        // The weapon key is one-shot, like vanilla.
        let cmd = g_build_ticcmd(&mut st, &mut t, 1);
        assert_eq!(cmd.buttons & BT_CHANGE, 0);
    }

    #[test]
    fn events_fold_into_the_snapshot() {
        let mut st = InputState::default();
        st.apply_event(&Event {
            ty: EvType::KeyDown,
            data1: 2,
            data2: 0,
            data3: 0,
        });
        assert!(st.keys & (1 << 2) != 0);
        st.apply_event(&Event {
            ty: EvType::KeyUp,
            data1: 2,
            data2: 0,
            data3: 0,
        });
        assert_eq!(st.keys, 0);

        st.apply_event(&Event {
            ty: EvType::Mouse,
            data1: 1,
            data2: 7,
            data3: -3,
        });
        assert_eq!(st.mouse.dx, 7);
        assert_eq!(st.mouse.buttons, 1);
    }
}
