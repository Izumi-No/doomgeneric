//! Deterministic random number generators, port of `m_random.c`.
//!
//! Doom keeps *two* independent lookup-table streams: one for the play
//! simulation (`P_Random`) and one for the rest (`M_Random`). Demo playback
//! depends on `P_Random` being bit-for-bit identical.

const RND_TABLE: [u8; 256] = [
    0, 8, 109, 220, 222, 241, 149, 107, 75, 248, 254, 140, 16, 66, 74, 21, 211, 47, 80, 242, 154,
    27, 205, 128, 161, 89, 77, 36, 95, 110, 85, 48, 212, 140, 211, 249, 22, 79, 200, 50, 28, 188,
    52, 140, 202, 120, 68, 145, 62, 70, 184, 190, 91, 197, 152, 224, 149, 104, 25, 178, 252, 182,
    202, 182, 141, 197, 4, 81, 181, 242, 145, 42, 39, 227, 156, 198, 225, 193, 219, 93, 122, 175,
    249, 0, 175, 143, 70, 239, 46, 246, 163, 53, 163, 109, 168, 135, 2, 235, 25, 92, 20, 145, 138,
    77, 69, 166, 78, 176, 173, 212, 166, 113, 94, 161, 41, 50, 239, 49, 111, 164, 70, 60, 2, 37,
    171, 75, 136, 156, 11, 56, 42, 146, 138, 229, 73, 146, 77, 61, 98, 196, 135, 106, 63, 197, 195,
    86, 96, 203, 113, 101, 170, 247, 181, 113, 80, 250, 108, 7, 255, 237, 129, 226, 79, 107, 112,
    166, 103, 241, 24, 223, 239, 120, 198, 58, 60, 82, 128, 3, 184, 66, 143, 224, 145, 224, 81,
    206, 163, 45, 63, 90, 168, 114, 59, 33, 159, 95, 28, 139, 123, 98, 125, 196, 15, 70, 194, 253,
    54, 14, 109, 226, 71, 17, 161, 93, 186, 87, 244, 138, 20, 52, 123, 251, 26, 36, 17, 46, 52,
    231, 232, 76, 31, 221, 84, 37, 216, 165, 212, 106, 197, 242, 98, 43, 39, 175, 254, 145, 190,
    84, 118, 222, 187, 136, 120, 163, 236, 249,
];

/// One 0-255 lookup-table stream.
#[derive(Clone, Copy, Debug, Default)]
pub struct RandomStream {
    index: u8,
}

impl RandomStream {
    pub const fn new() -> Self {
        Self { index: 0 }
    }

    /// Savegame snapshot: the raw stream position.
    pub fn state(&self) -> u8 {
        self.index
    }

    /// Savegame restore.
    pub fn set_state(&mut self, index: u8) {
        self.index = index;
    }

    pub fn clear(&mut self) {
        self.index = 0;
    }

    /// Returns numbers from 0 to 255, pre-incrementing like the original
    /// `rndindex = (rndindex + 1) & 0xff` (so the first call reads index 1).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u8 {
        self.index = self.index.wrapping_add(1);
        RND_TABLE[self.index as usize]
    }
}

/// The engine-wide RNG with separate play and map streams.
#[derive(Clone, Copy, Debug, Default)]
pub struct Random {
    pub m: RandomStream,
    pub p: RandomStream,
}

impl Random {
    pub const fn new() -> Self {
        Self {
            m: RandomStream::new(),
            p: RandomStream::new(),
        }
    }

    /// Resets both streams; used to fix randomness for demo playback.
    pub fn clear(&mut self) {
        self.m.clear();
        self.p.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_values_match_c() {
        let mut rng = RandomStream::new();
        // rndtable[1..4] = 8, 109, 220
        assert_eq!(rng.next(), 8);
        assert_eq!(rng.next(), 109);
        assert_eq!(rng.next(), 220);
    }

    #[test]
    fn wraps_around_256() {
        let mut rng = RandomStream::new();
        let mut last = 0u8;
        for _ in 0..256 {
            last = rng.next();
        }
        assert_eq!(rng.next(), 8);
        let _ = last;
    }

    #[test]
    fn streams_are_independent() {
        let mut a = RandomStream::new();
        let mut b = RandomStream::new();
        a.next();
        assert_eq!(a.next(), 109);
        assert_eq!(b.next(), 8);
    }

    #[test]
    fn clear_resets_stream() {
        let mut rng = Random::new();
        rng.p.next();
        rng.p.next();
        rng.m.next();
        rng.clear();
        assert_eq!(rng.p.next(), 8);
        assert_eq!(rng.m.next(), 8);
    }
}
