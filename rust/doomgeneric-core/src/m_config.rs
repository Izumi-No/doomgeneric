//! Configuration store, port of `m_config.c`'s data side.
//!
//! The core owns the key/value pairs and the (de)serialization; where the
//! bytes come from and go to is the host's `FileSystem` seam. Defaults come
//! from vanilla's `M_SetDefaults`-style table; joystick/driver-specific
//! entries stay host-side.

use alloc::string::String;
use alloc::vec::Vec;

use crate::s_sound::{DEFAULT_MUSIC_VOLUME, DEFAULT_SFX_VOLUME};

/// The default table (`m_config.c`); values mirror vanilla where they map
/// onto core state (controls are host-interpreted).
pub fn defaults() -> Vec<(String, String)> {
    vec_into([
        ("mouse_sensitivity", "5"),
        ("sfx_volume", "10"),
        ("music_volume", "10"),
        ("show_messages", "1"),
        ("screenblocks", "10"),
        ("detaillevel", "0"),
        ("use_mouse", "1"),
        ("use_joystick", "0"),
        ("always_run", "0"),
        ("gamma_correction", "0"),
        ("chatmacro0", "No"),
    ])
}

fn vec_into(pairs: [(&str, &str); 11]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (String::from(*k), String::from(*v)))
        .collect()
}

/// A parsed config (`default_t` + values).
#[derive(Clone, Debug, Default)]
pub struct Config {
    entries: Vec<(String, String)>,
}

impl Config {
    /// Load defaults (then the host overlays its file's values).
    pub fn with_defaults() -> Self {
        Config {
            entries: defaults(),
        }
    }

    /// Parse `default.cfg`-style lines (`M_LoadDefaults` body).
    pub fn parse(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = String::from(value);
            } else {
                self.entries.push((key, String::from(value)));
            }
        }
    }

    /// Serialize back (`M_SaveDefaults` body).
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.entries {
            out.push_str(k);
            out.push('=');
            out.push_str(v);
            out.push('\n');
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: &str, value: &str) {
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| k == key) {
            slot.1 = String::from(value);
        } else {
            self.entries.push((String::from(key), String::from(value)));
        }
    }

    /// Numeric accessor with a fallback.
    pub fn get_int(&self, key: &str, default: i32) -> i32 {
        self.get(key)
            .map(|v| crate::m_misc::string_to_int(v, default))
            .unwrap_or(default)
    }
}

/// Map the config onto the sound system volumes (the only core-owned
/// effects; the rest are interpreted by the host input/render layers).
pub fn apply_to_sound(config: &Config, sounds: &mut crate::s_sound::SoundSystem) {
    let sfx = config.get_int("sfx_volume", DEFAULT_SFX_VOLUME).clamp(0, 15);
    let music = config
        .get_int("music_volume", DEFAULT_MUSIC_VOLUME)
        .clamp(0, 15);
    sounds.sfx_volume = sfx;
    sounds.music_volume = music;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_key_values() {
        let mut c = Config::with_defaults();
        c.parse("sfx_volume=3\n; comment\nnoise\nunknown_key=hello\n");
        assert_eq!(c.get("sfx_volume"), Some("3"));
        assert_eq!(c.get("unknown_key"), Some("hello"));
        let text = c.serialize();
        assert!(text.contains("sfx_volume=3\n"));
        assert!(text.contains("unknown_key=hello\n"));

        let mut c2 = Config::with_defaults();
        c2.parse(&text);
        assert_eq!(c2.get("sfx_volume"), Some("3"));
    }

    #[test]
    fn sound_volumes_apply_clamped() {
        let mut c = Config::with_defaults();
        c.parse("sfx_volume=99\nmusic_volume=-3\n");
        let mut s = crate::s_sound::SoundSystem::new();
        apply_to_sound(&c, &mut s);
        assert_eq!(s.sfx_volume, 15);
        assert_eq!(s.music_volume, 0);
    }
}
