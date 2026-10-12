//! Rendering main loop and setup, port of `r_main.c`.
//!
//! Holds the global view/render state (originally loose globals), the
//! precomputed texture/light mapping tables and the geometry helpers used by
//! the BSP walker and the segment/plane/sprite renderers.

use crate::doomdef::NF_SUBSECTOR;
use crate::fixed::{fixed_div, fixed_mul, Fixed, FRAC_BITS, FRAC_UNIT};
use crate::level::{Level, Node, Vertex};
use crate::r_draw::{Draw, MAXHEIGHT, MAXWIDTH, SCREENHEIGHT, SCREENWIDTH};
use crate::tables::{
    fine_cosine, fine_sine, fine_tangent, ANG180, ANG270, ANG90, ANGLETOFINESHIFT, FINEANGLES,
    FINETANGENT,
};

/// Fineangles in the SCREENWIDTH wide window.
pub const FIELDOFVIEW: u32 = 2048;

// Lighting constants (`r_main.h`).
pub const LIGHTLEVELS: usize = 16;
pub const LIGHTSEGSHIFT: u32 = 4;
pub const MAXLIGHTSCALE: usize = 48;
pub const LIGHTSCALESHIFT: u32 = 12;
pub const MAXLIGHTZ: usize = 128;
pub const LIGHTZSHIFT: u32 = 20;
pub const NUMCOLORMAPS: usize = 32;
const DISTMAP: i32 = 2;

/// Grey-scale light lookup stored as a byte offset into the COLORMAP lump.
pub type LightOffset = usize;

/// All refresh/render state.
pub struct Render {
    pub draw: Draw,

    pub viewangleoffset: u32,
    pub validcount: i32,
    pub fixedcolormap: i32,
    pub centerx: i32,
    pub centerxfrac: Fixed,
    pub centeryfrac: Fixed,
    pub projection: Fixed,
    pub framecount: i32,
    pub sscount: i32,
    pub linecount: i32,
    pub loopcount: i32,

    pub viewx: Fixed,
    pub viewy: Fixed,
    pub viewz: Fixed,
    pub viewangle: u32,
    pub viewcos: Fixed,
    pub viewsin: Fixed,

    pub detailshift: i32,
    pub clipangle: u32,

    pub viewangletox: [i32; FINEANGLES / 2],
    pub xtoviewangle: [u32; SCREENWIDTH + 1],

    pub scalelight: [[LightOffset; MAXLIGHTSCALE]; LIGHTLEVELS],
    pub scalelightfixed: [LightOffset; MAXLIGHTSCALE],
    pub zlight: [[LightOffset; MAXLIGHTZ]; LIGHTLEVELS],
    pub extralight: i32,

    pub setsizeneeded: bool,
    pub setblocks: i32,
    pub setdetail: i32,
    pub screenheightarray: [i32; MAXWIDTH],
    /// Constant `-1` array used as a "no clip" sprite bottom bound.
    pub negonearray: [i32; MAXWIDTH],
    pub yslope: [Fixed; MAXHEIGHT],
    pub distscale: [Fixed; MAXWIDTH],

    /// Player (weapon) sprite scales, set by `R_ExecuteSetViewSize`.
    pub pspritescale: Fixed,
    pub pspriteiscale: Fixed,

    /// Per-segment values set while clipping walls (`r_bsp`/`r_segs`).
    pub rw_distance: Fixed,
    pub rw_normalangle: u32,
    pub rw_angle1: u32,
}

impl Default for Render {
    fn default() -> Self {
        Self::new()
    }
}

impl Render {
    pub fn new() -> Self {
        let mut render = Render {
            draw: Draw::new(),
            viewangleoffset: 0,
            validcount: 1,
            fixedcolormap: 0,
            centerx: 0,
            centerxfrac: 0,
            centeryfrac: 0,
            projection: 0,
            framecount: 0,
            sscount: 0,
            linecount: 0,
            loopcount: 0,
            viewx: 0,
            viewy: 0,
            viewz: 0,
            viewangle: 0,
            viewcos: 0,
            viewsin: 0,
            detailshift: 0,
            clipangle: 0,
            viewangletox: [0; FINEANGLES / 2],
            xtoviewangle: [0; SCREENWIDTH + 1],
            scalelight: [[0; MAXLIGHTSCALE]; LIGHTLEVELS],
            scalelightfixed: [0; MAXLIGHTSCALE],
            zlight: [[0; MAXLIGHTZ]; LIGHTLEVELS],
            extralight: 0,
            setsizeneeded: false,
            setblocks: 0,
            setdetail: 0,
            screenheightarray: [0; MAXWIDTH],
            negonearray: [-1; MAXWIDTH],
            yslope: [0; MAXHEIGHT],
            distscale: [0; MAXWIDTH],
            pspritescale: 0,
            pspriteiscale: 0,
            rw_distance: 0,
            rw_normalangle: 0,
            rw_angle1: 0,
        };

        // `R_InitLightTables`: independent of view size, so initialize here.
        render.init_light_tables();
        render
    }

