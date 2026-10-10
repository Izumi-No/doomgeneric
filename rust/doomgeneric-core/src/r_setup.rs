//! Runtime texture/flat number resolution, the part of `P_SetupLevel` that
//! turns the WAD's name-based sidedef/sector references into the integer
//! numbers used by the renderer.
//!
//! In the original C the level structures are mutated in place. Keeping names
//! intact in [`Level`](crate::level::Level) and exposing the resolved numbers
//! separately avoids coupling the geometry loader to the texture manager.

use alloc::vec::Vec;

use crate::level::Level;
use crate::r_data::{RDataError, RenderData};
use crate::wad::{LumpName, WadManager};

/// `SKYFLATNAME`.
pub const SKYFLATNAME: &[u8] = b"F_SKY1";

/// `skytexture` name used by the retail/registered sky.
pub const SKYTEXTURE_NAME: &[u8] = b"SKY1";

/// Integer texture/flat numbers for every sidedef and sector.
pub struct TextureNumbers {
    /// Top texture number per sidedef.
    pub side_top: Vec<i32>,
    /// Bottom texture number per sidedef.
    pub side_bottom: Vec<i32>,
    /// Middle texture number per sidedef.
    pub side_mid: Vec<i32>,
    /// Floor flat number per sector.
    pub sector_floor: Vec<i32>,
    /// Ceiling flat number per sector.
    pub sector_ceiling: Vec<i32>,

    /// `skyflatnum`, the special flat number that selects the sky.
    pub skyflatnum: i32,
    /// `skytexture`, the texture number of the sky wall.
    pub skytexture: i32,
}

impl TextureNumbers {
    /// Resolve every side/sector reference against the texture directory.
    pub fn resolve(
        level: &Level,
        data: &RenderData,
        manager: &WadManager,
    ) -> Result<Self, RDataError> {
        let mut side_top = Vec::with_capacity(level.sides.len());
        let mut side_bottom = Vec::with_capacity(level.sides.len());
        let mut side_mid = Vec::with_capacity(level.sides.len());
        for s in &level.sides {
            side_top.push(data.texture_num_for_name(&s.toptexture)?);
            side_bottom.push(data.texture_num_for_name(&s.bottomtexture)?);
            side_mid.push(data.texture_num_for_name(&s.midtexture)?);
        }

        let mut sector_floor = Vec::with_capacity(level.sectors.len());
        let mut sector_ceiling = Vec::with_capacity(level.sectors.len());
        for sec in &level.sectors {
            sector_floor.push(data.flat_num_for_name(manager, &sec.floorpic)?);
            sector_ceiling.push(data.flat_num_for_name(manager, &sec.ceilingpic)?);
        }

        let skyflatnum = data.flat_num_for_name(manager, &LumpName::from_bytes(SKYFLATNAME))?;
        let skytexture = data.check_texture_num_for_name(&LumpName::from_bytes(SKYTEXTURE_NAME));

        Ok(TextureNumbers {
            side_top,
            side_bottom,
            side_mid,
            sector_floor,
            sector_ceiling,
            skyflatnum,
            skytexture,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Sector, Side};

    #[test]
    fn constants_fit_a_lump_name() {
        assert!(SKYFLATNAME.len() <= 8);
        assert!(SKYTEXTURE_NAME.len() <= 8);
        // Names are space padded by `LumpName::from_bytes`.
        assert_eq!(LumpName::from_bytes(SKYFLATNAME).as_bytes(), b"F_SKY1  ");
    }

    // Resolution itself is covered by the WAD integration tests once a map is
    // loaded; here we only check the bookkeeping shape.
    #[test]
    fn default_level_has_no_sides() {
        let lvl = Level::default();
        assert!(lvl.sides.is_empty());
        let _ = (Sector::default(), Side::default());
    }
}
