//! Small string/number helpers, port of `m_misc.c`.
//!
//! Only what the core needs: filename case folding, tolerant number parsing
//! and bounded copies. The heavy file helpers of vanilla are host concerns
//! (they go through the `FileSystem` seam).

use alloc::string::String;

/// `M_ForceUppercase`: WAD lump names compare case-insensitively.
pub fn force_uppercase(s: &str) -> String {
    s.chars().map(|c| c.to_ascii_uppercase()).collect()
}

/// `M_StringToInt`: tolerant atoi (defaults to `default`).
pub fn string_to_int(s: &str, default: i32) -> i32 {
    let t = s.trim_start();
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let mut v: i32 = 0;
    let mut any = false;
    for c in digits.chars() {
        let Some(d) = c.to_digit(10) else {
            break;
        };
        v = v.saturating_mul(10).saturating_add(d as i32);
        any = true;
    }
    if !any {
        return default;
    }
    if neg {
        -v
    } else {
        v
    }
}

/// `M_StringCopy`: bounded copy (at most `n` bytes, cut on a char boundary).
pub fn string_copy(dst: &mut String, src: &str, n: usize) {
    dst.clear();
    for (i, c) in src.char_indices() {
        if i + c.len_utf8() >= n {
            break;
        }
        dst.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uppercase_folds_lump_names() {
        assert_eq!(force_uppercase("e1m1"), "E1M1");
    }

    #[test]
    fn atoi_is_tolerant() {
        assert_eq!(string_to_int("42", 0), 42);
        assert_eq!(string_to_int("-7", 0), -7);
        assert_eq!(string_to_int("12abc", 0), 12);
        assert_eq!(string_to_int("abc", 5), 5);
        assert_eq!(string_to_int("", 5), 5);
    }
}
