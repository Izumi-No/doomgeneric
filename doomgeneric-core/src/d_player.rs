//! Player state, port of `d_player.h`.
//!
//! Vanilla stores `mobj_t*`/`pspdef_t` pointers and a `ticcmd_t` inline;
//! here `mo`/`attacker` are [`MobjId`] handles and psprites live in a small
//! fixed array. Messages are kept as a fate string marker (the message
//! contents live in `dstrings`).

use crate::d_ticcmd::TicCmd;
use crate::doomdef::{NUM_AMMO, NUM_CARDS, NUM_POWERS, NUM_WEAPONS};
use crate::fixed::Fixed;
use crate::info::StateId;
use crate::p_mobj::MobjId;
use crate::doomdef::{AmmoType, Card, PowerType, WeaponType};

pub const NUM_PSPRITES: usize = 2;

/// `psprnum_t`: weapon overlay, then muzzle flash.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PspriteNum {
    Weapon,
    Flash,
}

impl PspriteNum {
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// `pspdef_t`. `state` is a [`StateId`]; `None` marks an inactive slot
/// (vanilla's NULL `state_t*`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Psprite {
    pub state: Option<StateId>,
    pub tics: i32,
    pub sx: Fixed,
    pub sy: Fixed,
}

/// `playerstate_t`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerState {
    Live,
    Dead,
    Reborn,
}

/// `player_t`.
#[derive(Clone, Debug)]
pub struct Player {
    pub mo: Option<MobjId>,
    pub playerstate: PlayerState,
    pub cmd: TicCmd,

    pub viewz: Fixed,
    pub viewheight: Fixed,
    pub deltaviewheight: Fixed,
    pub bob: Fixed,

    pub health: i32,
    pub armorpoints: i32,
    pub armortype: i32,

    pub powers: [i32; NUM_POWERS],
    pub cards: [bool; NUM_CARDS],
    pub backpack: bool,

    pub frags: [i32; 4],
    pub readyweapon: WeaponType,
    /// `pendingweapon == wp_nochange` is `None`.
    pub pendingweapon: Option<WeaponType>,

    pub weaponowned: [bool; NUM_WEAPONS],
    pub ammo: [i32; NUM_AMMO],
    pub maxammo: [i32; NUM_AMMO],

    pub attackdown: bool,
    pub usedown: bool,
    pub cheats: u32,
    pub refire: i32,

    pub killcount: i32,
    pub itemcount: i32,
    pub secretcount: i32,

    pub damagecount: i32,
    pub bonuscount: i32,

    pub attacker: Option<MobjId>,
    pub extralight: i32,
    pub fixedcolormap: i32,
    pub colormap: i32,

    pub psprites: [Psprite; NUM_PSPRITES],

    pub didsecret: bool,

    /// Pending HUD message (`player->message`); the strings live with the
    /// caller (`dstrings` is a host concern).
    pub message: Option<&'static str>,
}

impl Player {
    #[inline]
    pub fn has_ammo(&self, ammo: Option<AmmoType>, amount: i32) -> bool {
        match ammo {
            None => true,
            Some(a) => self.ammo[a as usize] >= amount,
        }
    }

    #[inline]
    pub fn has_power(&self, power: PowerType) -> bool {
        self.powers[power as usize] != 0
    }

    #[inline]
    pub fn has_card(&self, card: Card) -> bool {
        self.cards[card as usize]
    }

    pub fn new() -> Self {
        let mut p = Player {
            mo: None,
            playerstate: PlayerState::Live,
            cmd: TicCmd::default(),
            viewz: 0,
            viewheight: 41 << 16,
            deltaviewheight: 0,
            bob: 0,
            health: 100,
            armorpoints: 0,
            armortype: 0,
            powers: [0; NUM_POWERS],
            cards: [false; NUM_CARDS],
            backpack: false,
            frags: [0; 4],
            readyweapon: WeaponType::Pistol,
            pendingweapon: None,
            weaponowned: [false; NUM_WEAPONS],
            ammo: [0; NUM_AMMO],
            maxammo: [200, 50, 300, 50],
            attackdown: false,
            usedown: false,
            cheats: 0,
            refire: 0,
            killcount: 0,
            itemcount: 0,
            secretcount: 0,
            damagecount: 0,
            bonuscount: 0,
            attacker: None,
            extralight: 0,
            fixedcolormap: 0,
            colormap: 0,
            psprites: [Psprite::default(); NUM_PSPRITES],
            didsecret: false,
            message: None,
        };
        p.weaponowned[WeaponType::Fist as usize] = true;
        p.weaponowned[WeaponType::Pistol as usize] = true;
        p.ammo[AmmoType::Clip as usize] = 50;
        p
    }

    /// `CF_NOCLIP`/`CF_GODMODE`/`CF_NOMOMENTUM` bits (`cheat_t`).
    pub const CF_NOCLIP: u32 = 1;
    pub const CF_GODMODE: u32 = 2;
    pub const CF_NOMOMENTUM: u32 = 4;
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_player_starts_with_pistol() {
        let p = Player::new();
        assert_eq!(p.readyweapon, WeaponType::Pistol);
        assert!(p.weaponowned[WeaponType::Fist as usize]);
        assert_eq!(p.maxammo[AmmoType::Clip as usize], 200);
        assert_eq!(p.viewheight, 41 << 16);
    }

    #[test]
    fn psprite_inactive_state_is_none() {
        let p: Player = Default::default();
        assert!(p.psprites[PspriteNum::Weapon.index()].state.is_none());
    }
}
