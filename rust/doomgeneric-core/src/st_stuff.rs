//! Status bar, port of `st_stuff.c`.
//!
//! The ticker and the face animation are pure state; the drawer blits the
//! vanilla patches (`STBAR`, `STTNUM`, `STF*`...) into an indexed 320x32
//! screen through [`crate::v_video`], which the host composes below the 3D
//! view. Everything here reads the WAD through the data-only `WadManager`.

use crate::d_player::Player;
use crate::doomdef::{PowerType, MAX_PLAYERS, NUM_WEAPONS};
use crate::r_main::point_to_angle_from;
use crate::tables::{ANG180, ANG45};
use crate::v_video::{draw_patch, Patch, SCREENWIDTH, ST_HEIGHT};
use crate::wad::{LumpName, WadManager};

/// Bar layout (`st_stuff.c` defines).
const ST_AMMOX: i32 = 44;
const ST_AMMOY: i32 = 171 - 168;
const ST_HEALTHX: i32 = 90;
const ST_HEALTHY: i32 = 171 - 168;
const ST_ARMSX: i32 = 111;
const ST_ARMSY: i32 = 172 - 168;
const ST_ARMSXSPACE: i32 = 12;
const ST_ARMSYSPACE: i32 = 10;
const ST_FACESX: i32 = 143;
const ST_FACESY: i32 = 0;
const ST_FRAGSX: i32 = 138;
const ST_FRAGSY: i32 = 171 - 168;
const ST_ARMORX: i32 = 221;
const ST_ARMORY: i32 = 171 - 168;
const ST_KEY0X: i32 = 239;
const ST_KEY0Y: i32 = 171 - 168;
const ST_KEY1Y: i32 = 181 - 168;

/// Face animation constants.
const ST_NUMPAINFACES: i32 = 5;
const ST_NUMSTRAIGHTFACES: i32 = 3;
const ST_NUMTURNFACES: i32 = 2;
const ST_NUMSPECIALFACES: i32 = 3;
const ST_FACESTRIDE: i32 = ST_NUMSTRAIGHTFACES + ST_NUMTURNFACES + ST_NUMSPECIALFACES;
const ST_TURNOFFSET: i32 = ST_NUMSTRAIGHTFACES;
const ST_OUCHOFFSET: i32 = ST_TURNOFFSET + ST_NUMTURNFACES;
const ST_EVILGRINOFFSET: i32 = ST_OUCHOFFSET + 1;
const ST_RAMPAGEOFFSET: i32 = ST_EVILGRINOFFSET + 1;
const ST_GODFACE: i32 = ST_NUMPAINFACES * ST_FACESTRIDE;
const ST_DEADFACE: i32 = ST_GODFACE + 1;

const ST_EVILGRINCOUNT: i32 = 2 * crate::doomdef::TICRATE as i32;
const ST_STRAIGHTFACECOUNT: i32 = crate::doomdef::TICRATE as i32 / 2;
const ST_TURNCOUNT: i32 = crate::doomdef::TICRATE as i32;
const ST_RAMPAGEDELAY: i32 = 2 * crate::doomdef::TICRATE as i32;
const ST_MUCHPAIN: i32 = 20;

/// The per-tic widget values the drawer needs.
#[derive(Clone, Copy, Debug, Default)]
pub struct StatusState {
    /// Current weapon ammo (1994 = "n/a" for the fist/chainsaw).
    pub ammo: i32,
    pub health: i32,
    pub armor: i32,
    /// Weapon slots 2..7 owned.
    pub arms: [bool; 6],
    /// Key card boxes: card or skull color, `-1` = none.
    pub keyboxes: [i32; 3],
    /// `st_faceindex` into the face table.
    pub face_index: i32,
    /// Deathmatch frag tally.
    pub frags: i32,
    pub deathmatch: bool,
}

impl StatusState {
    /// The vanilla face patch name for `face_index` (`STF*`).
    pub fn face_patch_name(&self) -> LumpName {
        face_name(self.face_index)
    }
}

/// Face index -> patch name (the `faces[]` table from `ST_loadFace`).
fn face_name(index: i32) -> LumpName {
    let stride = ST_FACESTRIDE;
    let pain = index / stride;
    let face = index % stride;
    let name: [u8; 8] = if index == ST_GODFACE {
        *b"STFGOD0 "
    } else if index == ST_DEADFACE {
        *b"STFDEAD0"
    } else {
        match face {
            0..=2 => {
                let mut n = *b"STFST00 ";
                write_digit(&mut n, 5, pain as u8);
                write_digit(&mut n, 6, face as u8);
                n
            }
            3 => *b"STFTR000",
            4 => *b"STFTL000",
            5 => {
                let mut n = *b"STFOUCH0";
                write_digit(&mut n, 7, pain as u8);
                n
            }
            6 => {
                let mut n = *b"STFEVL0 ";
                write_digit(&mut n, 6, pain as u8);
                n
            }
            _ => {
                let mut n = *b"STFKILL0";
                write_digit(&mut n, 7, pain as u8);
                n
            }
        }
    };
    LumpName(name)
}

