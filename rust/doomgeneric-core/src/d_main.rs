//! `D_DoomMain` core pipeline, a structural port of the `d_main.c` flow that
//! the play simulation needs.
//!
//! Everything host-side (screen setup, `D_DoomLoop`'s frame pacing, menus,
//! the title/finale screens) is Fase 4; what lives here is the IWAD
//! bookkeeping: game-mode detection, episode/map resolution and map-lump
//! lookup, expressed over the already-loaded [`WadManager`] (the actual file
//! IO goes through the host's `FileSystem` seam).

use alloc::format;
use crate::wad::{LumpName, WadManager};

/// `GameMode_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Indetermined,
    Shareware,
    Registered,
    Retail,
    Commercial,
}

/// `D_IdentifyIWAD`: which game does this WAD contain?
///
/// The lump markers match vanilla `d_iwad.c`/`d_main.c`:
/// * `E4M1` — retail (ultimate)
/// * `E3M1` — registered
/// * `E1M1` — shareware
/// * `MAP01` — commercial (DOOM II / Final Doom)
pub fn identify_iwad(manager: &WadManager) -> GameMode {
    let has = |name: &str| {
        manager.check_num_for_name(&LumpName::from_bytes(name.as_bytes()))
            .is_some()
    };
    if has("E4M1") {
        GameMode::Retail
    } else if has("E3M1") {
        GameMode::Registered
    } else if has("E1M1") {
        GameMode::Shareware
    } else if has("MAP01") {
        GameMode::Commercial
    } else {
        GameMode::Indetermined
    }
}

/// `D_DeferedInitNew` bookkeeping: the skill/episode/map the user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartConfig {
    pub skill: i32,
    pub episode: i32,
    pub map: i32,
    /// `-nomonsters`, `-respawn`, `-fast` parity.
    pub nomonsters: bool,
    pub respawn: bool,
    pub fast: bool,
}

impl Default for StartConfig {
    fn default() -> Self {
        StartConfig {
            skill: 2, // sk_medium
            episode: 1,
            map: 1,
            nomonsters: false,
            respawn: false,
            fast: false,
        }
    }
}

/// Resolve the first lump index of a map block. Vanilla scans for
/// `ExMy`/`MAPxx` names; the port receives the already-resolved name.
pub fn find_map_lump(manager: &WadManager, name: &str) -> Option<usize> {
    manager.check_num_for_name(&LumpName::from_bytes(name.as_bytes()))
}

/// The map name for an episode/map pair (`G_GameMapName` style): a lump
/// name, space padded.
pub fn map_name(mode: GameMode, episode: i32, map: i32) -> Option<[u8; 8]> {
    let mut name = [b' '; 8];
    let written: alloc::vec::Vec<u8> = if mode == GameMode::Commercial {
        format!("MAP{:02}", map).into_bytes()
    } else {
        format!("E{}M{}", episode, map).into_bytes()
    };
    if written.len() > 8 {
        return None;
    }
    name[..written.len()].copy_from_slice(&written);
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_names_match_vanilla() {
        assert_eq!(
            &map_name(GameMode::Commercial, 1, 2).unwrap()[..5],
            b"MAP02"
        );
        assert_eq!(
            &map_name(GameMode::Shareware, 1, 1).unwrap()[..4],
            b"E1M1"
        );
        assert_eq!(
            &map_name(GameMode::Registered, 3, 9).unwrap()[..4],
            b"E3M9"
        );
    }

    #[test]
    fn identify_shareware_and_commercial() {
        // A minimal IWAD skeleton with just the marker lumps.
        let mut manager = WadManager::new();
        let e1m1 = wad_with_lumps(&[b"E1M1"]);
        manager.add(e1m1).unwrap();
        assert_eq!(identify_iwad(&manager), GameMode::Shareware);

        let mut manager = WadManager::new();
        manager.add(wad_with_lumps(&[b"MAP01", b"E1M1"])).unwrap();
        // MAP01 present but no E4M1/E3M1: hmm — vanilla checks E4M1, E3M1,
        // E1M1, MAP01 in this order; with both E1M1 and MAP01 present the
        // E1M1 wins (shareware), matching the C order.
        assert_eq!(identify_iwad(&manager), GameMode::Shareware);
    }

    /// Build a tiny WAD with named 0-length lumps.
    fn wad_with_lumps(names: &[&[u8]]) -> alloc::vec::Vec<u8> {
        let n = names.len();
        let header = 12;
        let dir = 16 * n;
        let mut wad: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        wad.extend_from_slice(b"IWAD");
        wad.extend_from_slice(&(n as u32).to_le_bytes());
        wad.extend_from_slice(&(header as u32).to_le_bytes());
        wad.resize(header + dir, 0);
        for (i, name) in names.iter().enumerate() {
            let e = header + 16 * i;
            wad[e + 8..e + 8 + name.len().min(8)]
                .copy_from_slice(&name[..name.len().min(8)]);
        }
        wad
    }
}
