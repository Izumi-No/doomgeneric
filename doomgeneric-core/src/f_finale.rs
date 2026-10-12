//! The finale: text screens and the DOOM II cast, port of `f_finale.c`.
//!
//! The stages (`F_STAGE_TEXT`/`F_STAGE_ARTSCREEN`/`F_STAGE_CAST`), the
//! text typing speed and the cast animation are pure state; drawing uses
//! the shared `v_video` primitives (flats, `STCFN` font patches, sprite
//! patches), all fed from the data-only `WadManager`.

use alloc::format;
use alloc::vec::Vec;

use crate::info::{MobjType, StateId, MOBJINFO, STATES};
use crate::sounds::Sfx;
use crate::v_video::{self, Patch};
use crate::wad::{LumpName, WadManager};

/// `TEXTSPEED`: chars per 3 tics.
pub const TEXTSPEED: i32 = 3;
/// `TEXTWAIT`: tics to wait after the text.
pub const TEXTWAIT: i32 = 250;
/// `SLOWTURNTICS`-analog for the cast: base cast tic delay.
const CAST_TICS: i32 = crate::doomdef::TICRATE as i32 / 5;

/// `E1TEXT` (d_englsh.h).
pub const E1_TEXT: &str = "Once you beat the big badasses and\nclean out the moon base you're supposed\nto win, aren't you? Aren't you? Where's\nyour fat reward and ticket home? What\nthe hell is this? It's not supposed to\nend this way!\n\nIt stinks like rotten meat, but looks\nlike the lost Deimos base.  Looks like\nyou're stuck on The Shores of Hell.\nThe only way out is through.\n\nTo continue the DOOM experience, play\nThe Shores of Hell and its amazing\nsequel, Inferno!\n";

/// `E2TEXT` (d_englsh.h).
pub const E2_TEXT: &str = "You've done it! The hideous cyber-\ndemon lord that ruled the lost Deimos\nmoon base has been slain and you\nare triumphant! But ... where are\nyou? You clamber to the edge of the\nmoon and look down to see the awful\ntruth.\n\nDeimos floats above Hell itself!\nYou've never heard of anyone escaping\nfrom Hell, but you'll make the bastards\nsorry they ever heard of you! Quickly,\nyou rappel down to  the surface of\nHell.\n\nNow, it's on to the final chapter of\nDOOM! -- Inferno.";

/// `E3TEXT` (d_englsh.h).
pub const E3_TEXT: &str = "The loathsome spiderdemon that\nmasterminded the invasion of the moon\nbases and caused so much death has had\nits ass kicked for all time.\n\nA hidden doorway opens and you enter.\nYou've proven too tough for Hell to\ncontain, and now Hell at last plays\nfair -- for you emerge from the door\nto see the green fields of Earth!\nHome at last.\n\nYou wonder what's been happening on\nEarth while you were battling evil\nunleashed. It's good that no Hell-\nspawn could have come through that\ndoor with you ...";

/// `E4TEXT` (d_englsh.h).
pub const E4_TEXT: &str = "the spider mastermind must have sent forth\nits legions of hellspawn before your\nfinal confrontation with that terrible\nbeast from hell.  but you stepped forward\nand brought forth eternal damnation and\nsuffering upon the horde as a true hero\nwould in the face of something so evil.\n\nbesides, someone was gonna pay for what\nhappened to daisy, your pet rabbit.\n\nbut now, you see spread before you more\npotential pain and gibbitude as a nation\nof demons run amok among our cities.\n\nnext stop, hell on earth!";

/// `C1TEXT` (d_englsh.h).
pub const C1_TEXT: &str = "YOU HAVE ENTERED DEEPLY INTO THE INFESTED\nSTARPORT. BUT SOMETHING IS WRONG. THE\nMONSTERS HAVE BROUGHT THEIR OWN REALITY\nWITH THEM, AND THE STARPORT'S TECHNOLOGY\nIS BEING SUBVERTED BY THEIR PRESENCE.\n\nAHEAD, YOU SEE AN OUTPOST OF HELL, A\nFORTIFIED ZONE. IF YOU CAN GET PAST IT,\nYOU CAN PENETRATE INTO THE HAUNTED HEART\nOF THE STARBASE AND FIND THE CONTROLLING\nSWITCH WHICH HOLDS EARTH'S POPULATION\nHOSTAGE.";