fn write_digit(name: &mut [u8; 8], pos: usize, d: u8) {
    name[pos] = b'0' + d;
}

/// The persistent per-player status state (`st_*` globals + `oldweaponsowned`).
#[derive(Clone, Debug)]
pub struct StatusBar {
    pnum: usize,
    /// The display player's widgets.
    pub state: StatusState,
    // face widget state
    old_health: i32,
    lastcalc: i32,
    oldcalc_health: i32,
    oldweaponsowned: [bool; NUM_WEAPONS],
    lastattackdown: i32,
    priority: i32,
    facecount: i32,
    /// `st_randomnumber` (drawn from the world RNG at tick time by callers
    /// that pass a `random` value).
    random: u8,
}

impl StatusBar {
    pub fn new(pnum: usize) -> Self {
        StatusBar {
            pnum,
            state: StatusState::default(),
            old_health: -1,
            lastcalc: 0,
            oldcalc_health: -1,
            oldweaponsowned: [false; NUM_WEAPONS],
            lastattackdown: -1,
            priority: 0,
            facecount: 0,
            random: 0,
        }
    }

    /// `ST_Ticker`: one status bar tic for the display player.
    pub fn ticker(
        &mut self,
        player: &Player,
        mobjs: &crate::p_mobj::Mobjs,
        deathmatch: bool,
        rng: u8,
    ) {
        self.random = rng;

        // ST_updateWidgets
        self.state.ammo = match crate::info::WEAPONINFO[player.readyweapon as usize].ammo {
            None => 1994, // means "n/a"
            Some(a) => player.ammo[a as usize],
        };
        self.state.health = player.health;
        self.state.armor = player.armorpoints;
        for i in 0..6usize {
            self.state.arms[i] = player.weaponowned[i + 2];
        }
        for i in 0..3usize {
            let card = i as u8;
            self.state.keyboxes[i] = if player.cards[i] { card as i32 } else { -1 };
            if player.cards[i + 3] {
                self.state.keyboxes[i] = card as i32 + 3;
            }
        }
        self.state.deathmatch = deathmatch;
        self.state.frags = (0..MAX_PLAYERS)
            .filter(|&i| i != self.pnum)
            .map(|i| player.frags[i])
            .sum::<i32>()
            - player.frags[self.pnum];

        self.update_face(player, mobjs);
    }

    /// `ST_calcPainOffset`.
    fn calc_pain_offset(&mut self, player: &Player) -> i32 {
        let health = player.health.min(100);
        if health != self.oldcalc_health {
            self.lastcalc =
                ST_FACESTRIDE * (((100 - health) * ST_NUMPAINFACES) / 101);
            self.oldcalc_health = health;
        }
        self.lastcalc
    }

