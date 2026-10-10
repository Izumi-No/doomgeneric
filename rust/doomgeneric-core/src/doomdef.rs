//! Core enums and constants, port of `doomdef.h`, `doomdata.h`, `i_timer.h`.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameState {
    Level,
    Intermission,
    Finale,
    DemoScreen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameAction {
    Nothing,
    LoadLevel,
    NewGame,
    LoadGame,
    SaveGame,
    PlayDemo,
    Completed,
    Victory,
    WorldDone,
    Screenshot,
}

pub const MAX_PLAYERS: usize = 4;

pub const MTF_EASY: u16 = 1;
pub const MTF_NORMAL: u16 = 2;
pub const MTF_HARD: u16 = 4;
pub const MTF_AMBUSH: u16 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Card {
    BlueCard,
    YellowCard,
    RedCard,
    BlueSkull,
    YellowSkull,
    RedSkull,
}

pub const NUM_CARDS: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WeaponType {
    Fist,
    Pistol,
    Shotgun,
    Chaingun,
    Missile,
    Plasma,
    Bfg,
    Chainsaw,
    SuperShotgun,
}

pub const NUM_WEAPONS: usize = 9;

/// Identity table `index -> WeaponType` (savegames store raw discriminants).
pub static WEAPONTYPE_BY_INDEX: [WeaponType; NUM_WEAPONS] = [
    WeaponType::Fist,
    WeaponType::Pistol,
    WeaponType::Shotgun,
    WeaponType::Chaingun,
    WeaponType::Missile,
    WeaponType::Plasma,
    WeaponType::Bfg,
    WeaponType::Chainsaw,
    WeaponType::SuperShotgun,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmmoType {
    Clip,
    Shell,
    Cell,
    Missile,
}

pub const NUM_AMMO: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerType {
    Invulnerability,
    Strength,
    Invisibility,
    IronFeet,
    AllMap,
    InfraRed,
}

pub const NUM_POWERS: usize = 6;

pub const TICRATE: u32 = 35;

pub const INVULN_TICS: i32 = 30 * TICRATE as i32;
pub const INVIS_TICS: i32 = 60 * TICRATE as i32;
pub const INFRARED_TICS: i32 = 120 * TICRATE as i32;
pub const IRON_TICS: i32 = 60 * TICRATE as i32;

// Map lump order in a level.
pub const ML_LABEL: usize = 0;
pub const ML_THINGS: usize = 1;
pub const ML_LINEDEFS: usize = 2;
pub const ML_SIDEDEFS: usize = 3;
pub const ML_VERTEXES: usize = 4;
pub const ML_SEGS: usize = 5;
pub const ML_SSECTORS: usize = 6;
pub const ML_NODES: usize = 7;
pub const ML_SECTORS: usize = 8;
pub const ML_REJECT: usize = 9;
pub const ML_BLOCKMAP: usize = 10;

// Linedef flags.
pub const ML_BLOCKING: u16 = 1;
pub const ML_BLOCKMONSTERS: u16 = 2;
pub const ML_TWOSIDED: u16 = 4;
pub const ML_DONTPEGTOP: u16 = 8;
pub const ML_DONTPEGBOTTOM: u16 = 16;
pub const ML_SECRET: u16 = 32;
pub const ML_SOUNDBLOCK: u16 = 64;
pub const ML_DONTDRAW: u16 = 128;
pub const ML_MAPPED: u16 = 256;

pub const NF_SUBSECTOR: u16 = 0x8000;

impl WeaponType {
    /// Table index -> enum (see `WEAPONTYPE_BY_INDEX`).
    pub fn from_index(i: i32) -> WeaponType {
        WEAPONTYPE_BY_INDEX[(i as usize).min(NUM_WEAPONS - 1)]
    }
}