/// `C2TEXT` (d_englsh.h).
pub const C2_TEXT: &str = "YOU HAVE WON! YOUR VICTORY HAS ENABLED\nHUMANKIND TO EVACUATE EARTH AND ESCAPE\nTHE NIGHTMARE.  NOW YOU ARE THE ONLY\nHUMAN LEFT ON THE FACE OF THE PLANET.\nCANNIBAL MUTATIONS, CARNIVOROUS ALIENS,\nAND EVIL SPIRITS ARE YOUR ONLY NEIGHBORS.\nYOU SIT BACK AND WAIT FOR DEATH, CONTENT\nTHAT YOU HAVE SAVED YOUR SPECIES.\n\nBUT THEN, EARTH CONTROL BEAMS DOWN A\nMESSAGE FROM SPACE: \"SENSORS HAVE LOCATED\nTHE SOURCE OF THE ALIEN INVASION. IF YOU\nGO THERE, YOU MAY BE ABLE TO BLOCK THEIR\nENTRY.  THE ALIEN BASE IS IN THE HEART OF\nYOUR OWN HOME CITY, NOT FAR FROM THE\nSTARPORT.\" SLOWLY AND PAINFULLY YOU GET\nUP AND RETURN TO THE FRAY.";

/// `C3TEXT` (d_englsh.h).
pub const C3_TEXT: &str = "YOU ARE AT THE CORRUPT HEART OF THE CITY,\nSURROUNDED BY THE CORPSES OF YOUR ENEMIES.\nYOU SEE NO WAY TO DESTROY THE CREATURES'\nENTRYWAY ON THIS SIDE, SO YOU CLENCH YOUR\nTEETH AND PLUNGE THROUGH IT.\n\nTHERE MUST BE A WAY TO CLOSE IT ON THE\nOTHER SIDE. WHAT DO YOU CARE IF YOU'VE\nGOT TO GO THROUGH HELL TO GET TO IT?";

/// `C4TEXT` (d_englsh.h).
pub const C4_TEXT: &str = "THE HORRENDOUS VISAGE OF THE BIGGEST\nDEMON YOU'VE EVER SEEN CRUMBLES BEFORE\nYOU, AFTER YOU PUMP YOUR ROCKETS INTO\nHIS EXPOSED BRAIN. THE MONSTER SHRIVELS\nUP AND DIES, ITS THRASHING LIMBS\nDEVASTATING UNTOLD MILES OF HELL'S\nSURFACE.\n\nYOU'VE DONE IT. THE INVASION IS OVER.\nEARTH IS SAVED. HELL IS A WRECK. YOU\nWONDER WHERE BAD FOLKS WILL GO WHEN THEY\nDIE, NOW. WIPING THE SWEAT FROM YOUR\nFOREHEAD YOU BEGIN THE LONG TREK BACK\nHOME. REBUILDING EARTH OUGHT TO BE A\nLOT MORE FUN THAN RUINING IT WAS.\n";

/// `C5TEXT` (d_englsh.h).
pub const C5_TEXT: &str = "CONGRATULATIONS, YOU'VE FOUND THE SECRET\nLEVEL! LOOKS LIKE IT'S BEEN BUILT BY\nHUMANS, RATHER THAN DEMONS. YOU WONDER\nWHO THE INMATES OF THIS CORNER OF HELL\nWILL BE.";

/// `C6TEXT` (d_englsh.h).
pub const C6_TEXT: &str = "CONGRATULATIONS, YOU'VE FOUND THE\nSUPER SECRET LEVEL!  YOU'D BETTER\nBLAZE THROUGH THIS ONE!\n";


/// `textscreen_t` (the `textscreens[]` table).
struct TextScreen {
    commercial: bool,
    episode: i32,
    map: i32,
    flat: &'static str,
    text: &'static str,
}

