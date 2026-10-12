//! Player input per tick, port of `d_ticcmd.h`.
//!
//! The Doom tick command is a compact struct of signed deltas; multi-player
//! consistency fields (`buttons2`, `inventory`, `lookfly`, `arti`) come from
//! the extended Heretic/Hexen layout the doomgeneric tree carries.

/// `ticcmd_t`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TicCmd {
    /// `*2048` for move.
    pub forwardmove: i8,
    /// `*2048` for move.
    pub sidemove: i8,
    /// `<<16` for angle delta.
    pub angleturn: i16,
    pub chatchar: u8,
    pub buttons: u8,
    /// Consistency check byte.
    pub consistancy: u8,
    pub buttons2: u8,
    pub inventory: i32,
    pub lookfly: u8,
    pub arti: u8,
}

// Button bits (`d_event.h` / `doomdef.h`).
pub const BT_ATTACK: u8 = 1;
pub const BT_USE: u8 = 2;
pub const BT_CHANGE: u8 = 4;
pub const BT_SPECIAL: u8 = 128;
pub const BT_SPECIALMASK: u8 = 3;
pub const BT_WEAPONMASK: u8 = 0x38;
pub const BT_WEAPONSHIFT: u8 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero() {
        assert_eq!(TicCmd::default(), TicCmd::default());
    }

    #[test]
    fn button_bits_match_c() {
        assert_eq!(BT_ATTACK, 1);
        assert_eq!(BT_CHANGE, 4);
        assert_eq!(BT_WEAPONMASK, 0x38);
    }
}
