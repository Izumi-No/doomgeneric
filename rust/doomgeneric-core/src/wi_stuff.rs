//! Intermission screens, port of the `wi_stuff.c` state machine.
//!
//! The counters (kills/items/secrets percentages with the accelerating
//! "binging" animation) are pure game state, so they live in the core. The
//! host draws [`Intermission::state`] however it wants (the vanilla screens
//! are just patches over the `WIMAP` background).

use alloc::vec::Vec;

use crate::doomdef::MAX_PLAYERS;
use crate::s_sound::WbPlayerStats;
use crate::sounds::Sfx;

/// `wbstartstruct_t`.
#[derive(Clone, Copy, Debug, Default)]
pub struct WbStart {
    pub epsd: i32,
    pub last: i32,
    pub next: i32,
    pub maxkills: i32,
    pub maxitems: i32,
    pub maxsecret: i32,
    pub maxfrags: i32,
    pub partime: i32,
    pub pnum: usize,
    pub plyr: [WbPlayerStats; MAX_PLAYERS],
    pub didsecret: bool,
}

/// The animated counters (vanilla `cnt_*` globals).
#[derive(Clone, Copy, Debug, Default)]
pub struct IntermissionCounters {
    pub kills: i32,
    pub items: i32,
    pub secret: i32,
    pub time: i32,
    pub par: i32,
    pub frags: [i32; MAX_PLAYERS],
}

/// `stateenum_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WiState {
    NoState,
    StatCount,
    ShowNextLoc,
    ShowNextEnter,
}

/// Which numbers the drawer wants (the vanilla `WI_draw%` screens).
#[derive(Clone, Copy, Debug, Default)]
pub struct IntermissionState {
    pub state: Option<WiState>,
    pub cnt: IntermissionCounters,
    /// `accelstage`: how many bings have happened (used for the sound cadence
    /// by the host drain below).
    pub bings: i32,
}

/// The intermission module state (vanilla `wi_stuff.c` globals, owned).
#[derive(Clone, Debug)]
pub struct Intermission {
    pub info: WbStart,
    pub state: IntermissionState,
    pub cnt_kills: [i32; MAX_PLAYERS],
    pub cnt_items: [i32; MAX_PLAYERS],
    pub cnt_secret: [i32; MAX_PLAYERS],
    pub cnt_frags: [i32; MAX_PLAYERS],
    accelerate: bool,
    sound_queue: Vec<Sfx>,
}

/// `WI_THRESHOLD`-driven acceleration: each frame the counters advance by an
/// accelerating amount and a `sfx_bing`/`sfx_...pct` is queued.
const ACCEL_START: i32 = 1;

impl Intermission {
    /// `WI_Start` (+ `WI_initStats`): zero the counters, pick the state.
    pub fn start(info: WbStart) -> Self {
        let mut wi = Intermission {
            info,
            state: IntermissionState {
                state: None,
                cnt: IntermissionCounters {
                    par: info.partime / crate::doomdef::TICRATE as i32,
                    ..IntermissionCounters::default()
                },
                bings: 0,
            },
            cnt_kills: [0; MAX_PLAYERS],
            cnt_items: [0; MAX_PLAYERS],
            cnt_secret: [0; MAX_PLAYERS],
            cnt_frags: [0; MAX_PLAYERS],
            accelerate: false,
            sound_queue: Vec::new(),
        };
        wi.init_stats();
        wi
    }

    /// `WI_initStats`: deathmatch tallies frags, cooperators tally the
    /// percentages, single player goes straight to the next-location screen.
    fn init_stats(&mut self) {
        self.state.cnt.kills = 0;
        self.state.cnt.items = 0;
        self.state.cnt.secret = 0;
        self.state.cnt.time = 0;
        self.state.bings = 0;

        // The host is responsible for playing the track
        // (mus_inter/mus_dm2int); the core only tracks the state.

        self.state.state = Some(WiState::StatCount);

        if self.info.maxkills == 0 {
            self.info.maxkills = 1;
        }
        if self.info.maxitems == 0 {
            self.info.maxitems = 1;
        }
        if self.info.maxsecret == 0 {
            self.info.maxsecret = 1;
        }
    }