static TEXTSCREENS: [TextScreen; 10] = [
    TextScreen { commercial: false, episode: 1, map: 8, flat: "FLOOR4_8", text: E1_TEXT },
    TextScreen { commercial: false, episode: 2, map: 8, flat: "SFLR6_1", text: E2_TEXT },
    TextScreen { commercial: false, episode: 3, map: 8, flat: "MFLR8_4", text: E3_TEXT },
    TextScreen { commercial: false, episode: 4, map: 8, flat: "MFLR8_3", text: E4_TEXT },
    TextScreen { commercial: true, episode: 1, map: 6, flat: "SLIME16", text: C1_TEXT },
    TextScreen { commercial: true, episode: 1, map: 11, flat: "RROCK14", text: C2_TEXT },
    TextScreen { commercial: true, episode: 1, map: 20, flat: "RROCK07", text: C3_TEXT },
    TextScreen { commercial: true, episode: 1, map: 30, flat: "RROCK17", text: C4_TEXT },
    TextScreen { commercial: true, episode: 1, map: 15, flat: "RROCK13", text: C5_TEXT },
    TextScreen { commercial: true, episode: 1, map: 31, flat: "RROCK19", text: C6_TEXT },
];

/// The cast members (`castorder[]`, names from `d_englsh.h`).
pub static CAST_ORDER: [(MobjType, &str); 17] = [
    (MobjType::MT_POSSESSED, "Zombieman"),
    (MobjType::MT_SHOTGUY, "Shotgun guy"),
    (MobjType::MT_CHAINGUY, "Heavy Weapon Dude"),
    (MobjType::MT_TROOP, "Imp"),
    (MobjType::MT_SERGEANT, "Demon"),
    (MobjType::MT_SKULL, "Lost Soul"),
    (MobjType::MT_HEAD, "Cacodemon"),
    (MobjType::MT_KNIGHT, "Hell Knight"),
    (MobjType::MT_BRUISER, "Baron of Hell"),
    (MobjType::MT_BABY, "Arachnotron"),
    (MobjType::MT_PAIN, "Pain Elemental"),
    (MobjType::MT_UNDEAD, "Revenant"),
    (MobjType::MT_FATSO, "Mancubus"),
    (MobjType::MT_VILE, "Arch-Vile"),
    (MobjType::MT_SPIDER, "The Spider Mastermind"),
    (MobjType::MT_CYBORG, "The Cyberdemon"),
    (MobjType::MT_PLAYER, "Our Hero"),
];

/// `finalestage_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinaleStage {
    Text,
    ArtScreen,
    Cast,
}

/// The `f_finale.c` globals, owned.
pub struct Finale {
    pub stage: FinaleStage,
    /// `finalecount`.
    pub count: i32,
    /// The active text (one of the `*_TEXT` constants).
    pub text: &'static str,
    /// The background flat of the text screen.
    pub flat: &'static str,
    pub commercial: bool,
    pub episode: i32,
    pub map: i32,
    // cast state
    pub cast_num: usize,
    pub cast_tics: i32,
    pub cast_state: StateId,
    pub cast_death: bool,
    pub cast_on_melee: bool,
    pub cast_frames: i32,
    pub cast_attacking: bool,
    sounds: Vec<Sfx>,
}

impl Finale {
    /// `F_StartFinale`: pick the screen for the current position.
    pub fn start(commercial: bool, episode: i32, map: i32) -> Self {
        let mut fin = Finale {
            stage: FinaleStage::Text,
            count: 0,
            text: E1_TEXT,
            flat: "FLOOR4_8",
            commercial,
            episode,
            map,
            cast_num: 0,
            cast_tics: 0,
            cast_state: StateId::S_NULL,
            cast_death: false,
            cast_on_melee: false,
            cast_frames: 0,
            cast_attacking: false,
            sounds: Vec::new(),
        };
        for screen in TEXTSCREENS.iter() {
            // Vanilla: the episode is only checked for DOOM (non-commercial).
            if screen.commercial == commercial
                && (commercial || episode == screen.episode)
                && map == screen.map
            {
                fin.text = screen.text;
                fin.flat = screen.flat;
            }
        }
        fin
    }

    /// `F_StartCast` (the DOOM II map-30 secret ending).
    pub fn start_cast(&mut self) {
        self.stage = FinaleStage::Cast;
        self.cast_num = 0;
        self.cast_state = MOBJINFO[CAST_ORDER[0].0 as usize].seestate;
        self.cast_tics = CAST_TICS;
        self.cast_death = false;
        self.cast_on_melee = false;
        self.cast_frames = 0;
        self.cast_attacking = false;
    }

