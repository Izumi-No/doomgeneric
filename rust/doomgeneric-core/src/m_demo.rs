//! Demo recording and playback, port of the demo parts of `g_game.c`
//! (`G_ReadDemoTiccmd`/`G_WriteDemoTiccmd`) and `d_main.c`'s header
//! (`G_DeferedPlayDemo` data).
//!
//! Demos are plain byte buffers so the host owns where they live (files,
//! memory, network); the format is vanilla v1.9 low-res turn, so original
//! `.lmp` files play back too.

use crate::d_ticcmd::TicCmd;
use crate::doomdef::MAX_PLAYERS;

/// The vanilla demo header magic/version byte (`demoversion` 109).
pub const DEMO_VERSION: u8 = 109;
/// The extended "longtics" demo version (Chocolate Doom v1.10, 221): the
/// turn is recorded at full 16-bit resolution so playbacks are exact.
pub const DEMO_VERSION_LONGTICS: u8 = 221;

/// The demo header the original engine writes before the ticcmd stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemoHeader {
    pub skill: i32,
    pub episode: i32,
    pub map: i32,
    pub deathmatch: i32,
    pub respawn: bool,
    pub fast: bool,
    pub nomonsters: bool,
    pub consoleplayer: usize,
    pub playeringame: [bool; MAX_PLAYERS],
}

impl DemoHeader {
    /// Whether this header's stream uses the extended 5-byte records.
    pub fn longtics(data: &[u8]) -> bool {
        data.first() == Some(&DEMO_VERSION_LONGTICS)
    }

    /// The vanilla 13-byte prefix: version, skill, episode, map, deathmatch,
    /// respawn, fast, nomonsters, consoleplayer, playeringame bits and two
    /// filler bytes.
    pub fn to_bytes(&self) -> [u8; 13] {
        let mut out = [0u8; 13];
        out[0] = DEMO_VERSION;
        out[1] = self.skill as u8;
        out[2] = self.episode as u8;
        out[3] = self.map as u8;
        out[4] = self.deathmatch as u8;
        out[5] = self.respawn as u8;
        out[6] = self.fast as u8;
        out[7] = self.nomonsters as u8;
        out[8] = self.consoleplayer as u8;
        for (i, ingame) in self.playeringame.iter().enumerate() {
            if *ingame {
                out[9] |= 1 << i;
            }
        }
        out[10] = 0x1f; // filler
        out[11] = 0x03; // filler
        out[12] = 0; // filler
        out
    }

    /// Parse the vanilla prefix; `None` if it is not a v1.9/longtics demo.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 13 || (data[0] != DEMO_VERSION && data[0] != DEMO_VERSION_LONGTICS) {
            return None;
        }
        let mut playeringame = [false; MAX_PLAYERS];
        for (i, slot) in playeringame.iter_mut().enumerate() {
            *slot = data[9] & (1 << i) != 0;
        }
        Some(DemoHeader {
            skill: data[1] as i32,
            episode: data[2] as i32,
            map: data[3] as i32,
            deathmatch: data[4] as i32,
            respawn: data[5] != 0,
            fast: data[6] != 0,
            nomonsters: data[7] != 0,
            consoleplayer: (data[8] as usize).min(MAX_PLAYERS - 1),
            playeringame,
        })
    }
}

/// `G_WriteDemoTiccmd`: append one tic's worth of commands. The vanilla
/// 1.9 record is 4 bytes: forwardmove, sidemove, angleturn-low-byte,
/// buttons — the high byte of the turn is dropped (low-res turns).
pub fn write_ticcmd(out: &mut alloc::vec::Vec<u8>, cmds: &[TicCmd]) {
    for cmd in cmds {
        out.push(cmd.forwardmove as u8);
        out.push(cmd.sidemove as u8);
        out.push((cmd.angleturn & 0xff) as u8);
        out.push(cmd.buttons);
    }
}

