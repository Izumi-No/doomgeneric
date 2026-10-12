//! HUD messages, port of `hu_stuff.c`.
//!
//! Vanilla keeps a one-line message with a timer per player; the core keeps
//! the same queue (the chat line and widgets are host rendering concerns).

use alloc::string::String;
use alloc::vec::Vec;

use crate::doomdef::MAX_PLAYERS;

/// Tics a message stays on screen (`HU_MSGTIMEOUT`).
pub const HU_MSGTIMEOUT: i32 = 4 * crate::doomdef::TICRATE as i32;

/// One message line (`message_t`).
#[derive(Clone, Debug)]
pub struct Message {
    pub text: String,
    pub timer: i32,
}

/// Per-player message list (`player_msg`).
#[derive(Clone, Debug, Default)]
pub struct Messages {
    queue: Vec<Message>,
}

impl Messages {
    /// `HU_AddMessage`: put a line on the HUD.
    pub fn push(&mut self, text: &str) {
        self.queue.push(Message {
            text: String::from(text),
            timer: HU_MSGTIMEOUT,
        });
    }

    /// The newest, still-visible message (the one line vanilla shows).
    pub fn current(&self) -> Option<&Message> {
        self.queue.first()
    }

    /// `HU_Ticker`: age the visible message out.
    pub fn ticker(&mut self) {
        if let Some(front) = self.queue.first_mut() {
            front.timer -= 1;
            if front.timer <= 0 {
                self.queue.remove(0);
            }
        }
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

/// The whole `hu_stuff.c` state (`HU_Start`).
#[derive(Clone, Debug)]
pub struct Hud {
    pub messages: [Messages; MAX_PLAYERS],
}

impl Default for Hud {
    fn default() -> Self {
        Self::new()
    }
}

impl Hud {
    pub fn new() -> Self {
        Hud {
            messages: Default::default(),
        }
    }

    /// Feed `player.message` (set by the gameplay layer) into the HUD.
    pub fn ticker(&mut self, players: &mut [crate::d_player::Player; MAX_PLAYERS]) {
        for (msgs, player) in self.messages.iter_mut().zip(players.iter_mut()) {
            if let Some(text) = player.message.take() {
                msgs.push(text);
            }
            msgs.ticker();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_expire_after_timeout() {
        let mut m = Messages::default();
        m.push("hello");
        assert_eq!(m.current().unwrap().text, "hello");
        for _ in 0..HU_MSGTIMEOUT {
            m.ticker();
        }
        assert!(m.current().is_none());
    }

    #[test]
    fn hud_consumes_player_messages() {
        let mut hud = Hud::new();
        let mut players = core::array::from_fn(|_| crate::d_player::Player::new());
        players[0].message = Some("you need a red key");
        hud.ticker(&mut players);
        assert!(players[0].message.is_none());
        assert_eq!(hud.messages[0].current().unwrap().text, "you need a red key");
    }
}