    pub fn viewwidth(&self) -> i32 {
        self.draw.viewwidth
    }

    pub fn viewheight(&self) -> i32 {
        self.draw.viewheight
    }

    /// `R_SetViewSize`: defer the change to the next refresh.
    pub fn set_view_size(&mut self, blocks: i32, detail: i32) {
        self.setsizeneeded = true;
        self.setblocks = blocks;
        self.setdetail = detail;
    }

    /// `R_ExecuteSetViewSize`.
    pub fn execute_set_view_size(&mut self) {
        self.setsizeneeded = false;

        let (scaledviewwidth, viewheight) = if self.setblocks == 11 {
            (SCREENWIDTH as i32, SCREENHEIGHT as i32)
        } else {
            (self.setblocks * 32, (self.setblocks * 168 / 10) & !7)
        };

        self.detailshift = self.setdetail;
        let viewwidth = scaledviewwidth >> self.detailshift;
        self.draw.viewwidth = viewwidth;
        self.draw.viewheight = viewheight;

        self.draw.centery = viewheight / 2;
        self.centerx = viewwidth / 2;
        self.centerxfrac = self.centerx << FRAC_BITS;
        self.centeryfrac = self.draw.centery << FRAC_BITS;
        self.projection = self.centerxfrac;

        self.draw.init_buffer(scaledviewwidth, viewheight);
        self.init_texture_mapping();

        // psprite scales
        self.pspritescale = FRAC_UNIT * viewwidth / SCREENWIDTH as i32;
        self.pspriteiscale = FRAC_UNIT * SCREENWIDTH as i32 / viewwidth;

        // thing clipping
        for i in 0..viewwidth as usize {
            self.screenheightarray[i] = viewheight;
        }

        // planes
        for i in 0..viewheight {
            let mut dy = ((i - viewheight / 2) << FRAC_BITS) + FRAC_UNIT / 2;
            dy = dy.abs();
            self.yslope[i as usize] =
                fixed_div((viewwidth << self.detailshift) / 2 * FRAC_UNIT, dy);
        }

        for i in 0..viewwidth {
            let cosadj = fine_cosine(self.xtoviewangle[i as usize]).abs();
            self.distscale[i as usize] = fixed_div(FRAC_UNIT, cosadj);
        }

        // Calculate the light levels to use for each level / scale combination.
        for i in 0..LIGHTLEVELS {
            let startmap =
                ((LIGHTLEVELS - 1 - i) * 2) as i32 * NUMCOLORMAPS as i32 / LIGHTLEVELS as i32;
            for j in 0..MAXLIGHTSCALE {
                let level = startmap
                    - (j as i32 * SCREENWIDTH as i32) / (viewwidth << self.detailshift) / DISTMAP;

                let level = level.clamp(0, NUMCOLORMAPS as i32 - 1);
                self.scalelight[i][j] = level as usize * 256;
            }
        }
    }

    /// `R_InitTextureMapping`.
    pub fn init_texture_mapping(&mut self) {
        let viewwidth = self.draw.viewwidth;
        let focallength = fixed_div(
            self.centerxfrac,
            fine_tangent(FINEANGLES / 4 + FIELDOFVIEW as usize / 2),
        );

        for (i, ft) in FINETANGENT.iter().take(FINEANGLES / 2).enumerate() {
            let t = if *ft > FRAC_UNIT * 2 {
                -1
            } else if *ft < -FRAC_UNIT * 2 {
                viewwidth + 1
            } else {
                let t = fixed_mul(*ft, focallength);
                let t = (self.centerxfrac - t + FRAC_UNIT - 1) >> FRAC_BITS;
                t.clamp(-1, viewwidth + 1)
            };
            self.viewangletox[i] = t;
        }

        for x in 0..=viewwidth {
            let mut i = 0usize;
            while i < FINEANGLES / 2 && self.viewangletox[i] > x {
                i += 1;
            }
            self.xtoviewangle[x as usize] = ((i as u32) << ANGLETOFINESHIFT).wrapping_sub(ANG90);
        }

        for i in 0..FINEANGLES / 2 {
            if self.viewangletox[i] == -1 {
                self.viewangletox[i] = 0;
            } else if self.viewangletox[i] == viewwidth + 1 {
                self.viewangletox[i] = viewwidth;
            }
        }

        self.clipangle = self.xtoviewangle[0];
    }

