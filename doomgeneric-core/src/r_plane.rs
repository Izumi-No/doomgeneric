//! Floor/ceiling (visplane) rendering, port of `r_plane.c`.
//!
//! The original stores loose globals (`visplanes`, `floorclip`, span caches,
//! ...). They are grouped here in [`PlaneManager`] and passed explicitly to the
//! rest of the renderer. `R_DrawPlanes` needs the texture/flat data, so it
//! takes the [`RenderData`] and [`WadManager`] directly.

use alloc::vec;
use alloc::vec::Vec;

use crate::fixed::{fixed_div, fixed_mul, Fixed, FRAC_UNIT};
use crate::r_data::RenderData;
use crate::r_draw::{SCREENHEIGHT, SCREENWIDTH};
use crate::r_main::{LightOffset, Render, LIGHTLEVELS, LIGHTSEGSHIFT, LIGHTZSHIFT, MAXLIGHTZ};
use crate::tables::{finecosine_index, finesine_index, ANG90, ANGLETOFINESHIFT};
use crate::wad::WadManager;

/// Maximum number of distinct visplanes in a frame.
pub const MAXVISPLANES: usize = 128;
/// Opening list length (`SCREENWIDTH * 64`).
pub const MAXOPENINGS: usize = SCREENWIDTH * 64;

/// `visplane_t`: a horizontally-contiguous run of floor/ceiling spans sharing
/// the same height, flat and light level.
#[derive(Clone)]
pub struct Visplane {
    pub height: Fixed,
    pub picnum: i32,
    pub lightlevel: i32,
    pub minx: i32,
    pub maxx: i32,
    /// Top of the span at each screen column; `0xff` marks "not present".
    /// `top[SCREENWIDTH]` with the vanilla `pad1`/`pad2` bytes: valid indices
    /// run `-1..=SCREENWIDTH` through [`Visplane::top_at`] (index + 1).
    pub top: [u8; SCREENWIDTH + 2],
    /// Bottom of the span at each screen column.
    pub bottom: [u8; SCREENWIDTH + 2],
}

impl Default for Visplane {
    fn default() -> Self {
        Visplane {
            height: 0,
            picnum: 0,
            lightlevel: 0,
            minx: 0,
            maxx: 0,
            top: [0xff; SCREENWIDTH + 2],
            bottom: [0; SCREENWIDTH + 2],
        }
    }
}

/// All `r_plane.c` state.
pub struct PlaneManager {
    pub visplanes: Vec<Visplane>,
    pub lastvisplane: usize,

    /// Index into `visplanes`, or `-1` for "none" (`NULL`).
    pub floorplane: i32,
    pub ceilingplane: i32,

    /// Opening list used for sprite clipping (shared with `r_segs`).
    pub openings: [i16; MAXOPENINGS],
    pub lastopening: usize,

    /// Solid-pixel bounding ranges per column.
    pub floorclip: [i16; SCREENWIDTH],
    pub ceilingclip: [i16; SCREENWIDTH],

    /// Start column of the current plane span for each row.
    pub spanstart: [i32; SCREENHEIGHT],

    /// Texture mapping.
    pub planeheight: Fixed,
    pub basexscale: Fixed,
    pub baseyscale: Fixed,
    pub cachedheight: [Fixed; SCREENHEIGHT],
    pub cacheddistance: [Fixed; SCREENHEIGHT],
    pub cachedxstep: [Fixed; SCREENHEIGHT],
    pub cachedystep: [Fixed; SCREENHEIGHT],

    /// Sky state (`r_sky.c`).
    pub skyflatnum: i32,
    pub skytexture: i32,
    pub skytexturemid: Fixed,
}

impl Default for PlaneManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaneManager {
    pub fn new() -> Self {
        PlaneManager {
            visplanes: vec![Visplane::default(); MAXVISPLANES],
            lastvisplane: 0,
            floorplane: -1,
            ceilingplane: -1,
            openings: [0; MAXOPENINGS],
            lastopening: 0,
            floorclip: [0; SCREENWIDTH],
            ceilingclip: [0; SCREENWIDTH],
            spanstart: [0; SCREENHEIGHT],
            planeheight: 0,
            basexscale: 0,
            baseyscale: 0,
            cachedheight: [0; SCREENHEIGHT],
            cacheddistance: [0; SCREENHEIGHT],
            cachedxstep: [0; SCREENHEIGHT],
            cachedystep: [0; SCREENHEIGHT],
            skyflatnum: 0,
            skytexture: 0,
            skytexturemid: 100 * FRAC_UNIT,

        }
    }