    /// `ST_updateFaceWidget`: the vanilla priority ladder.
    fn update_face(&mut self, player: &Player, mobjs: &crate::p_mobj::Mobjs) {
        let mut priority = 0;
        let mut faceindex = 0;
        let mut facecount = self.facecount;
        let pain = self.calc_pain_offset(player);

        // (each block mirrors the C `if (priority < N)` chain)
        if player.health == 0 {
            priority = 9;
            faceindex = ST_DEADFACE;
            facecount = 1;
        }
        if priority < 9 && player.bonuscount != 0 {
            let mut doevilgrin = false;
            for i in 0..NUM_WEAPONS {
                if self.oldweaponsowned[i] != player.weaponowned[i] {
                    doevilgrin = true;
                    self.oldweaponsowned[i] = player.weaponowned[i];
                }
            }
            if doevilgrin {
                // evil grin if just picked up weapon
                priority = 8;
                facecount = ST_EVILGRINCOUNT;
                faceindex = pain + ST_EVILGRINOFFSET;
            }
        }
        if priority < 8 && player.damagecount != 0 && player.attacker.is_some() {
            // being attacked
            priority = 7;
            let Some(attacker) = player.attacker else {
                return;
            };
            let Some(mo) = player.mo else {
                return;
            };
            if player.health - self.old_health > ST_MUCHPAIN {
                facecount = ST_TURNCOUNT;
                faceindex = pain + ST_OUCHOFFSET;
            } else {
                let (mx, my, mangle) = match mobjs.get(mo) {
                    Some(m) => (m.x, m.y, m.angle),
                    None => return,
                };
                let Some(a) = mobjs.get(attacker) else { return };
                let badguyangle = point_to_angle_from(mx, my, a.x, a.y);
                let (diffang, turn_right) = if badguyangle > mangle {
                    (badguyangle.wrapping_sub(mangle), badguyangle.wrapping_sub(mangle) > ANG180)
                } else {
                    (mangle.wrapping_sub(badguyangle), mangle.wrapping_sub(badguyangle) <= ANG180)
                };

                facecount = ST_TURNCOUNT;
                faceindex = pain;

                if diffang < ANG45 {
                    // head-on
                    faceindex += ST_RAMPAGEOFFSET;
                } else if turn_right {
                    // turn face right
                    faceindex += ST_TURNOFFSET;
                } else {
                    // turn face left
                    faceindex += ST_TURNOFFSET + 1;
                }
            }
        }
        if priority < 7 && player.damagecount != 0 {
            // getting hurt because of your own damn stupidity
            if player.health - self.old_health > ST_MUCHPAIN {
                priority = 7;
                facecount = ST_TURNCOUNT;
                faceindex = pain + ST_OUCHOFFSET;
            } else {
                priority = 6;
                facecount = ST_TURNCOUNT;
                faceindex = pain + ST_RAMPAGEOFFSET;
            }
        }
        if priority < 6 {
            // rapid firing
            if player.attackdown {
                if self.lastattackdown == -1 {
                    self.lastattackdown = ST_RAMPAGEDELAY;
                } else {
                    self.lastattackdown -= 1;
                    if self.lastattackdown == 0 {
                        priority = 5;
                        faceindex = pain + ST_RAMPAGEOFFSET;
                        facecount = 1;
                        self.lastattackdown = 1;
                    }
                }
            } else {
                self.lastattackdown = -1;
            }
        }
        if priority < 5 {
            // invulnerability
            if player.cheats & crate::d_player::Player::CF_GODMODE != 0
                || player.powers[PowerType::Invulnerability as usize] != 0
            {
                faceindex = ST_GODFACE;
                facecount = 1;
            }
        }

        // look left or look right if the facecount has timed out
        if facecount <= 0 {
            faceindex = pain + (self.random as i32 % 3);
            facecount = ST_STRAIGHTFACECOUNT;
        }

        self.old_health = player.health;
        self.facecount = facecount - 1;
        self.priority = priority;
        self.state.face_index = faceindex;
        let _ = self.priority;
    }
}

/// `ST_Drawer`-style composite of the whole bar into `screen`
/// (a 320x32 indexed buffer). Patches come from the WAD; missing ones are
/// skipped, so stripped-down IWADs still render the numbers.
pub fn st_draw(
    manager: &WadManager,
    state: &StatusState,
    screen: &mut [u8],
) {
    let screen_height = ST_HEIGHT as i32;

    // Background.
    draw_named(manager, screen, screen_height, b"STBAR", 0, 0);

    if state.deathmatch {
        draw_num(manager, screen, state.frags, ST_FRAGSX, ST_FRAGSY, 2);
    } else {
        // Arms owned.
        for (i, &owned) in state.arms.iter().enumerate() {
            let owned_name = *b"STGNUM0 ";
            let missing_name = *b"STYSNUM0";
            let mut n = if owned { owned_name } else { missing_name };
            let digit = if owned { 6 } else { 7 };
            n[digit] = b'0' + i as u8;
            draw_lump(manager, screen, screen_height, &n,
                       ST_ARMSX + i as i32 % 3 * ST_ARMSXSPACE,
                       ST_ARMSY + i as i32 / 3 * ST_ARMSYSPACE);
        }

        // Key boxes.
        for (i, &kb) in state.keyboxes.iter().enumerate() {
            if kb < 0 {
                continue;
            }
            let mut n = *b"STKEYS0 ";
            n[6] = b'0' + kb as u8;
            let y = if i == 1 { ST_KEY1Y } else { ST_KEY0Y };
            draw_lump(manager, screen, screen_height, &n, ST_KEY0X, y);
        }
    }

    // Ammo, health (with percent), armor.
    draw_num(manager, screen, state.ammo, ST_AMMOX, ST_AMMOY, 3);
    draw_num(manager, screen, state.health, ST_HEALTHX, ST_HEALTHY, 3);
    draw_named(manager, screen, screen_height, b"STTPRCNT", ST_HEALTHX + 39, ST_HEALTHY);
    draw_num(manager, screen, state.armor, ST_ARMORX, ST_ARMORY, 3);
    draw_named(manager, screen, screen_height, b"STTPRCNT", ST_ARMORX + 39, ST_ARMORY);

    // The face.
    draw_lump(manager, screen, screen_height, state.face_patch_name().as_bytes(), ST_FACESX, ST_FACESY);
}