    /// `R_InitLightTables`: distance-based (z) light lookup.
    pub fn init_light_tables(&mut self) {
        for i in 0..LIGHTLEVELS {
            let startmap =
                ((LIGHTLEVELS - 1 - i) * 2) as i32 * NUMCOLORMAPS as i32 / LIGHTLEVELS as i32;
            for j in 0..MAXLIGHTZ {
                let mut scale = fixed_div(
                    SCREENWIDTH as i32 / 2 * FRAC_UNIT,
                    (j as i32 + 1) << LIGHTZSHIFT,
                );
                scale >>= LIGHTSCALESHIFT;
                let mut level = startmap - scale / DISTMAP;

                if level < 0 {
                    level = 0;
                }
                if level >= NUMCOLORMAPS as i32 {
                    level = NUMCOLORMAPS as i32 - 1;
                }

                self.zlight[i][j] = level as usize * 256;
            }
        }
    }

    /// `R_SetupFrame` (without the player mobj: the caller passes the values).
    pub fn setup_frame(
        &mut self,
        x: Fixed,
        y: Fixed,
        z: Fixed,
        angle: u32,
        extralight: i32,
        fixedcolormap: i32,
    ) {
        self.viewx = x;
        self.viewy = y;
        self.viewangle = angle.wrapping_add(self.viewangleoffset);
        self.extralight = extralight;
        self.viewz = z;

        self.viewsin = fine_sine(self.viewangle);
        self.viewcos = fine_cosine(self.viewangle);

        self.sscount = 0;

        if fixedcolormap != 0 {
            self.fixedcolormap = fixedcolormap * 256;
            for i in 0..MAXLIGHTSCALE {
                self.scalelightfixed[i] = self.fixedcolormap as usize;
            }
        } else {
            self.fixedcolormap = 0;
        }

        self.framecount += 1;
        self.validcount += 1;
    }

    /// `R_PointToAngle` relative to the current view origin.
    pub fn point_to_angle(&self, x: Fixed, y: Fixed) -> u32 {
        point_to_angle_from(self.viewx, self.viewy, x, y)
    }

    /// `R_PointToAngle2` (sets the view origin, like the original).
    pub fn point_to_angle2(&mut self, x1: Fixed, y1: Fixed, x2: Fixed, y2: Fixed) -> u32 {
        self.viewx = x1;
        self.viewy = y1;
        self.point_to_angle(x2, y2)
    }

    /// `R_PointToDist`.
    pub fn point_to_dist(&self, x: Fixed, y: Fixed) -> Fixed {
        let mut dx = (x - self.viewx).abs();
        let mut dy = (y - self.viewy).abs();

        if dy > dx {
            core::mem::swap(&mut dx, &mut dy);
        }

        let frac = if dx != 0 { fixed_div(dy, dx) } else { 0 };

        let angle = (tantoangle_at(frac >> DBITS).wrapping_add(ANG90)) >> ANGLETOFINESHIFT;

        // `angle` is already an index into finesine (the original divides by
        // ANGLETOFINESHIFT before the lookup).
        fixed_div(dx, crate::tables::FINESINE[angle as usize])
    }

    /// `R_ScaleFromGlobalAngle`. Requires `rw_distance`/`rw_normalangle`.
    pub fn scale_from_global_angle(&self, visangle: u32) -> Fixed {
        let anglea = ANG90.wrapping_add(visangle.wrapping_sub(self.viewangle));
        let angleb = ANG90.wrapping_add(visangle.wrapping_sub(self.rw_normalangle));

        let sinea = fine_sine(anglea);
        let sineb = fine_sine(angleb);
        let num = fixed_mul(self.projection, sineb) << self.detailshift;
        let den = fixed_mul(self.rw_distance, sinea);

        if den > num >> 16 {
            fixed_div(num, den).clamp(256, 64 * FRAC_UNIT)
        } else {
            64 * FRAC_UNIT
        }
    }