/// `G_WriteDemoTiccmd` in longtics mode (5-byte records, full turn).
pub fn write_ticcmd_long(out: &mut alloc::vec::Vec<u8>, cmds: &[TicCmd]) {
    for cmd in cmds {
        out.push(cmd.forwardmove as u8);
        out.push(cmd.sidemove as u8);
        out.extend_from_slice(&cmd.angleturn.to_le_bytes());
        out.push(cmd.buttons);
    }
}

/// `G_ReadDemoTiccmd`: pull one tic's commands for `players` slots from a
/// demo buffer. Returns `None` at end of data (`G_CheckDemoStatus`).
pub fn read_ticcmd(data: &[u8], pos: &mut usize, players: usize) -> Option<alloc::vec::Vec<TicCmd>> {
    read_ticcmd_stride(data, pos, players, false)
}

/// `G_ReadDemoTiccmd` for either record width.
pub fn read_ticcmd_stride(
    data: &[u8],
    pos: &mut usize,
    players: usize,
    longtics: bool,
) -> Option<alloc::vec::Vec<TicCmd>> {
    let record = if longtics { 5 } else { 4 };
    let stride = players * record;
    if *pos + stride > data.len() {
        return None;
    }
    let mut out = alloc::vec::Vec::with_capacity(players);
    for _ in 0..players {
        let b = *pos;
        out.push(if longtics {
            TicCmd {
                forwardmove: data[b] as i8,
                sidemove: data[b + 1] as i8,
                angleturn: i16::from_le_bytes([data[b + 2], data[b + 3]]),
                buttons: data[b + 4],
                ..TicCmd::default()
            }
        } else {
            TicCmd {
                forwardmove: data[b] as i8,
                sidemove: data[b + 1] as i8,
                // sign-extend the low-res turn byte
                angleturn: (data[b + 2] as i8) as i16,
                buttons: data[b + 3],
                ..TicCmd::default()
            }
        });
        *pos += record;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longtics_round_trip_exactly() {
        let cmd = TicCmd {
            forwardmove: 50,
            sidemove: -24,
            angleturn: -600, // any 16-bit turn survives
            buttons: 3,
            ..TicCmd::default()
        };
        let mut buf = alloc::vec::Vec::new();
        write_ticcmd_long(&mut buf, alloc::slice::from_ref(&cmd));
        assert_eq!(buf.len(), 5);
        let mut pos = 0;
        let back = read_ticcmd_stride(&buf, &mut pos, 1, true).unwrap();
        assert_eq!(back[0], cmd);
    }

    #[test]
    fn header_round_trips_vanilla_format() {
        let h = DemoHeader {
            skill: 3,
            episode: 1,
            map: 2,
            deathmatch: 0,
            respawn: false,
            fast: false,
            nomonsters: false,
            consoleplayer: 0,
            playeringame: [true, false, false, false],
        };
        let bytes = h.to_bytes();
        assert_eq!(bytes[0], 109);
        assert_eq!(bytes[9], 1);
        let back = DemoHeader::parse(&bytes).unwrap();
        assert_eq!(back, h);
        assert!(DemoHeader::parse(&bytes[..8]).is_none());
    }

    #[test]
    fn ticcmds_round_trip_lowres_turn() {
        let cmd = TicCmd {
            forwardmove: 50,
            sidemove: -24,
            angleturn: -100, // fits a low-res byte
            buttons: 3,
            ..TicCmd::default()
        };
        let mut buf = alloc::vec::Vec::new();
        write_ticcmd(&mut buf, alloc::slice::from_ref(&cmd));
        assert_eq!(buf.len(), 4);
        let mut pos = 0;
        let back = read_ticcmd(&buf, &mut pos, 1).unwrap();
        assert_eq!(back[0].forwardmove, 50);
        assert_eq!(back[0].sidemove, -24);
        assert_eq!(back[0].angleturn, -100);
        assert_eq!(back[0].buttons, 3);
        assert!(read_ticcmd(&buf, &mut pos, 1).is_none(), "demo ends");
    }
}
