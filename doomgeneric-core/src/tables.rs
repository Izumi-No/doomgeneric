//! Trigonometry / lookup tables.
//!
//! The arrays (`finesine`, `finetangent`, `tantoangle`, `gammatable`) live in
//! [`tables_data`] and are the exact historical Doom values, so the engine is
//! bit-for-bit identical to the original C source and vanilla demos.

#[path = "tables_data.rs"]
mod tables_data;
pub use tables_data::{FINESINE, FINETANGENT, GAMMATABLE, TANTOANGLE};

pub const FINEANGLES: usize = 8192;
pub const FINE_MASK: usize = FINEANGLES - 1;
pub const ANG_TO_FINE_SHIFT: u32 = 19;
pub const SLOPERANGE: u32 = 2048;
pub const SLOPEBITS: u32 = 11;
pub const DBITS: u32 = 16 - SLOPEBITS;

/// Amount to shift a BAM angle by to index `finesine` (0x100000000 -> 0x2000).
pub const ANGLETOFINESHIFT: u32 = ANG_TO_FINE_SHIFT;

// Binary angle measurement (BAM) constants, from `tables.h`.
pub const ANG45: u32 = 0x2000_0000;
pub const ANG90: u32 = 0x4000_0000;
pub const ANG180: u32 = 0x8000_0000;
pub const ANG270: u32 = 0xc000_0000;
pub const ANG_MAX: u32 = 0xffff_ffff;
pub const ANG1: u32 = ANG45 / 45;
pub const ANG60: u32 = ANG180 / 3;

/// `SlopeDiv` from `tables.c`: `num/den` scaled to `SLOPERANGE`, saturated.
#[inline]
pub fn slope_div(num: u32, den: u32) -> u32 {
    if den < 512 {
        SLOPERANGE
    } else {
        let ans = (num << 3) / (den >> 8);
        if ans <= SLOPERANGE {
            ans
        } else {
            SLOPERANGE
        }
    }
}

#[inline]
pub fn fine_sine(angle: u32) -> i32 {
    FINESINE[(angle >> ANGLETOFINESHIFT) as usize & FINE_MASK]
}

#[inline]
pub fn fine_cosine(angle: u32) -> i32 {
    // Doom's finecosine is finesine shifted by a quarter turn (PI/2).
    FINESINE[((angle >> ANGLETOFINESHIFT) as usize & FINE_MASK) + FINEANGLES / 4]
}

#[inline]
pub fn fine_tangent(i: usize) -> i32 {
    FINETANGENT[i & (FINEANGLES / 2 - 1)]
}

/// Index into the raw `finesine` array, as `finesine[angle]` is used in the C
/// renderer after `>> ANGLETOFINESHIFT` (masked like the original).
#[inline]
pub fn finesine_index(i: u32) -> i32 {
    FINESINE[(i & FINE_MASK as u32) as usize]
}

/// Index into `finecosine` (`finesine` shifted by a quarter turn).
#[inline]
pub fn finecosine_index(i: u32) -> i32 {
    FINESINE[((i & FINE_MASK as u32) as usize) + FINEANGLES / 4]
}

/// Index into the raw `finetangent` array.
#[inline]
pub fn finetangent_index(i: u32) -> i32 {
    FINETANGENT[(i & (FINEANGLES as u32 / 2 - 1)) as usize]
}

#[test]
fn sine_matches_reference() {
    let f = |ang: u32| FINESINE[(ang >> ANGLETOFINESHIFT) as usize & FINE_MASK];
    assert_eq!(f(0), 25);
    assert_eq!(f(0x4000_0000), 65535);
    assert_eq!(f(0x8000_0000), -25);
    assert_eq!(f(0xC000_0000), -65535);
}

#[test]
fn cosine_matches_reference() {
    assert_eq!(fine_cosine(0), 65535); // cos(0) = 1
    assert_eq!(fine_cosine(0x4000_0000), -25); // cos(90deg) = 0
    assert_eq!(fine_cosine(0x8000_0000), -65535); // cos(180deg) = -1
}

#[test]
fn tantoangle_quarter() {
    assert_eq!(TANTOANGLE[2048], 0x2000_0000);
}

#[test]
fn tables_match_the_original_c_source() {
    // Spot checks across every table against the historical `tables.c`; these
    // values are what vanilla demos depend on for lockstep playback.
    assert_eq!(FINETANGENT[0], -170910304);
    assert_eq!(FINETANGENT[1], -56965752);
    assert_eq!(FINETANGENT[4095], 170910304);
    assert_eq!(FINESINE[2048], 65535);
    assert_eq!(TANTOANGLE[1], 333772);
    assert_eq!(GAMMATABLE[0][0], 1);
    assert_eq!(GAMMATABLE[0][1], 2);
    assert_eq!(GAMMATABLE[4][255], 255);
}