    /// `R_PointInSubsector`.
    pub fn point_in_subsector(&self, level: &Level, x: Fixed, y: Fixed) -> usize {
        if level.nodes.is_empty() {
            return 0;
        }

        let mut nodenum = level.nodes.len() - 1;
        while nodenum & NF_SUBSECTOR as usize == 0 {
            let node = &level.nodes[nodenum];
            let side = point_on_side(x, y, node);
            nodenum = node.children[side] as usize;
        }

        nodenum & !(NF_SUBSECTOR as usize)
    }
}

const DBITS: u32 = 16 - 11;

/// `tantoangle[frac >> DBITS]` with saturation to the table size.
#[inline]
fn tantoangle_at(i: Fixed) -> u32 {
    let idx = if i < 0 {
        0
    } else if i as usize >= crate::tables::TANTOANGLE.len() {
        crate::tables::TANTOANGLE.len() - 1
    } else {
        i as usize
    };
    crate::tables::TANTOANGLE[idx]
}

/// `R_PointToAngle` with an explicit origin (pure form).
pub fn point_to_angle_from(vx: Fixed, vy: Fixed, x: Fixed, y: Fixed) -> u32 {
    let mut x = x.wrapping_sub(vx);
    let mut y = y.wrapping_sub(vy);

    if x == 0 && y == 0 {
        return 0;
    }

    if x >= 0 {
        if y >= 0 {
            if x > y {
                crate::tables::TANTOANGLE[crate::tables::slope_div(y as u32, x as u32) as usize]
            } else {
                ANG90.wrapping_sub(1).wrapping_sub(
                    crate::tables::TANTOANGLE
                        [crate::tables::slope_div(x as u32, y as u32) as usize],
                )
            }
        } else {
            y = -y;
            if x > y {
                crate::tables::TANTOANGLE[crate::tables::slope_div(y as u32, x as u32) as usize]
                    .wrapping_neg()
            } else {
                ANG270.wrapping_add(
                    crate::tables::TANTOANGLE
                        [crate::tables::slope_div(x as u32, y as u32) as usize],
                )
            }
        }
    } else {
        x = -x;
        if y >= 0 {
            if x > y {
                ANG180.wrapping_sub(1).wrapping_sub(
                    crate::tables::TANTOANGLE
                        [crate::tables::slope_div(y as u32, x as u32) as usize],
                )
            } else {
                ANG90.wrapping_add(
                    crate::tables::TANTOANGLE
                        [crate::tables::slope_div(x as u32, y as u32) as usize],
                )
            }
        } else {
            y = -y;
            if x > y {
                ANG180.wrapping_add(
                    crate::tables::TANTOANGLE
                        [crate::tables::slope_div(y as u32, x as u32) as usize],
                )
            } else {
                ANG270.wrapping_sub(1).wrapping_sub(
                    crate::tables::TANTOANGLE
                        [crate::tables::slope_div(x as u32, y as u32) as usize],
                )
            }
        }
    }
}

/// `R_PointOnSide`.
pub fn point_on_side(x: Fixed, y: Fixed, node: &Node) -> usize {
    if node.dx == 0 {
        return if x <= node.x {
            (node.dy > 0) as usize
        } else {
            (node.dy < 0) as usize
        };
    }
    if node.dy == 0 {
        return if y <= node.y {
            (node.dx < 0) as usize
        } else {
            (node.dx > 0) as usize
        };
    }

    let dx = x.wrapping_sub(node.x);
    let dy = y.wrapping_sub(node.y);

    if ((node.dy ^ node.dx ^ dx ^ dy) as u32) & 0x8000_0000 != 0 {
        return if ((node.dy ^ dx) as u32) & 0x8000_0000 != 0 {
            1
        } else {
            0
        };
    }

    let left = fixed_mul(node.dy >> FRAC_BITS, dx);
    let right = fixed_mul(dy, node.dx >> FRAC_BITS);

    if right < left {
        0
    } else {
        1
    }
}