    /// `F_Ticker`: advance the stage machine. `any_button` is "some player
    /// pressed something" (the commercial skip). Returns `true` when the
    /// finale wants the world to advance (`ga_worlddone`).
    pub fn ticker(&mut self, any_button: bool) -> bool {
        // check for skipping
        if self.commercial && self.count > 50 && any_button {
            if self.map == 30 {
                self.start_cast();
            } else {
                return true; // ga_worlddone
            }
        }

        // advance animation
        self.count += 1;

        if self.stage == FinaleStage::Cast {
            self.cast_ticker();
            return false;
        }

        if self.commercial {
            return false;
        }

        if self.stage == FinaleStage::Text
            && self.count > self.text.chars().count() as i32 * TEXTSPEED + TEXTWAIT
        {
            self.count = 0;
            self.stage = FinaleStage::ArtScreen;
        }
        false
    }

    /// `F_CastTicker`.
    fn cast_ticker(&mut self) {
        self.cast_tics -= 1;
        if self.cast_tics > 0 {
            return; // not time to change state yet
        }

        let st = STATES[self.cast_state as usize];
        if st.tics == -1 || st.nextstate == StateId::S_NULL {
            // switch from deathstate to next monster
            self.cast_num = (self.cast_num + 1) % CAST_ORDER.len();
            self.cast_death = false;
            let ty = CAST_ORDER[self.cast_num].0;
            let seesound = MOBJINFO[ty as usize].seesound;
            if seesound != Sfx::sfx_None {
                self.sounds.push(seesound);
            }
            self.cast_state = MOBJINFO[ty as usize].seestate;
            self.cast_frames = 0;
        } else {
            // just advance to next state in animation
            let next = st.nextstate;

            // sound hacks....
            let sfx = match next {
                StateId::S_PLAY_ATK1 => Some(Sfx::sfx_dshtgn),
                StateId::S_POSS_ATK2 => Some(Sfx::sfx_pistol),
                StateId::S_SPOS_ATK2 => Some(Sfx::sfx_shotgn),
                StateId::S_VILE_ATK2 => Some(Sfx::sfx_vilatk),
                StateId::S_SKEL_FIST2 => Some(Sfx::sfx_skeswg),
                StateId::S_SKEL_FIST4 => Some(Sfx::sfx_skepch),
                StateId::S_SKEL_MISS2 => Some(Sfx::sfx_skeatk),
                StateId::S_FATT_ATK2 | StateId::S_FATT_ATK5 | StateId::S_FATT_ATK8 => {
                    Some(Sfx::sfx_firsht)
                }
                StateId::S_CPOS_ATK2 | StateId::S_CPOS_ATK3 | StateId::S_CPOS_ATK4 => {
                    Some(Sfx::sfx_shotgn)
                }
                StateId::S_TROO_ATK3 => Some(Sfx::sfx_claw),
                StateId::S_SARG_ATK2 => Some(Sfx::sfx_sgtatk),
                StateId::S_BOSS_ATK2 | StateId::S_BOS2_ATK2 | StateId::S_HEAD_ATK2 => {
                    Some(Sfx::sfx_firsht)
                }
                StateId::S_SKULL_ATK2 => Some(Sfx::sfx_sklatk),
                StateId::S_SPID_ATK2 | StateId::S_SPID_ATK3 => Some(Sfx::sfx_shotgn),
                StateId::S_BSPI_ATK2 => Some(Sfx::sfx_plasma),
                StateId::S_CYBER_ATK2 | StateId::S_CYBER_ATK4 | StateId::S_CYBER_ATK6 => {
                    Some(Sfx::sfx_rlaunc)
                }
                StateId::S_PAIN_ATK3 => Some(Sfx::sfx_sklatk),
                _ => None,
            };
            if let Some(s) = sfx {
                self.sounds.push(s);
            }

            self.cast_state = next;
            self.cast_frames += 1;
        }

        let ty = CAST_ORDER[self.cast_num].0;
        if self.cast_frames == 12 {
            // go into attack frame
            self.cast_attacking = true;
            let st_id = if self.cast_on_melee {
                MOBJINFO[ty as usize].meleestate
            } else {
                MOBJINFO[ty as usize].missilestate
            };
            self.cast_on_melee = !self.cast_on_melee;
            if st_id != StateId::S_NULL {
                self.cast_state = st_id;
            }
        }

        if self.cast_attacking {
            let st = STATES[self.cast_state as usize];
            if st.tics == -1 || st.nextstate == StateId::S_NULL {
                // Oh, gross hack! (stop the attack)
                self.cast_state = MOBJINFO[ty as usize].seestate;
                self.cast_attacking = false;
                self.cast_on_melee = false;
                self.cast_frames = 0;
                self.cast_tics = CAST_TICS;
                return;
            }
        }

        self.cast_tics = STATES[self.cast_state as usize].tics;
        if self.cast_tics <= 0 {
            self.cast_tics = 1;
        }
    }

