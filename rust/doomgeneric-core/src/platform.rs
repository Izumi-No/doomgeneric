//! Decoupled host/platform boundaries (`IO` in general).
//!
//! The engine core never performs IO directly. Instead it is driven by
//! objects implementing the traits below. Each concern (rendering, input,
//! audio, timing, filesystem, startup args) is isolated so a platform can
//! stub out, replace or drop whichever parts it does not need.
//!
//! Everything here is `no_std` and free of any libc dependency.

use alloc::string::String;
use alloc::vec::Vec;

/// A single key identifier, mapped to the engine's portable key codes.
pub type KeyCode = u8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub pressed: bool,
    pub key: KeyCode,
}

/// A fully rendered frame, ready to be blitted by the host.
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub width: usize,
    pub height: usize,
    pub pixels: &'a [Pixel],
}

/// Produces the final image. The host is responsible only for presenting
/// `Frame` to the physical screen.
pub trait Renderer {
    type Error;

    fn draw_frame(&mut self, frame: Frame<'_>) -> Result<(), Self::Error>;
    fn set_window_title(&mut self, _title: &str) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// A relative mouse update (`mousex`/`mousey` accumulators in vanilla).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseDelta {
    pub dx: i32,
    pub dy: i32,
    /// Mouse buttons as bits (`mousebuttons[0..3]`).
    pub buttons: u8,
}

/// A joystick snapshot (`joyxmove`/`joyymove`/`joybuttons` in vanilla).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoystickState {
    pub x: i32,
    pub y: i32,
    pub buttons: u8,
    /// `joystrafemove` (the strafe axis some pads expose).
    pub strafe: i32,
}

/// Supplies keyboard events to the engine. The mouse/joystick polls are
/// optional: a host that has no pointer hardware simply never produces
/// them and `G_BuildTiccmd` sees zeros.
pub trait Input {
    fn poll_key(&mut self) -> Option<KeyEvent>;
    fn poll_mouse(&mut self) -> Option<MouseDelta> {
        None
    }
    fn poll_joystick(&mut self) -> Option<JoystickState> {
        None
    }
}

/// Receives PCM samples for the host to mix into its audio output.
pub trait Audio {
    fn submit_samples(&mut self, _samples: &[i16]) {}
}

/// Wall-clock used by the engine for frame timing and delays.
pub trait Clock {
    fn sleep_ms(&mut self, ms: u32);
    fn ticks_ms(&self) -> u32;
}

/// Byte-level filesystem access (e.g. WAD loading). The engine asks for raw
/// bytes; the host decides where they come from (disk, ROM, network...).
pub trait FileSystem {
    type Error;

    fn read(&mut self, path: &str) -> Result<Vec<u8>, Self::Error>;
    fn exists(&mut self, _path: &str) -> bool {
        false
    }
}

/// Startup arguments (equivalent to `argc`/`argv`).
pub trait Args {
    fn args(&self) -> Vec<String>;
}

use crate::Pixel;

pub trait PlatformIo: Renderer + Input + Audio + Clock {}

impl<T> PlatformIo for T where T: Renderer + Input + Audio + Clock {}