    /// `R_InitSkyMap`.
    pub fn init_sky_map(&mut self) {
        self.skytexturemid = 100 * FRAC_UNIT;
    }

    /// `R_ClearPlanes`: at the beginning of a frame.
    pub fn clear_planes(&mut self, render: &Render) {
        let viewwidth = render.viewwidth();
        let viewheight = render.viewheight();

        for i in 0..viewwidth as usize {
            self.floorclip[i] = viewheight as i16;
            self.ceilingclip[i] = -1;
        }

        self.lastvisplane = 0;
        self.lastopening = 0;
        self.floorplane = -1;
        self.ceilingplane = -1;

        for c in self.cachedheight.iter_mut() {
            *c = 0;
        }

        // left to right mapping
        let angle = render.viewangle.wrapping_sub(ANG90) >> ANGLETOFINESHIFT;
        self.basexscale = fixed_div(finecosine_index(angle), render.centerxfrac);
        self.baseyscale = -fixed_div(finesine_index(angle), render.centerxfrac);
    }

    /// `R_FindPlane`: locate (or create) the visplane for these plane params.
    /// Returns an index into [`Self::visplanes`], or `-1` on overflow.
    pub fn find_plane(&mut self, mut height: Fixed, picnum: i32, mut lightlevel: i32) -> i32 {
        if picnum == self.skyflatnum {
            height = 0;
            lightlevel = 0;
        }

        let mut found = self.lastvisplane;
        for i in 0..self.lastvisplane {
            let p = &self.visplanes[i];
            if height == p.height && picnum == p.picnum && lightlevel == p.lightlevel {
                found = i;
                break;
            }
        }

        if found < self.lastvisplane {
            return found as i32;
        }

        if self.lastvisplane >= MAXVISPLANES {
            return -1;
        }

        let idx = self.lastvisplane;
        self.lastvisplane += 1;
        let p = &mut self.visplanes[idx];
        p.height = height;
        p.picnum = picnum;
        p.lightlevel = lightlevel;
        p.minx = SCREENWIDTH as i32;
        p.maxx = -1;
        p.top = [0xff; SCREENWIDTH + 2];
        idx as i32
    }

    /// `R_CheckPlane`: if the new range overlaps a used part of `pl`, make a
    /// new visplane; otherwise extend `pl`. Returns the plane index to use.
    pub fn check_plane(&mut self, pl: i32, start: i32, stop: i32) -> i32 {
        let pl = pl as usize;
        let (minx, maxx) = (self.visplanes[pl].minx, self.visplanes[pl].maxx);

        let (intrl, unionl) = if start < minx {
            (minx, start)
        } else {
            (start, minx)
        };
        let (intrh, unionh) = if stop > maxx {
            (maxx, stop)
        } else {
            (stop, maxx)
        };

        let mut x = intrl;
        while x <= intrh {
            if self.visplanes[pl].top[(x + 1) as usize] != 0xff {
                break;
            }
            x += 1;
        }

        if x > intrh {
            self.visplanes[pl].minx = unionl;
            self.visplanes[pl].maxx = unionh;
            return pl as i32;
        }

        // make a new visplane
        if self.lastvisplane >= MAXVISPLANES {
            return pl as i32;
        }
        let idx = self.lastvisplane;
        self.lastvisplane += 1;

        let (height, picnum, lightlevel) = {
            let old = &self.visplanes[pl];
            (old.height, old.picnum, old.lightlevel)
        };
        let np = &mut self.visplanes[idx];
        np.height = height;
        np.picnum = picnum;
        np.lightlevel = lightlevel;
        np.minx = start;
        np.maxx = stop;
        np.top = [0xff; SCREENWIDTH + 2];
        idx as i32
    }