    /// `WI_Ticker`: advance the state machine for one tic. `skip` is the use
    /// button (skips animations). Returns `true` when the intermission is
    /// over (`WI_End` → the caller runs `G_WorldDone`).
    pub fn ticker(&mut self, skip: bool) -> bool {
        self.accelerate = skip;

        match self.state.state {
            Some(WiState::StatCount) => {
                if self.update_entry_state() {
                    self.state.state = Some(WiState::ShowNextLoc);
                }
            }
            Some(WiState::ShowNextLoc) => {
                if skip {
                    self.state.state = Some(WiState::ShowNextEnter);
                }
            }
            Some(WiState::ShowNextEnter) => {
                if skip {
                    self.state.state = None;
                    return true; // WI_End
                }
            }
            Some(WiState::NoState) | None => return true,
        }
        false
    }

    /// The counter animation (`WI_updateStats`): accelerate the percentages
    /// and time; done when everything reached its total.
    fn update_entry_state(&mut self) -> bool {
        let mut done = true;

        let pnum = self.info.pnum;
        let maxes = (
            self.info.maxkills,
            self.info.maxitems,
            self.info.maxsecret,
        );
        let totals = (
            self.info.plyr[pnum].skills,
            self.info.plyr[pnum].sitems,
            self.info.plyr[pnum].ssecret,
        );

        let accel = if self.accelerate { 3 } else { 1 };
        let mut bump = |cur: &mut i32, total: i32, max: i32, sfx_done: Sfx, sfx_bing: Sfx, done: &mut bool| {
            let target = if max > 0 { (total.wrapping_mul(100)) / max } else { 0 };
            if *cur < target {
                let step = ACCEL_START * accel + *cur / 20;
                *cur = (*cur + step.max(1)).min(target);
                self.sound_queue.push(if *cur >= target { sfx_done } else { sfx_bing });
                *done = false;
            }
        };

        bump(
            &mut self.state.cnt.kills,
            totals.0,
            maxes.0,
            Sfx::sfx_barexp,
            Sfx::sfx_barexp,
            &mut done,
        );
        bump(
            &mut self.state.cnt.items,
            totals.1,
            maxes.1,
            Sfx::sfx_itemup,
            Sfx::sfx_barexp,
            &mut done,
        );
        bump(
            &mut self.state.cnt.secret,
            totals.2,
            maxes.2,
            Sfx::sfx_wpnup,
            Sfx::sfx_barexp,
            &mut done,
        );

        // Time counts up to the elapsed time, in seconds.
        let total_time = self.info.plyr[pnum].stime / crate::doomdef::TICRATE as i32;
        if self.state.cnt.time < total_time {
            self.state.cnt.time = (self.state.cnt.time + 2 * accel).min(total_time);
            done = false;
        }

        if !done {
            self.state.bings += 1;
        }
        done
    }

    /// Drain the sound effects the counters requested (the host plays them).
    pub fn drain_sounds(&mut self) -> Vec<Sfx> {
        core::mem::take(&mut self.sound_queue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> WbStart {
        let mut info = WbStart {
            epsd: 0,
            last: 0,
            next: 1,
            maxkills: 10,
            maxitems: 5,
            maxsecret: 2,
            partime: 30 * crate::doomdef::TICRATE as i32,
            pnum: 0,
            ..WbStart::default()
        };
        info.plyr[0].in_game = true;
        info.plyr[0].skills = 5; // 50%
        info.plyr[0].sitems = 5; // 100%
        info.plyr[0].ssecret = 1; // 50%
        info.plyr[0].stime = 60 * crate::doomdef::TICRATE as i32;
        info
    }

    #[test]
    fn counters_finish_then_skip_ends() {
        let mut wi = Intermission::start(info());
        assert_eq!(wi.state.state, Some(WiState::StatCount));

        let mut done = false;
        for _ in 0..200 {
            done = wi.ticker(false);
            if done {
                break;
            }
        }
        assert!(!done, "stats settle, waiting on the player");
        assert_eq!(wi.state.cnt.kills, 50);
        assert_eq!(wi.state.cnt.items, 100);
        assert_eq!(wi.state.cnt.secret, 50);
        assert_eq!(wi.state.cnt.time, 60);
        assert_eq!(wi.state.cnt.par, 30);
        assert_eq!(wi.state.state, Some(WiState::ShowNextLoc));

        // Skipping hops through the next-location screens and ends.
        assert!(!wi.ticker(true)); // ShowNextEnter
        assert!(wi.ticker(true)); // WI_End
    }

    #[test]
    fn acceleration_speeds_the_bings() {
        let mut calm = Intermission::start(info());
        let mut fast = Intermission::start(info());
        for _ in 0..3 {
            calm.ticker(false);
            fast.ticker(true);
        }
        assert!(fast.state.cnt.kills > calm.state.cnt.kills);
        assert!(!calm.drain_sounds().is_empty());
    }
}
