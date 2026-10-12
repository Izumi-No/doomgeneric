//! Sound and music request queue, port of `s_sound.c`.
//!
//! The core never mixes audio. Like vanilla's `I_StartSound` seam, every
//! `S_StartSound`/`S_ChangeMusic` becomes a request in a queue that the host
//! drains through its [`Audio`](crate::platform::Audio) implementation and
//! plays however it wants (SDL mixer, PCM stream, dropped entirely...).
//!
//! Positional attenuation (`S_AdjustSoundParams`) is computed here so the
//! queue stays host-agnostic: requests carry `volume` and stereo `sep`
//! already resolved against the listener.

use alloc::vec::Vec;

use crate::doomdef::MAX_PLAYERS;
use crate::fixed::{fixed_mul, Fixed, FRAC_BITS, FRAC_UNIT};
use crate::r_main::point_to_angle_from;
use crate::sounds::{Music, Sfx, NUMMUSIC as NUM_MUSIC};
use crate::tables::{finesine_index, ANGLETOFINESHIFT};

/// `S_CLIPPING_DIST`.
const S_CLIPPING_DIST: Fixed = 1200 * FRAC_UNIT;
/// `S_CLOSE_DIST`.
const S_CLOSE_DIST: Fixed = 160 * FRAC_UNIT;
/// `S_ATTENUATOR`.
const S_ATTENUATOR: Fixed = (S_CLIPPING_DIST - S_CLOSE_DIST) >> FRAC_BITS;
/// `S_STEREO_SWING`.
const S_STEREO_SWING: Fixed = 96 * FRAC_UNIT;

/// Default `snd_SfxVolume`/`snd_MusicVolume` (0..15).
pub const DEFAULT_SFX_VOLUME: i32 = 8;
pub const DEFAULT_MUSIC_VOLUME: i32 = 8;

/// One `S_StartSound`: already attenuated for the host.
#[derive(Clone, Copy, Debug)]
pub struct SoundRequest {
    pub sfx: Sfx,
    /// 0..15, resolved from the distance to the listener.
    pub volume: i32,
    /// Stereo separation (128 = centered).
    pub sep: i32,
}

/// A music change request (`S_ChangeMusic`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicRequest {
    pub music: Music,
    pub looping: bool,
}

/// The `s_sound.c` globals, owned by the [`crate::p_world::World`].
#[derive(Clone, Debug)]
pub struct SoundSystem {
    /// `snd_SfxVolume` (0..15).
    pub sfx_volume: i32,
    /// `snd_MusicVolume` (0..15).
    pub music_volume: i32,
    /// `mus_playing`.
    pub playing: Option<Music>,
    requests: Vec<SoundRequest>,
    music_request: Option<MusicRequest>,
}

impl Default for SoundSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundSystem {
    pub fn new() -> Self {
        SoundSystem {
            sfx_volume: DEFAULT_SFX_VOLUME,
            music_volume: DEFAULT_MUSIC_VOLUME,
            playing: None,
            requests: Vec::new(),
            music_request: None,
        }
    }

    /// `S_AdjustSoundParams` + `S_StartSound`: enqueue a positioned sound.
    ///
    /// `listener` is the display player's `(x, y, angle)`; `None` means the
    /// listener is gone (drop the sound like vanilla does).
    #[allow(clippy::too_many_arguments)]
    pub fn start_sound_at(
        &mut self,
        sfx: Sfx,
        source: (Fixed, Fixed),
        listener: Option<(Fixed, Fixed, u32)>,
        map8_hack: bool,
    ) {
        let Some((lx, ly, langle)) = listener else {
            return;
        };

        // Approximate euclidean distance (from _GG1_ p.428).
        let adx = (lx - source.0).abs();
        let ady = (ly - source.1).abs();
        let approx_dist = adx + ady - ((if adx < ady { adx } else { ady }) >> 1);

        if !map8_hack && approx_dist > S_CLIPPING_DIST {
            return;
        }

        // Angle of source to listener.
        let mut angle = point_to_angle_from(lx, ly, source.0, source.1);
        if angle > langle {
            angle -= langle;
        } else {
            angle = angle.wrapping_add(0xffff_ffffu32.wrapping_sub(langle));
        }
        let fine = angle >> ANGLETOFINESHIFT;

        // Stereo separation.
        let sep = 128 - (fixed_mul(S_STEREO_SWING, finesine_index(fine)) >> FRAC_BITS);

        // Volume calculation.
        let vol = if approx_dist < S_CLOSE_DIST {
            self.sfx_volume
        } else if map8_hack {
            let d = approx_dist.min(S_CLIPPING_DIST);
            15 + ((self.sfx_volume - 15) * ((S_CLIPPING_DIST - d) >> FRAC_BITS)) / S_ATTENUATOR
        } else {
            (self.sfx_volume * ((S_CLIPPING_DIST - approx_dist) >> FRAC_BITS)) / S_ATTENUATOR
        };

        if vol > 0 {
            self.requests.push(SoundRequest {
                sfx,
                volume: vol,
                sep,
            });
        }
    }

