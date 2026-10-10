//! Fixed point arithmetic, 32-bit as 16.16.
//!
//! Port of `m_fixed.c`/`m_fixed.h`. Pure math, no libc.

pub const FRAC_BITS: u32 = 16;
pub const FRAC_UNIT: i32 = 1 << FRAC_BITS;

pub type Fixed = i32;

/// `(a * b) >> 16`, computed in 64-bit to avoid overflow.
pub fn fixed_mul(a: Fixed, b: Fixed) -> Fixed {
    ((i64::from(a) * i64::from(b)) >> FRAC_BITS) as Fixed
}

/// 16.16 division with saturation, faithful to the original C.
pub fn fixed_div(a: Fixed, b: Fixed) -> Fixed {
    // `wrapping_abs` reproduces C `abs()` semantics on INT_MIN; the shift of
    // a negative value is arithmetic, exactly like the C compiler generates.
    let abs_a = a.wrapping_abs() >> 14;
    let abs_b = b.wrapping_abs();
    if abs_a >= abs_b {
        if (a ^ b) < 0 {
            i32::MIN
        } else {
            i32::MAX
        }
    } else {
        ((i64::from(a) << 16) / i64::from(b)) as Fixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frac_constants() {
        assert_eq!(FRAC_BITS, 16);
        assert_eq!(FRAC_UNIT, 65_536);
    }

    #[test]
    fn mul_identity() {
        assert_eq!(fixed_mul(FRAC_UNIT, FRAC_UNIT), FRAC_UNIT);
        assert_eq!(fixed_mul(FRAC_UNIT, 0), 0);
    }

    #[test]
    fn mul_negative() {
        let half = FRAC_UNIT / 2;
        let neg_half = -half;
        assert_eq!(fixed_mul(half, neg_half), (FRAC_UNIT >> 2).wrapping_neg());
    }

    #[test]
    fn div_identity() {
        assert_eq!(fixed_div(3 * FRAC_UNIT, 3 * FRAC_UNIT), FRAC_UNIT);
    }

    #[test]
    fn div_small_denominator() {
        // 1.0 / (1/256) = 256.0, no saturation because |a|/16384 < |b|.
        assert_eq!(fixed_div(FRAC_UNIT, 256), 256 * FRAC_UNIT);
    }

    #[test]
    fn div_by_zero_saturates_positive() {
        assert_eq!(fixed_div(FRAC_UNIT, 0), i32::MAX);
    }

    #[test]
    fn div_small_denominator_saturates() {
        assert_eq!(fixed_div(FRAC_UNIT, 1), i32::MAX);
    }

    #[test]
    fn div_negative_saturates_negative() {
        assert_eq!(fixed_div(-FRAC_UNIT, 1), i32::MIN);
    }
}
