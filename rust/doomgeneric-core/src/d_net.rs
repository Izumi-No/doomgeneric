//! Netgame session, the play-simulation side of `d_net.c`.
//!
//! Vanilla routes every player's `ticcmd_t` through UDP with a maketic/
//! ticdup gear box; the core keeps only the contract that matters for the
//! simulation: each tic, every node broadcasts its command through the
//! [`Network`] seam and a tic only runs when all players' commands arrived.
//! The host owns the transport (sockets, loopback, replayed captures...).

use alloc::collections::VecDeque;
use alloc::rc::Rc;
use core::cell::RefCell;
use alloc::vec::Vec;

use crate::d_ticcmd::TicCmd;
use crate::doomdef::MAX_PLAYERS;
use crate::platform::Network;

/// One player's command datagram: `[player, fwd, side, turn-lo, buttons]`
/// (the low-res turn of the demo format).
fn encode(player: usize, cmd: &TicCmd) -> [u8; 5] {
    [
        player as u8,
        cmd.forwardmove as u8,
        cmd.sidemove as u8,
        (cmd.angleturn & 0xff) as u8,
        cmd.buttons,
    ]
}

fn decode(bytes: &[u8]) -> Option<(usize, TicCmd)> {
    if bytes.len() != 5 {
        return None;
    }
    Some((
        (bytes[0] as usize).min(MAX_PLAYERS - 1),
        TicCmd {
            forwardmove: bytes[1] as i8,
            sidemove: bytes[2] as i8,
            angleturn: (bytes[3] as i8) as i16,
            buttons: bytes[4],
            ..TicCmd::default()
        },
    ))
}

/// A `LoopbackNet` half: a test/local transport where the "network" is an
/// in-memory queue pair (vanilla's `I_InitNetwork` loopback degenerates to
/// exactly this for a single node).
pub struct LoopbackNet {
    inbox: Rc<RefCell<VecDeque<Vec<u8>>>>,
    peer_out: Rc<RefCell<VecDeque<Vec<u8>>>>,
}

/// Create a connected pair of loopback transports.
pub fn loopback_pair() -> (LoopbackNet, LoopbackNet) {
    let a_in: Rc<RefCell<VecDeque<Vec<u8>>>> = Rc::new(RefCell::new(VecDeque::new()));
    let b_in: Rc<RefCell<VecDeque<Vec<u8>>>> = Rc::new(RefCell::new(VecDeque::new()));
    (
        LoopbackNet {
            inbox: Rc::clone(&a_in),
            peer_out: Rc::clone(&b_in),
        },
        LoopbackNet {
            inbox: Rc::clone(&b_in),
            peer_out: Rc::clone(&a_in),
        },
    )
}

impl Network for LoopbackNet {
    fn send(&mut self, bytes: &[u8]) {
        self.peer_out.borrow_mut().push_back(bytes.to_vec());
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        self.inbox.borrow_mut().pop_front()
    }
}

/// The netgame state one node keeps (`maketic`, the pending tic commands).
pub struct NetSession {
    pub players: usize,
    pub consoleplayer: usize,
    /// `ticdup`: how many game tics ride on one net tic (1 = fine grain).
    pub ticdup: i32,
    /// Commands collected for the running net tic, one slot per player.
    pending: Vec<Option<TicCmd>>,
    /// `maketic`: how many net tics completed.
    pub maketic: i32,
}

impl NetSession {
    pub fn new(players: usize, consoleplayer: usize, ticdup: i32) -> Self {
        let players = players.clamp(1, MAX_PLAYERS);
        NetSession {
            players,
            consoleplayer: consoleplayer.min(players - 1),
            ticdup: ticdup.max(1),
            pending: alloc::vec![None; players],
            maketic: 0,
        }
    }