/// `R_PointOnSegSide` using explicit endpoints.
pub fn point_on_seg_side(x: Fixed, y: Fixed, v1: &Vertex, v2: &Vertex) -> usize {
    let lx = v1.x;
    let ly = v1.y;
    let ldx = v2.x - lx;
    let ldy = v2.y - ly;

    if ldx == 0 {
        return if x <= lx {
            (ldy > 0) as usize
        } else {
            (ldy < 0) as usize
        };
    }
    if ldy == 0 {
        return if y <= ly {
            (ldx < 0) as usize
        } else {
            (ldx > 0) as usize
        };
    }

    let dx = x.wrapping_sub(lx);
    let dy = y.wrapping_sub(ly);

    if ((ldy ^ ldx ^ dx ^ dy) as u32) & 0x8000_0000 != 0 {
        return if ((ldy ^ dx) as u32) & 0x8000_0000 != 0 {
            1
        } else {
            0
        };
    }

    let left = fixed_mul(ldy >> FRAC_BITS, dx);
    let right = fixed_mul(dy, ldx >> FRAC_BITS);

    if right < left {
        0
    } else {
        1
    }
}

/// `R_AddPointToBox`.
pub fn add_point_to_box(x: i32, y: i32, bbox: &mut [i32; 4]) {
    use crate::level::{BBOX_BOTTOM, BBOX_LEFT, BBOX_RIGHT, BBOX_TOP};
    if x < bbox[BBOX_LEFT] {
        bbox[BBOX_LEFT] = x;
    }
    if x > bbox[BBOX_RIGHT] {
        bbox[BBOX_RIGHT] = x;
    }
    if y < bbox[BBOX_BOTTOM] {
        bbox[BBOX_BOTTOM] = y;
    }
    if y > bbox[BBOX_TOP] {
        bbox[BBOX_TOP] = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_on_side_horizontal() {
        let node = Node {
            x: 0,
            y: 0,
            dx: FRAC_UNIT,
            dy: 0,
            ..Default::default()
        };
        // dy == 0: side depends on the y sign; dx > 0 flips the mapping.
        assert_eq!(point_on_side(10, -10, &node), 0);
        assert_eq!(point_on_side(10, 10, &node), 1);
    }

    #[test]
    fn angle_quadrants() {
        assert_eq!(point_to_angle_from(0, 0, FRAC_UNIT, 0), 0);
        assert_eq!(point_to_angle_from(0, 0, 0, FRAC_UNIT), ANG90 - 1);
        assert_eq!(point_to_angle_from(0, 0, -FRAC_UNIT, 0), ANG180 - 1);
        assert_eq!(point_to_angle_from(0, 0, 0, -FRAC_UNIT), ANG270);
    }

    #[test]
    fn distance_axis_aligned() {
        let mut r = Render::new();
        r.viewx = 0;
        r.viewy = 0;
        for d in [r.point_to_dist(FRAC_UNIT, 0), r.point_to_dist(0, FRAC_UNIT)] {
            assert!((d - FRAC_UNIT).abs() <= 2);
        }
    }

    #[test]
    fn execute_view_size_fullscreen() {
        let mut r = Render::new();
        r.set_view_size(11, 0);
        r.execute_set_view_size();
        assert_eq!(r.viewwidth(), SCREENWIDTH as i32);
        assert_eq!(r.viewheight(), SCREENHEIGHT as i32);
        assert_eq!(r.centerx, SCREENWIDTH as i32 / 2);
        assert_eq!(r.draw.centery, SCREENHEIGHT as i32 / 2);
        assert_eq!(r.draw.viewwindowx, 0);
        assert_eq!(r.scalelight[LIGHTLEVELS - 1][0], 0);
    }

    #[test]
    fn init_light_tables_bounds() {
        let mut r = Render::new();
        r.init_light_tables();
        for i in 0..LIGHTLEVELS {
            for j in 0..MAXLIGHTZ {
                assert!(r.zlight[i][j] < NUMCOLORMAPS * 256);
            }
        }
    }

    #[test]
    fn subsector_single_node() {
        let mut level = Level::default();
        level.nodes.push(Node {
            x: 0,
            y: 0,
            dx: FRAC_UNIT,
            dy: 0,
            children: [NF_SUBSECTOR, NF_SUBSECTOR | 1],
            ..Default::default()
        });
        level.subsectors.push(Default::default());
        level.subsectors.push(Default::default());

        let r = Render::new();
        // dy == 0: the side (and thus child) depends on the y sign.
        assert_eq!(r.point_in_subsector(&level, -FRAC_UNIT, 0), 0);
        assert_eq!(r.point_in_subsector(&level, FRAC_UNIT, FRAC_UNIT), 1);
    }
}
