//! Weapon table module, port of `d_items.c`.
//!
//! The table itself lives with the rest of the static data in
//! [`crate::info::WEAPONINFO`]; this re-export keeps the C module layout so
//! callers can mirror the original include structure.

pub use crate::info::{WeaponInfo, WEAPONINFO};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doomdef::WeaponType;

    #[test]
    fn table_has_all_weapons() {
        assert_eq!(WEAPONINFO.len(), crate::doomdef::NUM_WEAPONS);
        // fist needs no ammo, the pistol takes clips.
        assert_eq!(WEAPONINFO[WeaponType::Fist as usize].ammo, None);
        assert_eq!(
            WEAPONINFO[WeaponType::Pistol as usize].ammo,
            Some(crate::doomdef::AmmoType::Clip)
        );
    }
}