    /// A UI/global sound: full volume, centered (vanilla `S_StartSound(NULL, ..)`).
    pub fn start_sound(&mut self, sfx: Sfx) {
        self.requests.push(SoundRequest {
            sfx,
            volume: self.sfx_volume,
            sep: 128,
        });
    }

    /// `S_ChangeMusic`.
    pub fn change_music(&mut self, music: Music, looping: bool) {
        if self.playing == Some(music) {
            return;
        }
        self.playing = Some(music);
        self.music_request = Some(MusicRequest { music, looping });
    }

    /// `S_StartMusic`.
    pub fn start_music(&mut self, music: Music) {
        self.change_music(music, false);
    }

    /// `S_StopMusic`.
    pub fn stop_music(&mut self) {
        self.playing = None;
        self.music_request = Some(MusicRequest {
            music: Music::mus_None,
            looping: false,
        });
    }

    /// Take all queued sound requests (the host's per-tic drain).
    pub fn drain_sounds(&mut self) -> Vec<SoundRequest> {
        core::mem::take(&mut self.requests)
    }

    /// Take the pending music change, if any.
    pub fn take_music(&mut self) -> Option<MusicRequest> {
        self.music_request.take()
    }
}

/// The per-level music (`S_LevelMusic`): episode music is `mus_e1m1 +
/// (episode-1)*9 + (map-1)`, commercial music runs from `mus_runnin`.
pub fn s_level_music(commercial: bool, episode: i32, map: i32) -> Music {
    if commercial {
        let idx = (Music::mus_runnin as i32 + map - 1).clamp(1, NUM_MUSIC as i32 - 1);
        Music::from_index(idx)
    } else {
        let idx = (Music::mus_e1m1 as i32 + (episode - 1) * 9 + (map - 1)).clamp(1, NUM_MUSIC as i32 - 1);
        Music::from_index(idx)
    }
}

/// Frags tally per player (`wbstartstruct_t::plyr`), shared with the
/// intermission.
#[derive(Clone, Copy, Debug, Default)]
pub struct WbPlayerStats {
    pub in_game: bool,
    pub skills: i32,
    pub sitems: i32,
    pub ssecret: i32,
    pub stime: i32,
    pub frags: [i32; MAX_PLAYERS],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener() -> Option<(Fixed, Fixed, u32)> {
        Some((0, 0, 0))
    }

    #[test]
    fn close_sound_is_full_volume_and_centered_behind() {
        let mut s = SoundSystem::new();
        s.start_sound_at(Sfx::sfx_pistol, (0, 0), listener(), false);
        let req = s.drain_sounds().pop().unwrap();
        assert_eq!(req.volume, DEFAULT_SFX_VOLUME);
        // Dead ahead: centered (the vanilla `0xffffffff` wrap shifts the
        // finesine index one entry, leaving sep at 129 like the original).
        assert_eq!(req.sep, 129);
        assert!(s.drain_sounds().is_empty());
    }

    #[test]
    fn far_sound_is_clipped() {
        let mut s = SoundSystem::new();
        s.start_sound_at(Sfx::sfx_pistol, (2000 << 16, 0), listener(), false);
        assert!(s.drain_sounds().is_empty());
    }

    #[test]
    fn side_sound_gets_separation() {
        let mut s = SoundSystem::new();
        // Source to the "right" of a north-facing listener (angle 0 = east in
        // DOOM; any off-axis source shifts the separation away from 128).
        s.start_sound_at(Sfx::sfx_pistol, (0, 300 << 16), listener(), false);
        let req = s.drain_sounds().pop().unwrap();
        assert_ne!(req.sep, 128);
    }

    #[test]
    fn music_changes_are_deduped_and_drained() {
        let mut s = SoundSystem::new();
        s.start_music(Music::mus_e1m1);
        s.start_music(Music::mus_e1m1); // same track: no new request
        assert_eq!(s.take_music().unwrap().music, Music::mus_e1m1);
        assert!(s.take_music().is_none());
        s.change_music(Music::mus_e1m2, true);
        let req = s.take_music().unwrap();
        assert_eq!(req.music, Music::mus_e1m2);
        assert!(req.looping);
    }

    #[test]
    fn level_music_selection() {
        assert_eq!(s_level_music(false, 1, 1), Music::mus_e1m1);
        assert_eq!(s_level_music(false, 2, 3), Music::mus_e2m3);
        assert_eq!(s_level_music(true, 1, 1), Music::mus_runnin);
    }
}