    /// `R_MapPlane`: draw one horizontal span of a visplane.
    #[allow(clippy::too_many_arguments)]
    fn map_plane(
        &mut self,
        render: &mut Render,
        y: i32,
        x1: i32,
        x2: i32,
        zlight_row: &[LightOffset; MAXLIGHTZ],
        source: &[u8],
        colormaps: &[u8],
        screen: &mut [u8],
    ) {
        let distance;
        if self.planeheight != self.cachedheight[y as usize] {
            self.cachedheight[y as usize] = self.planeheight;
            distance = fixed_mul(self.planeheight, render.yslope[y as usize]);
            self.cacheddistance[y as usize] = distance;
            let xstep = fixed_mul(distance, self.basexscale);
            let ystep = fixed_mul(distance, self.baseyscale);
            self.cachedxstep[y as usize] = xstep;
            self.cachedystep[y as usize] = ystep;
            render.draw.ds_xstep = xstep;
            render.draw.ds_ystep = ystep;
        } else {
            distance = self.cacheddistance[y as usize];
            render.draw.ds_xstep = self.cachedxstep[y as usize];
            render.draw.ds_ystep = self.cachedystep[y as usize];
        }

        let length = fixed_mul(distance, render.distscale[x1 as usize]);
        let angle = render
            .viewangle
            .wrapping_add(render.xtoviewangle[x1 as usize])
            >> ANGLETOFINESHIFT;
        render.draw.ds_xfrac = render.viewx + fixed_mul(finecosine_index(angle), length);
        render.draw.ds_yfrac = -render.viewy - fixed_mul(finesine_index(angle), length);

                let colormap = if render.fixedcolormap != 0 {
            &colormaps[render.fixedcolormap as usize..]
        } else {
            let mut index = (distance >> LIGHTZSHIFT) as usize;
            if index >= MAXLIGHTZ {
                index = MAXLIGHTZ - 1;
            }
            &colormaps[zlight_row[index]..]
        };

        render.draw.ds_y = y;
        render.draw.ds_x1 = x1;
        render.draw.ds_x2 = x2;

        if render.detailshift == 0 {
            render.draw.draw_span(screen, colormap, source);
        } else {
            render.draw.draw_span_low(screen, colormap, source);
        }
    }

    /// `R_MakeSpans`: close and open plane spans as the column advances.
    #[allow(clippy::too_many_arguments)]
    fn make_spans(
        &mut self,
        render: &mut Render,
        x: i32,
        mut t1: i32,
        mut b1: i32,
        mut t2: i32,
        mut b2: i32,
        zlight_row: &[LightOffset; MAXLIGHTZ],
        source: &[u8],
        colormaps: &[u8],
        screen: &mut [u8],
    ) {
        while t1 < t2 && t1 <= b1 {
            let start = self.spanstart[t1 as usize];
            self.map_plane(
                render,
                t1,
                start,
                x - 1,
                zlight_row,
                source,
                colormaps,
                screen,
            );
            t1 += 1;
        }
        while b1 > b2 && b1 >= t1 {
            let start = self.spanstart[b1 as usize];
            self.map_plane(
                render,
                b1,
                start,
                x - 1,
                zlight_row,
                source,
                colormaps,
                screen,
            );
            b1 -= 1;
        }
        while t2 < t1 && t2 <= b2 {
            self.spanstart[t2 as usize] = x;
            t2 += 1;
        }
        while b2 > b1 && b2 >= t2 {
            self.spanstart[b2 as usize] = x;
            b2 -= 1;
        }
    }

