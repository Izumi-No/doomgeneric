#![no_std]

pub const DOOM_RESX: usize = 640;
pub const DOOM_RESY: usize = 400;

pub type Pixel = u32;
pub type KeyCode = u8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub pressed: bool,
    pub key: KeyCode,
}

#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub width: usize,
    pub height: usize,
    pub pixels: &'a [Pixel],
}

pub trait Renderer {
    type Error;

    fn draw_frame(&mut self, frame: Frame<'_>) -> Result<(), Self::Error>;
    fn set_window_title(&mut self, _title: &str) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub trait Input {
    fn poll_key(&mut self) -> Option<KeyEvent>;
}

pub trait Audio {
    fn submit_samples(&mut self, _samples: &[i16]) {}
}

pub trait Clock {
    fn sleep_ms(&mut self, ms: u32);
    fn ticks_ms(&self) -> u32;
}

pub trait PlatformIo: Renderer + Input + Audio + Clock {}

impl<T> PlatformIo for T where T: Renderer + Input + Audio + Clock {}