    /// `D_NetSend`/`D_NetGet` for one net tic: broadcast the local command,
    /// absorb everything that arrived, and run the tic once the set is
    /// complete. Returns the per-player command list in player order.
    pub fn net_tic(&mut self, net: &mut dyn Network, local: &TicCmd) -> Option<Vec<TicCmd>> {
        net.send(&encode(self.consoleplayer, local));
        // The local node's own command counts immediately, like vanilla
        // writing it into its own `netcmds` slot.
        if let Some(slot) = self.pending.get_mut(self.consoleplayer) {
            *slot = Some(*local);
        }
        while let Some(msg) = net.recv() {
            if let Some((player, cmd)) = decode(&msg) {
                if let Some(slot) = self.pending.get_mut(player) {
                    *slot = Some(cmd);
                }
            }
        }
        if self.pending.iter().any(|s| s.is_none()) {
            return None; // not everyone checked in yet
        }
        let out: Vec<TicCmd> = self
            .pending
            .iter()
            .map(|s| (*s).unwrap_or_default())
            .collect();
        for slot in self.pending.iter_mut() {
            *slot = None;
        }
        self.maketic += 1;
        Some(out)
    }
}

/// Single-player parity (kept from the first pass): a short FIFO between
/// the input poller and `G_Ticker`.
pub struct TicQueue {
    cmds: Vec<TicCmd>,
    players: usize,
}

impl TicQueue {
    pub fn new(players: usize) -> Self {
        TicQueue {
            cmds: alloc::vec![TicCmd::default(); players.max(1)],
            players: players.max(1),
        }
    }

    pub fn players(&self) -> usize {
        self.players
    }

    /// `D_NetSend`: record a player's command for the next tic.
    pub fn set(&mut self, player: usize, cmd: TicCmd) {
        if let Some(slot) = self.cmds.get_mut(player) {
            *slot = cmd;
        }
    }

    /// `D_NetGet`: fetch the buffered commands for this tic.
    pub fn snapshot(&self) -> &[TicCmd] {
        &self.cmds
    }
}

impl NetSession {
    /// Test/debug view of the pending slots.
    pub fn pending_debug(&self) -> &[Option<TicCmd>] {
        &self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_commands() {
        let mut q = TicQueue::new(2);
        let cmd = TicCmd {
            forwardmove: 25,
            ..TicCmd::default()
        };
        q.set(1, cmd);
        assert_eq!(q.snapshot()[1].forwardmove, 25);
        assert_eq!(q.snapshot()[0].forwardmove, 0);
    }

    #[test]
    fn session_runs_only_with_all_players() {
        let (mut net_a, mut net_b) = loopback_pair();
        let mut a = NetSession::new(2, 0, 1);
        let mut b = NetSession::new(2, 1, 1);

        let cmd_a = TicCmd {
            forwardmove: 50,
            ..TicCmd::default()
        };
        let cmd_b = TicCmd {
            buttons: 3,
            ..TicCmd::default()
        };

        // A sends first: nothing to run until B checks in.
        assert!(a.net_tic(&mut net_a, &cmd_a).is_none());
        // B's poll sees A's packet, sends its own: B runs the tic.
        let tic = b.net_tic(&mut net_b, &cmd_b).expect("B has both commands");
        assert_eq!(tic[0], cmd_a);
        assert_eq!(tic[1], cmd_b);
        assert_eq!(b.maketic, 1);
        // A still hasn't seen B's packet.
        assert_eq!(a.maketic, 0);

        // A polls again and the tic completes symmetrically.
        let tic = a.net_tic(&mut net_a, &cmd_a).expect("A has both now");
        assert_eq!(tic[1], cmd_b);
        assert_eq!(a.maketic, 1);
    }

    #[test]
    fn garbled_datagrams_are_dropped() {
        let (mut net_a, mut net_b) = loopback_pair();
        let mut b = NetSession::new(2, 1, 1);

        net_a.send(b"\xff"); // short garbage: dropped by the decoder
        net_a.send(&encode(0, &TicCmd::default()));
        let cmd = TicCmd {
            sidemove: 7,
            ..TicCmd::default()
        };
        // B's own command counts locally, so with A's valid packet the tic
        // completes even though the garbage datagram was dropped.
        let tic = b
            .net_tic(&mut net_b, &cmd)
            .expect("local slot + A's packet complete the tic");
        assert_eq!(b.maketic, 1);
        assert_eq!(tic[0], TicCmd::default(), "garbage did not leak in");
        assert_eq!(tic[1], cmd);
    }
}