    /// `F_CastResponder`: a fire/attack input restarts the death animation.
    pub fn cast_attacked(&mut self) {
        if self.stage != FinaleStage::Cast {
            return;
        }
        let ty = CAST_ORDER[self.cast_num].0;
        let death = MOBJINFO[ty as usize].deathstate;
        if !self.cast_death {
            self.sounds.push(MOBJINFO[ty as usize].deathsound);
            self.cast_death = true;
            self.cast_state = death;
            self.cast_tics = CAST_TICS;
            self.cast_attacking = false;
        }
    }

    /// Drain the queued sounds (played through the host's audio seam).
    pub fn drain_sounds(&mut self) -> Vec<Sfx> {
        core::mem::take(&mut self.sounds)
    }

    /// The number of text characters revealed so far (`F_TextWrite`).
    pub fn text_shown(&self) -> usize {
        (self.count / TEXTSPEED).max(0) as usize
    }

    /// `F_Drawer`: paint the current stage into an indexed 320x200 screen.
    pub fn draw(&self, manager: &WadManager, screen: &mut [u8]) {
        match self.stage {
            FinaleStage::Cast => self.draw_cast(manager, screen),
            FinaleStage::Text => self.draw_text(manager, screen),
            FinaleStage::ArtScreen => self.draw_art_screen(manager, screen),
        }
    }

    fn draw_text(&self, manager: &WadManager, screen: &mut [u8]) {
        // erase the entire screen to the background flat
        draw_flat_tile(manager, screen, self.flat);
        // type the revealed text
        let mut x = 10;
        let mut y = 10;
        for ch in self.text.chars().take(self.text_shown()) {
            if ch == '\n' {
                x = 10;
                y += 12;
            } else {
                draw_font_char(manager, screen, ch as u8, x, y);
                x += 7;
            }
        }
    }

    fn draw_cast(&self, manager: &WadManager, screen: &mut [u8]) {
        draw_full_patch(manager, screen, b"BOSSBACK");
        // print the name
        let name = CAST_ORDER[self.cast_num].1;
        let x = (v_video::SCREENWIDTH as i32 - name.len() as i32 * 7) / 2;
        for (i, ch) in name.bytes().enumerate() {
            draw_font_char(manager, screen, ch, x + i as i32 * 7, 180);
        }

        // draw the current frame in the middle of the screen
        let st = STATES[self.cast_state as usize];
        draw_state_sprite(manager, screen, st.sprite as usize, st.frame & crate::info::FF_FRAMEMASK);
    }

    fn draw_art_screen(&self, manager: &WadManager, screen: &mut [u8]) {
        // `F_ArtScreenDrawer`: the bunny scroll of episode 3 is a host
        // animation concern; the static screens are:
        // Episode 3 is the bunny scroll (a host animation); the static
        // screens match `F_ArtScreenDrawer` (retail E1 gets the CREDIT
        // screen, everything else falls back to HELP2 like shareware).
        let name: &[u8; 8] = match self.episode {
            1 if !self.commercial && self.episode == 1 => b"HELP2   ",
            2 => b"VICTORY2",
            4 => b"ENDPIC  ",
            _ => b"HELP2   ",
        };
        draw_full_patch(manager, screen, name);
    }
}

/// Tile a 64x64 flat across the screen (the `F_TextWrite` background).
fn draw_flat_tile(manager: &WadManager, screen: &mut [u8], name: &str) {
    let lname = LumpName::from_bytes(name.as_bytes());
    let Ok(flat) = manager.lump_data_by_name(&lname) else {
        screen.fill(0);
        return;
    };
    for y in 0..v_video::SCREENHEIGHT {
        for x in 0..v_video::SCREENWIDTH {
            let fx = x % 64;
            let fy = y % 64;
            if let Some(slot) = screen.get_mut(y * v_video::SCREENWIDTH + x) {
                *slot = flat.get(fy * 64 + fx).copied().unwrap_or(0);
            }
        }
    }
}

