//! Sky rendering state, port of `r_sky.c`.
//!
//! The sky is a texture mapped like a wall wrapping 360 degrees around the
//! viewer. Only the map/iterator state lives here; the actual sky columns are
//! drawn by the plane renderer (`r_plane`).

use crate::fixed::{Fixed, FRAC_UNIT};

/// Sky flat number (`skyflatnum`), set by the game setup.
pub const DEFAULT_SKYFLATNUM: i32 = 0;

/// `R_InitSkyMap`: called whenever the view size changes.
///
/// The concrete `skyflatnum`/`skytexture` are resolved during game startup;
/// this only establishes the vertical texture origin.
#[inline]
pub fn init_sky_map() -> Fixed {
    100 * FRAC_UNIT
}