/// Right-aligned small red number (`STlib_updateNum` with `STTNUM`).
fn draw_num(
    manager: &WadManager,
    screen: &mut [u8],
    mut value: i32,
    x: i32,
    y: i32,
    max_digits: usize,
) {
    let digit_w = 14;
    let mut digits: [u8; 8] = [0; 8];
    let mut n = 0;
    let negative = value < 0;
    if negative {
        value = -value;
    }
    loop {
        digits[n] = (value % 10) as u8;
        n += 1;
        value /= 10;
        if value == 0 || n == 8 {
            break;
        }
    }
    if negative && n < max_digits {
        n += 1;
        digits[n - 1] = 10; // minus
    }
    let n = n.max(1).min(max_digits);
    for (i, &d) in digits.iter().enumerate().take(n) {
        let mut name = *b"STTNUM0 ";
        if d == 10 {
            name = *b"STTMINS ";
        } else {
            name[6] = b'0' + d;
        }
        // Right-aligned: draw from the last digit backwards.
        let dx = x + (n - 1 - i) as i32 * digit_w;
        draw_lump(manager, screen, ST_HEIGHT as i32, &name, dx, y);
    }
}

fn draw_named(
    manager: &WadManager,
    screen: &mut [u8],
    screen_height: i32,
    name: &[u8],
    x: i32,
    y: i32,
) {
    draw_lump(manager, screen, screen_height, name, x, y);
}

fn draw_lump(
    manager: &WadManager,
    screen: &mut [u8],
    screen_height: i32,
    name: &[u8],
    x: i32,
    y: i32,
) {
    let lname = LumpName::from_bytes(&name[..name.len().min(8)]);
    let Ok(data) = manager.lump_data_by_name(&lname) else {
        return;
    };
    let patch = Patch::from_lump(data);
    draw_patch(screen, SCREENWIDTH as i32, screen_height, &patch, x, y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doomdef::{AmmoType, Card, WeaponType};

    fn player_with(health: i32, ammo: i32) -> Player {
        let mut p = Player::new();
        p.health = health;
        p.ammo[AmmoType::Clip as usize] = ammo;
        p
    }

    #[test]
    fn widgets_track_player_state() {
        let mut st = StatusBar::new(0);
        let mut p = player_with(73, 42);
        p.weaponowned[WeaponType::Shotgun as usize] = true;
        p.cards[Card::BlueCard as usize] = true;
        let mobjs = crate::p_mobj::Mobjs::new(1);
        st.ticker(&p, &mobjs, false, 7);
        assert_eq!(st.state.health, 73);
        assert_eq!(st.state.ammo, 42); // pistol uses clips
        assert!(st.state.arms[WeaponType::Shotgun as usize - 2]);
        assert_eq!(st.state.keyboxes[0], 0);
        assert_eq!(st.state.frags, 0);
    }

    #[test]
    fn dead_face_wins_then_recovers() {
        let mut st = StatusBar::new(0);
        let mobjs = crate::p_mobj::Mobjs::new(1);
        st.ticker(&player_with(0, 0), &mobjs, false, 0);
        assert_eq!(st.state.face_index, ST_DEADFACE);

        // Back alive: straight faces keyed off the (fresh) pain offset.
        st.ticker(&player_with(100, 0), &mobjs, false, 0);
        assert_eq!(st.state.face_index, 0); // pain 0, straight 0
    }

    #[test]
    fn god_face_overrides() {
        let mut st = StatusBar::new(0);
        let mut p = player_with(50, 0);
        p.cheats = crate::d_player::Player::CF_GODMODE;
        let mobjs = crate::p_mobj::Mobjs::new(1);
        st.ticker(&p, &mobjs, false, 0);
        assert_eq!(st.state.face_index, ST_GODFACE);
    }

    #[test]
    fn no_ammo_weapon_shows_na() {
        let mut st = StatusBar::new(0);
        let mut p = player_with(100, 50);
        p.readyweapon = WeaponType::Chainsaw;
        let mobjs = crate::p_mobj::Mobjs::new(1);
        st.ticker(&p, &mobjs, false, 0);
        assert_eq!(st.state.ammo, 1994);
    }

    #[test]
    fn face_patch_names_match_vanilla() {
        assert_eq!(face_name(0).as_bytes(), b"STFST00 ");
        assert_eq!(face_name(ST_TURNOFFSET).as_bytes(), b"STFTR000");
        assert_eq!(face_name(ST_TURNOFFSET + 1).as_bytes(), b"STFTL000");
        assert_eq!(face_name(ST_OUCHOFFSET + ST_FACESTRIDE * 2).as_bytes(), b"STFOUCH2");
        assert_eq!(face_name(ST_GODFACE).as_bytes(), b"STFGOD0 ");
        assert_eq!(face_name(ST_DEADFACE).as_bytes(), b"STFDEAD0");
        assert_eq!(face_name(ST_EVILGRINOFFSET + ST_FACESTRIDE * 4).as_bytes(), b"STFEVL4 ");
    }

}