/// Draw a full-screen patch at (0,0) (vanilla `V_DrawPatch` full screens).
fn draw_full_patch(manager: &WadManager, screen: &mut [u8], name: &[u8; 8]) {
    let lname = LumpName(*name);
    if let Ok(data) = manager.lump_data_by_name(&lname) {
        let patch = Patch::from_lump(data);
        v_video::draw_patch(
            screen,
            v_video::SCREENWIDTH as i32,
            v_video::SCREENHEIGHT as i32,
            &patch,
            0,
            0,
        );
    }
}

/// One `STCFN` character (the `F_WriteText` font).
fn draw_font_char(manager: &WadManager, screen: &mut [u8], ch: u8, x: i32, y: i32) {
    let upper = ch.to_ascii_uppercase();
    let mut name = *b"STCFN000";
    name[5] = b'0' + upper / 100;
    name[6] = b'0' + (upper / 10) % 10;
    name[7] = b'0' + upper % 10;
    let lname = LumpName(name);
    if let Ok(data) = manager.lump_data_by_name(&lname) {
        let patch = Patch::from_lump(data);
        v_video::draw_patch(
            screen,
            v_video::SCREENWIDTH as i32,
            v_video::SCREENHEIGHT as i32,
            &patch,
            x,
            y,
        );
    }
}

/// Draw a sprite frame centered, like `F_CastDrawer`'s scaler block.
fn draw_state_sprite(manager: &WadManager, screen: &mut [u8], sprite: usize, frame: i32) {
    // Resolve the lump name from the SPRNAMES table + frame/rotation A.
    let Some(name) = crate::info::SPRNAMES.get(sprite) else {
        return;
    };
    let frame_char = (b'A' + frame as u8) as char;
    // Rotation 1 = the front view (`sprframe->lump[0]` in the original).
    let lump_name = format!("{}{}1", name, frame_char);
    let lname = LumpName::from_bytes(lump_name.as_bytes());
    if let Ok(data) = manager.lump_data_by_name(&lname) {
        let patch = Patch::from_lump(data);
        // centered-ish at the same spot vanilla scales it to
        let x = (v_video::SCREENWIDTH as i32 - patch.width) / 2 - patch.leftoffset;
        let y = 170 - patch.height - patch.topoffset;
        v_video::draw_patch(
            screen,
            v_video::SCREENWIDTH as i32,
            v_video::SCREENHEIGHT as i32,
            &patch,
            x,
            y,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_right_screen() {
        let fin = Finale::start(false, 1, 8);
        assert_eq!(fin.text, E1_TEXT);
        assert_eq!(fin.flat, "FLOOR4_8");
        let fin = Finale::start(true, 1, 30);
        assert_eq!(fin.text, C4_TEXT);
        let fin = Finale::start(false, 4, 8);
        assert_eq!(fin.text, E4_TEXT);
        // Unknown position keeps the DOOM E1 screen like vanilla's `junk`.
        let fin = Finale::start(true, 1, 7);
        assert_eq!(fin.flat, "FLOOR4_8");
    }

    #[test]
    fn text_types_then_art_screen() {
        let mut fin = Finale::start(false, 1, 8);
        for _ in 0..100 {
            fin.ticker(false);
        }
        assert_eq!(fin.stage, FinaleStage::Text);
        assert!(fin.text_shown() > 0);
        let mut guard = 0;
        while fin.stage == FinaleStage::Text && guard < 100_000 {
            fin.ticker(false);
            guard += 1;
        }
        assert_eq!(fin.stage, FinaleStage::ArtScreen);
        // The stage change resets the counter (a few tics past it are fine).
        assert!(fin.count < 10);
    }

    #[test]
    fn commercial_skip_and_cast() {
        let mut fin = Finale::start(true, 1, 6);
        for _ in 0..60 {
            fin.ticker(false);
        }
        assert!(fin.ticker(true), "map 6 skip advances the world");

        let mut fin = Finale::start(true, 1, 30);
        for _ in 0..60 {
            fin.ticker(false);
        }
        fin.ticker(true);
        assert_eq!(fin.stage, FinaleStage::Cast);

        // The cast advances through states and wraps around.
        let start_state = fin.cast_state;
        for _ in 0..2000 {
            fin.ticker(false);
        }
        assert_ne!(fin.cast_state, start_state);
        assert!(!fin.drain_sounds().is_empty());
    }
}