    /// `R_DrawPlanes`: draw all visplanes at the end of a frame.
    pub fn draw_planes(
        &mut self,
        render: &mut Render,
        data: &mut RenderData,
        manager: &WadManager,
        screen: &mut [u8],
    ) {
        let colormaps = data.colormaps.clone();

        for i in 0..self.lastvisplane {
            let pl = self.visplanes[i].clone();
            if pl.minx > pl.maxx {
                continue;
            }

            // sky flat
            if pl.picnum == self.skyflatnum {
                let mut dc_iscale = render.pspriteiscale;
                if render.detailshift != 0 {
                    dc_iscale >>= render.detailshift;
                }
                render.draw.dc_iscale = dc_iscale;
                render.draw.dc_texturemid = self.skytexturemid;
                for x in pl.minx..=pl.maxx {
                    let yl = pl.top[(x + 1) as usize] as i32;
                    let yh = pl.bottom[(x + 1) as usize] as i32;
                    if yl <= yh {
                        let angle = render
                            .viewangle
                            .wrapping_add(render.xtoviewangle[x as usize])
                            >> ANGLETOFINESHIFT;
                        render.draw.dc_x = x;
                        render.draw.dc_yl = yl;
                        render.draw.dc_yh = yh;
                        let source =
                            match data.column(manager, self.skytexture as usize, angle as i32) {
                                Ok(s) => s,
                                Err(_) => continue,
                            };
                        if render.detailshift == 0 {
                            render.draw.draw_column(screen, &colormaps, source);
                        } else {
                            render.draw.draw_column_low(screen, &colormaps, source);
                        }
                    }
                }
                continue;
            }

            // regular flat
            self.planeheight = (pl.height - render.viewz).abs();

            let mut light = (pl.lightlevel >> LIGHTSEGSHIFT) + render.extralight;
            if light >= LIGHTLEVELS as i32 {
                light = LIGHTLEVELS as i32 - 1;
            }
            if light < 0 {
                light = 0;
            }
            let zlight_row = render.zlight[light as usize];

            let mut top = pl.top;
            let bottom = pl.bottom;
            let maxx = pl.maxx;
            // The +1/+2 offsets are the vanilla `pad1`/`pad2` aliasing.
            top[(maxx + 2) as usize] = 0xff;
            top[pl.minx as usize] = 0xff;
            let stop = maxx + 1;

            let source = match data.flat(manager, pl.picnum) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for x in pl.minx..=stop {
                self.make_spans(
                    render,
                    x,
                    top[x as usize] as i32,
                    bottom[x as usize] as i32,
                    top[(x + 1) as usize] as i32,
                    bottom[(x + 1) as usize] as i32,
                    &zlight_row,
                    source,
                    &colormaps,
                    screen,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_plane_dedups_and_sky_collapses() {
        let mut pm = PlaneManager::new();
        pm.skyflatnum = 10;

        let a = pm.find_plane(FRAC_UNIT, 5, 8);
        let b = pm.find_plane(FRAC_UNIT, 5, 8);
        assert_eq!(a, b);
        assert_eq!(pm.lastvisplane, 1);

        // different height -> new plane
        let c = pm.find_plane(2 * FRAC_UNIT, 5, 8);
        assert_ne!(a, c);
        assert_eq!(pm.lastvisplane, 2);

        // sky planes all map together regardless of height/light
        let s1 = pm.find_plane(100, 10, 3);
        let s2 = pm.find_plane(999, 10, 200);
        assert_eq!(s1, s2);
        assert_eq!(pm.visplanes[s1 as usize].height, 0);
        assert_eq!(pm.visplanes[s1 as usize].lightlevel, 0);
    }

    #[test]
    fn find_plane_initialises_top_sentinel() {
        let mut pm = PlaneManager::new();
        let i = pm.find_plane(0, 1, 0) as usize;
        assert_eq!(pm.visplanes[i].minx, SCREENWIDTH as i32);
        assert_eq!(pm.visplanes[i].maxx, -1);
        assert!(pm.visplanes[i].top.iter().all(|&v| v == 0xff));
    }

    #[test]
    fn check_plane_reuses_when_disjoint() {
        let mut pm = PlaneManager::new();
        let pl = pm.find_plane(0, 1, 0);

        // Mark columns 10..=20 as used in the plane.
        for x in 10..=20 {
            pm.visplanes[pl as usize].top[x + 1] = 5;
        }
        pm.visplanes[pl as usize].minx = 10;
        pm.visplanes[pl as usize].maxx = 20;

        // A range that does not touch used columns reuses the same plane.
        let same = pm.check_plane(pl, 30, 40);
        assert_eq!(same, pl);
        assert_eq!(pm.visplanes[same as usize].minx, 10);
        assert_eq!(pm.visplanes[same as usize].maxx, 40);
    }

    #[test]
    fn check_plane_splits_on_overlap() {
        let mut pm = PlaneManager::new();
        let pl = pm.find_plane(0, 1, 0);
        for x in 10..=20 {
            pm.visplanes[pl as usize].top[x + 1] = 5;
        }
        pm.visplanes[pl as usize].minx = 10;
        pm.visplanes[pl as usize].maxx = 20;

        let split = pm.check_plane(pl, 15, 25);
        assert_ne!(split, pl);
        assert_eq!(pm.lastvisplane, 2);
        assert_eq!(pm.visplanes[split as usize].minx, 15);
        assert_eq!(pm.visplanes[split as usize].maxx, 25);
    }

    #[test]
    fn clear_planes_resets_clip_and_mapping() {
        let mut render = Render::new();
        render.viewangle = 0;
        render.centerxfrac = FRAC_UNIT;
        render.draw.viewwidth = SCREENWIDTH as i32;
        render.draw.viewheight = SCREENHEIGHT as i32;
        let mut pm = PlaneManager::new();
        pm.lastvisplane = 7;
        pm.floorclip[0] = 0;

        pm.clear_planes(&render);
        assert_eq!(pm.lastvisplane, 0);
        assert_eq!(pm.lastopening, 0);
        assert_eq!(pm.floorclip[0], SCREENHEIGHT as i16);
        assert_eq!(pm.ceilingclip[0], -1);
        // angle = (0 - ANG90) >> shift == 3*FINEANGLES/4
        let angle = ANG90.wrapping_neg() >> ANGLETOFINESHIFT;
        assert_eq!(pm.baseyscale, -fixed_div(finesine_index(angle), FRAC_UNIT));
    }
}
