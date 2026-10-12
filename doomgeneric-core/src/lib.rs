#![no_std]
#![forbid(unsafe_code)]
// `usize::is_multiple_of` is newer than our MSRV, so `x % n == 0` is used on
// purpose and this (newer) clippy lint is expected.
#![allow(clippy::manual_is_multiple_of)]

extern crate alloc;

pub mod d_main;
pub mod d_net;
pub mod doomdef;
pub mod d_items;
pub mod d_loop;
pub mod d_player;
pub mod d_ticcmd;
pub mod f_finale;
pub mod fixed;
pub mod g_game;
pub mod g_save;
pub mod hu_stuff;
pub mod info;
pub mod level;
pub mod m_config;
pub mod m_demo;
pub mod m_menu;
pub mod m_misc;
pub mod p_ceilng;
pub mod p_doors;
pub mod p_enemy;
pub mod p_floor;
pub mod p_inter;
pub mod p_lights;
pub mod p_map;
pub mod p_mobj;
pub mod p_maputl;
pub mod p_plats;
pub mod p_pspr;
pub mod p_sight;
pub mod p_spec;
pub mod p_switch;
pub mod p_telept;
pub mod p_tick;
pub mod p_user;
pub mod p_world;
pub mod platform;
pub mod r_bsp;
pub mod r_data;
pub mod r_draw;
pub mod r_frame;
pub mod r_main;
pub mod r_plane;
pub mod r_setup;
pub mod r_sky;
pub mod r_things;
pub mod random;
pub mod s_sound;
pub mod sounds;
pub mod st_stuff;
pub mod tables;
pub mod v_video;
pub mod wad;
pub mod wi_stuff;

pub use doomdef::*;
pub use platform::{
    Audio, Clock, FileSystem, Frame, Input, KeyCode, KeyEvent, Network, PlatformIo, Renderer,
};
pub use random::{Random, RandomStream};
pub use wad::{LumpName, Wad, WadError, WadManager};

pub const DOOM_RESX: usize = 640;
pub const DOOM_RESY: usize = 400;

pub type Pixel = u32;
