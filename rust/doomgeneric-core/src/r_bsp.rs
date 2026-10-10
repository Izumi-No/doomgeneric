//! BSP traversal and wall-segment rendering, port of `r_bsp.c` and `r_segs.c`.
//!
//! The original shares a pile of globals (`curline`, `sidedef`, `drawsegs`,
//! `solidsegs`, `rw_*`). Those live in [`SegManager`]; the visplane pools and
//! clip columns are owned by [`PlaneManager`], and the view/light tables by
//! [`Render`]. Sprite insertion during a subsector visit is delegated to a
//! [`SpriteSink`] so this module stays independent of the thing/mobj system.

use crate::doomdef::{ML_DONTPEGBOTTOM, ML_DONTPEGTOP, NF_SUBSECTOR};
use crate::fixed::{fixed_mul, Fixed, FRAC_BITS};
use crate::level::{Level, Node};
use crate::r_data::RenderData;
use crate::r_main::{
    point_on_side, LightOffset, Render, LIGHTLEVELS, LIGHTSCALESHIFT, LIGHTSEGSHIFT, MAXLIGHTSCALE,
};
use crate::r_plane::PlaneManager;
use crate::r_setup::TextureNumbers;
use crate::tables::{
    finesine_index, finetangent_index, ANG180, ANG90, ANGLETOFINESHIFT, FINEANGLES,
};
use crate::wad::WadManager;

/// Maximum number of stored drawsegs.
pub const MAXDRAWSEGS: usize = 256;
/// Maximum number of solid clip posts.
pub const MAXSEGS: usize = 32;

// Silhouette flags (`r_defs.h`).
pub const SIL_NONE: i32 = 0;
pub const SIL_BOTTOM: i32 = 1;
pub const SIL_TOP: i32 = 2;
pub const SIL_BOTH: i32 = 3;

// Node bounding-box coordinate order (`m_bbox.h`).
const NBOX_TOP: usize = 0;
const NBOX_BOTTOM: usize = 1;
const NBOX_LEFT: usize = 2;
const NBOX_RIGHT: usize = 3;

// `R_RenderSegLoop` fixed-point height unit.
const HEIGHTBITS: u32 = 12;
const HEIGHTUNIT: i32 = 1 << HEIGHTBITS;

/// A clipping post in the solid-wall range list.
#[derive(Clone, Copy, Default)]
pub struct ClipRange {
    pub first: i32,
    pub last: i32,
}

/// Where a drawseg's per-column sprite clipping bounds live.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClipRef {
    /// No clipping recorded (`NULL`).
    None,
    /// The per-frame `screenheightarray` (bottom-most bound).
    ScreenHeight,
    /// The constant `negonearray` (all `-1`).
    NegOne,
    /// Offset into [`PlaneManager::openings`] (may be negative, like a pointer).
    Opening(isize),
}

/// `drawseg_t`.
#[derive(Clone)]
pub struct DrawSeg {
    pub curline: u32,
    pub x1: i32,
    pub x2: i32,
    pub scale1: Fixed,
    pub scale2: Fixed,
    pub scalestep: Fixed,
    pub silhouette: i32,
    pub bsilheight: Fixed,
    pub tsilheight: Fixed,
    pub sprtopclip: ClipRef,
    pub sprbottomclip: ClipRef,
    pub maskedtexturecol: Option<isize>,
}

impl Default for DrawSeg {
    fn default() -> Self {
        DrawSeg {
            curline: 0,
            x1: 0,
            x2: 0,
            scale1: 0,
            scale2: 0,
            scalestep: 0,
            silhouette: SIL_NONE,
            bsilheight: 0,
            tsilheight: 0,
            sprtopclip: ClipRef::None,
            sprbottomclip: ClipRef::None,
            maskedtexturecol: None,
        }
    }
}

/// Receiver for sprites gathered while visiting subsectors.
pub trait SpriteSink {
    fn add_sprites(&mut self, sector: usize);
}

impl SpriteSink for () {
    fn add_sprites(&mut self, _sector: usize) {}
}

/// `checkcoord` table from `r_bsp.c`, indexed by `(boxy<<2)+boxx`.
#[rustfmt::skip]
pub const CHECKCOORD: [[usize; 4]; 12] = [
    [3, 0, 2, 1],
    [3, 0, 2, 0],
    [3, 1, 2, 0],
    [0, 0, 0, 0],
    [2, 0, 2, 1],
    [0, 0, 0, 0],
    [3, 1, 3, 0],
    [0, 0, 0, 0],
    [2, 0, 3, 1],
    [2, 1, 3, 1],
    [2, 1, 3, 0],
    [0, 0, 0, 0],
];

/// All `r_bsp.c`/`r_segs.c` state (the loose globals in C).
pub struct SegManager {
    pub drawsegs: [DrawSeg; MAXDRAWSEGS],
    pub ds_p: usize,

    pub solidsegs: [ClipRange; MAXSEGS],
    pub newend: usize,

    pub curline: u32,

    // Per-wall state (`r_segs.c`).
    pub segtextured: bool,
    pub markfloor: bool,
    pub markceiling: bool,
    pub maskedtexture: bool,
    pub toptexture: i32,
    pub bottomtexture: i32,
    pub midtexture: i32,

    pub walllights: [LightOffset; MAXLIGHTSCALE],

    pub rw_x: i32,
    pub rw_stopx: i32,
    pub rw_centerangle: u32,
    pub rw_offset: Fixed,
    pub rw_scale: Fixed,
    pub rw_scalestep: Fixed,
    pub rw_midtexturemid: Fixed,
    pub rw_toptexturemid: Fixed,
    pub rw_bottomtexturemid: Fixed,
    pub maskedtexture_offset: isize,

    pub worldtop: i32,
    pub worldbottom: i32,
    pub worldhigh: i32,
    pub worldlow: i32,

    pub pixhigh: Fixed,
    pub pixlow: Fixed,
    pub pixhighstep: Fixed,
    pub pixlowstep: Fixed,

    pub topfrac: Fixed,
    pub topstep: Fixed,
    pub bottomfrac: Fixed,
    pub bottomstep: Fixed,
}

impl Default for SegManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything the wall renderer needs, bundled to keep the recursive BSP
/// signatures manageable.
struct WallCtx<'a> {
    render: &'a mut Render,
    planes: &'a mut PlaneManager,
    level: &'a Level,
    tn: &'a TextureNumbers,
    data: &'a mut RenderData,
    manager: &'a WadManager,
    colormaps: &'a [u8],
    screen: &'a mut [u8],
    sink: &'a mut dyn SpriteSink,
}

impl WallCtx<'_> {
    /// Draw one wall column with the selected light table.
    fn blit_column(&mut self, tex: i32, texturecolumn: i32, colormap_off: usize) {
        let Ok(source) = self.data.column(self.manager, tex as usize, texturecolumn) else {
            return;
        };
        let colormap = &self.colormaps[colormap_off..];
        if self.render.detailshift == 0 {
            self.render.draw.draw_column(self.screen, colormap, source);
        } else {
            self.render
                .draw
                .draw_column_low(self.screen, colormap, source);
        }
    }
}

#[inline]
fn fine_idx(angle: u32) -> usize {
    (angle >> ANGLETOFINESHIFT) as usize & (FINEANGLES / 2 - 1)
}

impl SegManager {
    pub fn new() -> Self {
        SegManager {
            drawsegs: core::array::from_fn(|_| DrawSeg::default()),
            ds_p: 0,
            solidsegs: [ClipRange::default(); MAXSEGS],
            newend: 0,
            curline: 0,
            segtextured: false,
            markfloor: false,
            markceiling: false,
            maskedtexture: false,
            toptexture: 0,
            bottomtexture: 0,
            midtexture: 0,
            walllights: [0; MAXLIGHTSCALE],
            rw_x: 0,
            rw_stopx: 0,
            rw_centerangle: 0,
            rw_offset: 0,
            rw_scale: 0,
            rw_scalestep: 0,
            rw_midtexturemid: 0,
            rw_toptexturemid: 0,
            rw_bottomtexturemid: 0,
            maskedtexture_offset: 0,
            worldtop: 0,
            worldbottom: 0,
            worldhigh: 0,
            worldlow: 0,
            pixhigh: 0,
            pixlow: 0,
            pixhighstep: 0,
            pixlowstep: 0,
            topfrac: 0,
            topstep: 0,
            bottomfrac: 0,
            bottomstep: 0,
        }
    }

    /// `R_ClearDrawSegs`.
    pub fn clear_drawsegs(&mut self) {
        self.ds_p = 0;
    }

    /// `R_ClearClipSegs`.
    pub fn clear_clip_segs(&mut self, viewwidth: i32) {
        self.solidsegs[0].first = i32::MIN;
        self.solidsegs[0].last = -1;
        self.solidsegs[1].first = viewwidth;
        self.solidsegs[1].last = i32::MAX;
        self.newend = 2;
    }

    /// Drive a full BSP walk, drawing walls into `screen` and collecting
    /// sprites through `sink`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_bsp(
        &mut self,
        bspnum: i32,
        render: &mut Render,
        planes: &mut PlaneManager,
        level: &Level,
        tn: &TextureNumbers,
        data: &mut RenderData,
        manager: &WadManager,
        screen: &mut [u8],
        sink: &mut dyn SpriteSink,
    ) {
        let colormaps = data.colormaps.clone();
        let mut ctx = WallCtx {
            render,
            planes,
            level,
            tn,
            data,
            manager,
            colormaps: &colormaps,
            screen,
            sink,
        };
        self.render_bsp_node(bspnum, &mut ctx);
    }

    fn render_bsp_node(&mut self, bspnum: i32, ctx: &mut WallCtx) {
        if bspnum & NF_SUBSECTOR as i32 != 0 {
            if bspnum == -1 {
                self.subsector(0, ctx);
            } else {
                self.subsector((bspnum & !(NF_SUBSECTOR as i32)) as usize, ctx);
            }
            return;
        }

        let node: &Node = &ctx.level.nodes[bspnum as usize];
        let side = point_on_side(ctx.render.viewx, ctx.render.viewy, node);

        self.render_bsp_node(node.children[side] as i32, ctx);

        let other = node.bbox[side ^ 1];
        if self.check_bbox(&other, ctx.render) {
            self.render_bsp_node(node.children[side ^ 1] as i32, ctx);
        }
    }

    /// `R_CheckBBox`: might any part of this BSP bounding box be visible?
    pub fn check_bbox(&self, bspcoord: &[i32; 4], render: &Render) -> bool {
        let boxx = if render.viewx <= bspcoord[NBOX_LEFT] {
            0
        } else if render.viewx < bspcoord[NBOX_RIGHT] {
            1
        } else {
            2
        };
        let boxy = if render.viewy >= bspcoord[NBOX_TOP] {
            0
        } else if render.viewy > bspcoord[NBOX_BOTTOM] {
            1
        } else {
            2
        };

        let boxpos = (boxy << 2) + boxx;
        if boxpos == 5 {
            return true;
        }

        let c = CHECKCOORD[boxpos];
        let x1 = bspcoord[c[0]];
        let y1 = bspcoord[c[1]];
        let x2 = bspcoord[c[2]];
        let y2 = bspcoord[c[3]];

        let mut angle1 = render.point_to_angle(x1, y1).wrapping_sub(render.viewangle);
        let mut angle2 = render.point_to_angle(x2, y2).wrapping_sub(render.viewangle);

        let span = angle1.wrapping_sub(angle2);
        if span >= ANG180 {
            return true;
        }

        let mut tspan = angle1.wrapping_add(render.clipangle);
        if tspan > 2 * render.clipangle {
            tspan = tspan.wrapping_sub(2 * render.clipangle);
            if tspan >= span {
                return false;
            }
            angle1 = render.clipangle;
        }

        let mut tspan = render.clipangle.wrapping_sub(angle2);
        if tspan > 2 * render.clipangle {
            tspan = tspan.wrapping_sub(2 * render.clipangle);
            if tspan >= span {
                return false;
            }
            angle2 = render.clipangle.wrapping_neg();
        }

        let sx1 = render.viewangletox[fine_idx(angle1.wrapping_add(ANG90))];
        let sx2 = render.viewangletox[fine_idx(angle2.wrapping_add(ANG90))];
        if sx1 == sx2 {
            return false;
        }
        let sx2 = sx2 - 1;

        let mut start = 0usize;
        while self.solidsegs[start].last < sx2 {
            start += 1;
        }

        !(sx1 >= self.solidsegs[start].first && sx2 <= self.solidsegs[start].last)
    }

    /// `R_Subsector`: find planes, add sprites, clip all segs.
    fn subsector(&mut self, num: usize, ctx: &mut WallCtx) {
        ctx.render.sscount += 1;
        let sub = ctx.level.subsectors[num];
        let front_idx = sub.sector;
        let sec = &ctx.level.sectors[front_idx];

        if sec.floorheight < ctx.render.viewz {
            ctx.planes.floorplane = ctx.planes.find_plane(
                sec.floorheight,
                ctx.tn.sector_floor[front_idx],
                sec.lightlevel as i32,
            );
        } else {
            ctx.planes.floorplane = -1;
        }

        if sec.ceilingheight > ctx.render.viewz
            || ctx.tn.sector_ceiling[front_idx] == ctx.planes.skyflatnum
        {
            ctx.planes.ceilingplane = ctx.planes.find_plane(
                sec.ceilingheight,
                ctx.tn.sector_ceiling[front_idx],
                sec.lightlevel as i32,
            );
        } else {
            ctx.planes.ceilingplane = -1;
        }

        ctx.sink.add_sprites(front_idx);

        let count = sub.numlines as i32;
        let first = sub.firstline as i32;
        for i in 0..count {
            self.add_line((first + i) as usize, ctx);
        }
    }

    /// `R_AddLine`.
    fn add_line(&mut self, seg_idx: usize, ctx: &mut WallCtx) {
        self.curline = seg_idx as u32;
        let seg = ctx.level.segs[seg_idx];
        let v1 = ctx.level.vertexes[seg.v1 as usize];
        let v2 = ctx.level.vertexes[seg.v2 as usize];

        let angle1 = ctx.render.point_to_angle(v1.x, v1.y);
        let angle2 = ctx.render.point_to_angle(v2.x, v2.y);

        let span = angle1.wrapping_sub(angle2);
        if span >= ANG180 {
            return;
        }

        ctx.render.rw_angle1 = angle1;
        let mut a1 = angle1.wrapping_sub(ctx.render.viewangle);
        let mut a2 = angle2.wrapping_sub(ctx.render.viewangle);

        let two_clip = ctx.render.clipangle.wrapping_mul(2);
        let mut tspan = a1.wrapping_add(ctx.render.clipangle);
        if tspan > two_clip {
            tspan = tspan.wrapping_sub(two_clip);
            if tspan >= span {
                return;
            }
            a1 = ctx.render.clipangle;
        }
        let mut tspan = ctx.render.clipangle.wrapping_sub(a2);
        if tspan > two_clip {
            tspan = tspan.wrapping_sub(two_clip);
            if tspan >= span {
                return;
            }
            a2 = ctx.render.clipangle.wrapping_neg();
        }

        let x1 = ctx.render.viewangletox[fine_idx(a1.wrapping_add(ANG90))];
        let x2 = ctx.render.viewangletox[fine_idx(a2.wrapping_add(ANG90))];
        if x1 == x2 {
            return;
        }

        let front_idx = seg.frontsector;
        let back_idx = seg.backsector;

        // Single sided line.
        let Some(back_idx) = back_idx else {
            self.clip_solid_wall_segment(x1, x2 - 1, ctx);
            return;
        };

        let front = &ctx.level.sectors[front_idx];
        let back = &ctx.level.sectors[back_idx];

        // Closed door.
        if back.ceilingheight <= front.floorheight || back.floorheight >= front.ceilingheight {
            self.clip_solid_wall_segment(x1, x2 - 1, ctx);
            return;
        }

        // Window.
        if back.ceilingheight != front.ceilingheight || back.floorheight != front.floorheight {
            self.clip_pass_wall_segment(x1, x2 - 1, ctx);
            return;
        }

        // Reject empty lines used for triggers / special events.
        let mid = ctx.tn.side_mid[sidedef_index(ctx, seg_idx) as usize];
        if ctx.tn.sector_ceiling[back_idx] == ctx.tn.sector_ceiling[front_idx]
            && ctx.tn.sector_floor[back_idx] == ctx.tn.sector_floor[front_idx]
            && back.lightlevel == front.lightlevel
            && mid == 0
        {
            return;
        }

        self.clip_pass_wall_segment(x1, x2 - 1, ctx);
    }

    /// `R_ClipSolidWallSegment`.
    fn clip_solid_wall_segment(&mut self, first: i32, last: i32, ctx: &mut WallCtx) {
        let mut start = 0usize;
        while self.solidsegs[start].last < first - 1 {
            start += 1;
        }

        if first < self.solidsegs[start].first {
            if last < self.solidsegs[start].first - 1 {
                self.store_wall_range(first, last, ctx);
                let mut next = self.newend;
                self.newend += 1;
                while next != start {
                    self.solidsegs[next] = self.solidsegs[next - 1];
                    next -= 1;
                }
                self.solidsegs[next].first = first;
                self.solidsegs[next].last = last;
                return;
            }

            let f = self.solidsegs[start].first - 1;
            self.store_wall_range(first, f, ctx);
            self.solidsegs[start].first = first;
        }

        if last <= self.solidsegs[start].last {
            return;
        }

        let mut next = start;
        let mut crunched = false;
        while last >= self.solidsegs[next + 1].first - 1 {
            let a = self.solidsegs[next].last + 1;
            let b = self.solidsegs[next + 1].first - 1;
            self.store_wall_range(a, b, ctx);
            next += 1;

            if last <= self.solidsegs[next].last {
                self.solidsegs[start].last = self.solidsegs[next].last;
                crunched = true;
                break;
            }
        }

        if !crunched {
            let a = self.solidsegs[next].last + 1;
            self.store_wall_range(a, last, ctx);
            self.solidsegs[start].last = last;
        }

        if next == start {
            return;
        }

        // Remove start+1..=next from the clip list, preserving the C idiom.
        let mut s = start;
        let mut n = next;
        while n != self.newend {
            n += 1;
            s += 1;
            self.solidsegs[s] = self.solidsegs[n];
        }
        self.newend = s + 1;
    }

    /// `R_ClipPassWallSegment`.
    fn clip_pass_wall_segment(&mut self, first: i32, last: i32, ctx: &mut WallCtx) {
        let mut start = 0usize;
        while self.solidsegs[start].last < first - 1 {
            start += 1;
        }

        if first < self.solidsegs[start].first {
            if last < self.solidsegs[start].first - 1 {
                self.store_wall_range(first, last, ctx);
                return;
            }
            let f = self.solidsegs[start].first - 1;
            self.store_wall_range(first, f, ctx);
        }

        if last <= self.solidsegs[start].last {
            return;
        }

        while last >= self.solidsegs[start + 1].first - 1 {
            let a = self.solidsegs[start].last + 1;
            let b = self.solidsegs[start + 1].first - 1;
            self.store_wall_range(a, b, ctx);
            start += 1;

            if last <= self.solidsegs[start].last {
                return;
            }
        }

        let a = self.solidsegs[start].last + 1;
        self.store_wall_range(a, last, ctx);
    }

    /// `R_StoreWallRange`.
    fn store_wall_range(&mut self, start: i32, stop: i32, ctx: &mut WallCtx) {
        if self.ds_p == MAXDRAWSEGS {
            return;
        }

        let seg = ctx.level.segs[self.curline as usize];
        let line = ctx.level.lines[seg.linedef as usize];
        let side_idx = seg.side as usize;
        let sidenum = line.sidenum[side_idx];
        if sidenum < 0 {
            return;
        }
        let sidenum = sidenum as usize;
        let side = ctx.level.sides[sidenum];
        let front_idx = seg.frontsector;
        let back_idx = seg.backsector;
        let front = &ctx.level.sectors[front_idx];
        let v1 = ctx.level.vertexes[seg.v1 as usize];

        ctx.render.rw_normalangle = seg.angle.wrapping_add(ANG90);
        let mut offsetangle = ctx.render.rw_normalangle.wrapping_sub(ctx.render.rw_angle1);
        if offsetangle > ANG90 {
            offsetangle = ANG90;
        }
        let distangle = ANG90 - offsetangle;
        let hyp = ctx.render.point_to_dist(v1.x, v1.y);
        let sineval = finesine_index(distangle >> ANGLETOFINESHIFT);
        ctx.render.rw_distance = fixed_mul(hyp, sineval);

        let mut ds = DrawSeg {
            x1: start,
            x2: stop,
            curline: self.curline,
            ..DrawSeg::default()
        };
        self.rw_x = start;
        self.rw_stopx = stop + 1;

        self.rw_scale = ctx.render.scale_from_global_angle(
            ctx.render
                .viewangle
                .wrapping_add(ctx.render.xtoviewangle[start as usize]),
        );
        ds.scale1 = self.rw_scale;

        if stop > start {
            ds.scale2 = ctx.render.scale_from_global_angle(
                ctx.render
                    .viewangle
                    .wrapping_add(ctx.render.xtoviewangle[stop as usize]),
            );
            self.rw_scalestep = (ds.scale2 - self.rw_scale) / (stop - start);
        } else {
            ds.scale2 = ds.scale1;
            self.rw_scalestep = 0;
        }
        ds.scalestep = self.rw_scalestep;

        self.worldtop = front.ceilingheight - ctx.render.viewz;
        self.worldbottom = front.floorheight - ctx.render.viewz;

        self.midtexture = 0;
        self.toptexture = 0;
        self.bottomtexture = 0;
        self.maskedtexture = false;
        self.markfloor = false;
        self.markceiling = false;
        ds.maskedtexturecol = None;

        match back_idx {
            None => {
                // Single sided line: terminal, must mark both ends.
                self.midtexture = ctx.data.texture_translation[ctx.tn.side_mid[sidenum] as usize];
                self.markfloor = true;
                self.markceiling = true;
                if line.flags & ML_DONTPEGBOTTOM != 0 {
                    let vtop = front.floorheight + ctx.data.texture_height[sidenum];
                    self.rw_midtexturemid = vtop - ctx.render.viewz;
                } else {
                    self.rw_midtexturemid = self.worldtop;
                }
                self.rw_midtexturemid += side.rowoffset;

                ds.silhouette = SIL_BOTH;
                ds.sprtopclip = ClipRef::ScreenHeight;
                ds.sprbottomclip = ClipRef::NegOne;
                ds.bsilheight = i32::MAX;
                ds.tsilheight = i32::MIN;
            }
            Some(back_idx) => {
                let back = &ctx.level.sectors[back_idx];
                ds.sprtopclip = ClipRef::None;
                ds.sprbottomclip = ClipRef::None;
                ds.silhouette = SIL_NONE;

                if front.floorheight > back.floorheight {
                    ds.silhouette = SIL_BOTTOM;
                    ds.bsilheight = front.floorheight;
                } else if back.floorheight > ctx.render.viewz {
                    ds.silhouette = SIL_BOTTOM;
                    ds.bsilheight = i32::MAX;
                }

                if front.ceilingheight < back.ceilingheight {
                    ds.silhouette |= SIL_TOP;
                    ds.tsilheight = front.ceilingheight;
                } else if back.ceilingheight < ctx.render.viewz {
                    ds.silhouette |= SIL_TOP;
                    ds.tsilheight = i32::MIN;
                }

                if back.ceilingheight <= front.floorheight {
                    ds.sprbottomclip = ClipRef::NegOne;
                    ds.bsilheight = i32::MAX;
                    ds.silhouette |= SIL_BOTTOM;
                }

                if back.floorheight >= front.ceilingheight {
                    ds.sprtopclip = ClipRef::ScreenHeight;
                    ds.tsilheight = i32::MIN;
                    ds.silhouette |= SIL_TOP;
                }

                self.worldhigh = back.ceilingheight - ctx.render.viewz;
                self.worldlow = back.floorheight - ctx.render.viewz;

                // Hack to allow height changes in outdoor areas.
                if ctx.tn.sector_ceiling[front_idx] == ctx.planes.skyflatnum
                    && ctx.tn.sector_ceiling[back_idx] == ctx.planes.skyflatnum
                {
                    self.worldtop = self.worldhigh;
                }

                self.markfloor = self.worldlow != self.worldbottom
                    || ctx.tn.sector_floor[back_idx] != ctx.tn.sector_floor[front_idx]
                    || back.lightlevel != front.lightlevel;
                self.markceiling = self.worldhigh != self.worldtop
                    || ctx.tn.sector_ceiling[back_idx] != ctx.tn.sector_ceiling[front_idx]
                    || back.lightlevel != front.lightlevel;

                if back.ceilingheight <= front.floorheight
                    || back.floorheight >= front.ceilingheight
                {
                    // closed door
                    self.markceiling = true;
                    self.markfloor = true;
                }

                if self.worldhigh < self.worldtop {
                    self.toptexture =
                        ctx.data.texture_translation[ctx.tn.side_top[sidenum] as usize];
                    if line.flags & ML_DONTPEGTOP != 0 {
                        self.rw_toptexturemid = self.worldtop;
                    } else {
                        let vtop = back.ceilingheight + ctx.data.texture_height[sidenum];
                        self.rw_toptexturemid = vtop - ctx.render.viewz;
                    }
                }
                if self.worldlow > self.worldbottom {
                    self.bottomtexture =
                        ctx.data.texture_translation[ctx.tn.side_bottom[sidenum] as usize];
                    if line.flags & ML_DONTPEGBOTTOM != 0 {
                        self.rw_bottomtexturemid = self.worldtop;
                    } else {
                        self.rw_bottomtexturemid = self.worldlow;
                    }
                }
                self.rw_toptexturemid += side.rowoffset;
                self.rw_bottomtexturemid += side.rowoffset;

                if ctx.tn.side_mid[sidenum] != 0 {
                    self.maskedtexture = true;
                    self.maskedtexture_offset =
                        ctx.planes.lastopening as isize - self.rw_x as isize;
                    ds.maskedtexturecol = Some(self.maskedtexture_offset);
                    ctx.planes.lastopening += (self.rw_stopx - self.rw_x) as usize;
                }
            }
        }

        self.segtextured =
            (self.midtexture | self.toptexture | self.bottomtexture) != 0 || self.maskedtexture;

        if self.segtextured {
            let mut offsetangle = ctx.render.rw_normalangle.wrapping_sub(ctx.render.rw_angle1);
            if offsetangle > ANG180 {
                offsetangle = offsetangle.wrapping_neg();
            }
            if offsetangle > ANG90 {
                offsetangle = ANG90;
            }
            let sineval = finesine_index(offsetangle >> ANGLETOFINESHIFT);
            let mut rw_offset = fixed_mul(hyp, sineval);
            if ctx.render.rw_normalangle.wrapping_sub(ctx.render.rw_angle1) < ANG180 {
                rw_offset = rw_offset.wrapping_neg();
            }
            rw_offset += side.textureoffset + seg.offset;
            self.rw_offset = rw_offset;
            self.rw_centerangle = ANG90
                .wrapping_add(ctx.render.viewangle)
                .wrapping_sub(ctx.render.rw_normalangle);

            if ctx.render.fixedcolormap == 0 {
                let mut lightnum =
                    (front.lightlevel >> LIGHTSEGSHIFT) as i32 + ctx.render.extralight;
                if v1.y == ctx.level.vertexes[seg.v2 as usize].y {
                    lightnum -= 1;
                } else if v1.x == ctx.level.vertexes[seg.v2 as usize].x {
                    lightnum += 1;
                }
                let row = if lightnum < 0 {
                    0
                } else if lightnum >= LIGHTLEVELS as i32 {
                    LIGHTLEVELS - 1
                } else {
                    lightnum as usize
                };
                self.walllights = ctx.render.scalelight[row];
            }
        }

        // Planes on the wrong side of the view plane are invisible.
        if front.floorheight >= ctx.render.viewz {
            self.markfloor = false;
        }
        if front.ceilingheight <= ctx.render.viewz
            && ctx.tn.sector_ceiling[front_idx] != ctx.planes.skyflatnum
        {
            self.markceiling = false;
        }

        // Incremental stepping values for texture edges.
        self.worldtop >>= 4;
        self.worldbottom >>= 4;
        self.topstep = -fixed_mul(self.rw_scalestep, self.worldtop);
        self.topfrac = (ctx.render.centeryfrac >> 4) - fixed_mul(self.worldtop, self.rw_scale);
        self.bottomstep = -fixed_mul(self.rw_scalestep, self.worldbottom);
        self.bottomfrac =
            (ctx.render.centeryfrac >> 4) - fixed_mul(self.worldbottom, self.rw_scale);

        if back_idx.is_some() {
            self.worldhigh >>= 4;
            self.worldlow >>= 4;

            if self.worldhigh < self.worldtop {
                self.pixhigh =
                    (ctx.render.centeryfrac >> 4) - fixed_mul(self.worldhigh, self.rw_scale);
                self.pixhighstep = -fixed_mul(self.rw_scalestep, self.worldhigh);
            }
            if self.worldlow > self.worldbottom {
                self.pixlow =
                    (ctx.render.centeryfrac >> 4) - fixed_mul(self.worldlow, self.rw_scale);
                self.pixlowstep = -fixed_mul(self.rw_scalestep, self.worldlow);
            }
        }

        if self.markceiling {
            let cp = ctx.planes.ceilingplane;
            ctx.planes.ceilingplane = ctx.planes.check_plane(cp, self.rw_x, self.rw_stopx - 1);
        }
        if self.markfloor {
            let fp = ctx.planes.floorplane;
            ctx.planes.floorplane = ctx.planes.check_plane(fp, self.rw_x, self.rw_stopx - 1);
        }

        self.render_seg_loop(ctx);

        // Save sprite clipping info.
        if ((ds.silhouette & SIL_TOP) != 0 || self.maskedtexture) && ds.sprtopclip == ClipRef::None
        {
            let base = ctx.planes.lastopening as isize - start as isize;
            let len = (self.rw_stopx - start) as usize;
            for i in 0..len {
                ctx.planes.openings[ctx.planes.lastopening + i] =
                    ctx.planes.ceilingclip[start as usize + i];
            }
            ctx.planes.lastopening += len;
            ds.sprtopclip = ClipRef::Opening(base);
        }
        if ((ds.silhouette & SIL_BOTTOM) != 0 || self.maskedtexture)
            && ds.sprbottomclip == ClipRef::None
        {
            let base = ctx.planes.lastopening as isize - start as isize;
            let len = (self.rw_stopx - start) as usize;
            for i in 0..len {
                ctx.planes.openings[ctx.planes.lastopening + i] =
                    ctx.planes.floorclip[start as usize + i];
            }
            ctx.planes.lastopening += len;
            ds.sprbottomclip = ClipRef::Opening(base);
        }

        if self.maskedtexture && (ds.silhouette & SIL_TOP) == 0 {
            ds.silhouette |= SIL_TOP;
            ds.tsilheight = i32::MIN;
        }
        if self.maskedtexture && (ds.silhouette & SIL_BOTTOM) == 0 {
            ds.silhouette |= SIL_BOTTOM;
            ds.bsilheight = i32::MAX;
        }

        self.drawsegs[self.ds_p] = ds;
        self.ds_p += 1;
    }

    /// `R_RenderSegLoop`.
    fn render_seg_loop(&mut self, ctx: &mut WallCtx) {
        let mut rw_x = self.rw_x;
        while rw_x < self.rw_stopx {
            // mark floor/ceiling areas
            let mut yl = (self.topfrac + HEIGHTUNIT - 1) >> HEIGHTBITS;
            if yl < ctx.planes.ceilingclip[rw_x as usize] as i32 + 1 {
                yl = ctx.planes.ceilingclip[rw_x as usize] as i32 + 1;
            }

            if self.markceiling {
                let top = ctx.planes.ceilingclip[rw_x as usize] as i32 + 1;
                let mut bottom = yl - 1;
                if bottom >= ctx.planes.floorclip[rw_x as usize] as i32 {
                    bottom = ctx.planes.floorclip[rw_x as usize] as i32 - 1;
                }
                if top <= bottom {
                    let cp = ctx.planes.ceilingplane as usize;
                    ctx.planes.visplanes[cp].top[(rw_x + 1) as usize] = top as u8;
                    ctx.planes.visplanes[cp].bottom[(rw_x + 1) as usize] = bottom as u8;
                }
            }

            let mut yh = self.bottomfrac >> HEIGHTBITS;
            if yh >= ctx.planes.floorclip[rw_x as usize] as i32 {
                yh = ctx.planes.floorclip[rw_x as usize] as i32 - 1;
            }

            if self.markfloor {
                let mut top = yh + 1;
                let bottom = ctx.planes.floorclip[rw_x as usize] as i32 - 1;
                if top <= ctx.planes.ceilingclip[rw_x as usize] as i32 {
                    top = ctx.planes.ceilingclip[rw_x as usize] as i32 + 1;
                }
                if top <= bottom {
                    let fp = ctx.planes.floorplane as usize;
                    ctx.planes.visplanes[fp].top[(rw_x + 1) as usize] = top as u8;
                    ctx.planes.visplanes[fp].bottom[(rw_x + 1) as usize] = bottom as u8;
                }
            }

            // texture column and lighting
            let mut texturecolumn = 0i32;
            let mut colormap_off = 0usize;
            if self.segtextured {
                let angle = self
                    .rw_centerangle
                    .wrapping_add(ctx.render.xtoviewangle[rw_x as usize])
                    >> ANGLETOFINESHIFT;
                texturecolumn = (self.rw_offset
                    - fixed_mul(finetangent_index(angle), ctx.render.rw_distance))
                    >> FRAC_BITS;

                let mut index = (self.rw_scale >> LIGHTSCALESHIFT) as usize;
                if index >= MAXLIGHTSCALE {
                    index = MAXLIGHTSCALE - 1;
                }
                colormap_off = self.walllights[index];
                ctx.render.draw.dc_x = rw_x;
                ctx.render.draw.dc_iscale = (0xffff_ffffu32 / self.rw_scale as u32) as i32;
            }

            // draw the wall tiers
            if self.midtexture != 0 {
                ctx.render.draw.dc_yl = yl;
                ctx.render.draw.dc_yh = yh;
                ctx.render.draw.dc_texturemid = self.rw_midtexturemid;
                ctx.blit_column(self.midtexture, texturecolumn, colormap_off);
                ctx.planes.ceilingclip[rw_x as usize] = ctx.render.draw.viewheight as i16;
                ctx.planes.floorclip[rw_x as usize] = -1;
            } else {
                if self.toptexture != 0 {
                    let mut mid = self.pixhigh >> HEIGHTBITS;
                    self.pixhigh += self.pixhighstep;
                    if mid >= ctx.planes.floorclip[rw_x as usize] as i32 {
                        mid = ctx.planes.floorclip[rw_x as usize] as i32 - 1;
                    }
                    if mid >= yl {
                        ctx.render.draw.dc_yl = yl;
                        ctx.render.draw.dc_yh = mid;
                        ctx.render.draw.dc_texturemid = self.rw_toptexturemid;
                        ctx.blit_column(self.toptexture, texturecolumn, colormap_off);
                        ctx.planes.ceilingclip[rw_x as usize] = mid as i16;
                    } else {
                        ctx.planes.ceilingclip[rw_x as usize] = (yl - 1) as i16;
                    }
                } else if self.markceiling {
                    ctx.planes.ceilingclip[rw_x as usize] = (yl - 1) as i16;
                }

                if self.bottomtexture != 0 {
                    let mut mid = (self.pixlow + HEIGHTUNIT - 1) >> HEIGHTBITS;
                    self.pixlow += self.pixlowstep;
                    if mid <= ctx.planes.ceilingclip[rw_x as usize] as i32 {
                        mid = ctx.planes.ceilingclip[rw_x as usize] as i32 + 1;
                    }
                    if mid <= yh {
                        ctx.render.draw.dc_yl = mid;
                        ctx.render.draw.dc_yh = yh;
                        ctx.render.draw.dc_texturemid = self.rw_bottomtexturemid;
                        ctx.blit_column(self.bottomtexture, texturecolumn, colormap_off);
                        ctx.planes.floorclip[rw_x as usize] = mid as i16;
                    } else {
                        ctx.planes.floorclip[rw_x as usize] = (yh + 1) as i16;
                    }
                } else if self.markfloor {
                    ctx.planes.floorclip[rw_x as usize] = (yh + 1) as i16;
                }

                if self.maskedtexture {
                    let idx = (self.maskedtexture_offset + rw_x as isize) as usize;
                    ctx.planes.openings[idx] = texturecolumn as i16;
                }
            }

            self.rw_scale += self.rw_scalestep;
            self.topfrac += self.topstep;
            self.bottomfrac += self.bottomstep;
            rw_x += 1;
        }
    }

    /// `R_RenderMaskedSegRange`: draw the masked mid-texture walls of a stored
    /// drawseg. `x1..=x2` is the visible sub-range of `ds.x1..=ds.x2`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_masked_seg_range(
        &mut self,
        ds_index: usize,
        x1: i32,
        x2: i32,
        render: &mut Render,
        planes: &mut PlaneManager,
        level: &Level,
        tn: &TextureNumbers,
        data: &mut RenderData,
        manager: &WadManager,
        screen: &mut [u8],
        colormaps: &[u8],
    ) {
        let ds = self.drawsegs[ds_index].clone();
        let seg = level.segs[ds.curline as usize];
        let line = level.lines[seg.linedef as usize];
        let front = &level.sectors[line.frontsector.unwrap_or(0)];
        let back = line.backsector.map(|b| &level.sectors[b]).unwrap_or(front);
        let sidenum = line.sidenum[seg.side as usize] as usize;
        let side = &level.sides[sidenum];
        let texnum = data.texture_translation[tn.side_mid[sidenum] as usize];

        let mut lightnum = i32::from(front.lightlevel >> LIGHTSEGSHIFT) + render.extralight;
        let v1 = level.vertexes[seg.v1 as usize];
        let v2 = level.vertexes[seg.v2 as usize];
        if v1.y == v2.y {
            lightnum -= 1;
        } else if v1.x == v2.x {
            lightnum += 1;
        }
        let row = if lightnum < 0 {
            0
        } else if lightnum >= LIGHTLEVELS as i32 {
            LIGHTLEVELS - 1
        } else {
            lightnum as usize
        };
        let walllights = render.scalelight[row];

        let offset = ds.maskedtexturecol.unwrap_or(0);
        let mut spryscale = ds
            .scale1
            .wrapping_add((x1 - ds.x1).wrapping_mul(ds.scalestep));

        let mut dc_texturemid = if line.flags & ML_DONTPEGBOTTOM != 0 {
            let base = front.floorheight.max(back.floorheight);
            base + data.texture_height[texnum as usize] - render.viewz
        } else {
            let base = front.ceilingheight.min(back.ceilingheight);
            base - render.viewz
        };
        dc_texturemid += side.rowoffset;

        for dc_x in x1..=x2 {
            let idx = (offset + dc_x as isize) as usize;
            let col_val = planes.openings[idx];
            if col_val != i16::MAX {
                let colormap_off = if render.fixedcolormap != 0 {
                    render.fixedcolormap as usize
                } else {
                    let mut index = (spryscale >> LIGHTSCALESHIFT) as usize;
                    if index >= MAXLIGHTSCALE {
                        index = MAXLIGHTSCALE - 1;
                    }
                    walllights[index]
                };
                let sprtopscreen = render
                    .centeryfrac
                    .wrapping_sub(fixed_mul(dc_texturemid, spryscale));
                render.draw.dc_iscale = (0xffff_ffffu32 / spryscale as u32) as Fixed;
                render.draw.dc_x = dc_x;

                let floorclip = clip_at(ds.sprbottomclip, dc_x, render, planes);
                let ceilingclip = clip_at(ds.sprtopclip, dc_x, render, planes);
                let colormap = &colormaps[colormap_off..];
                if let Ok(column) = data.masked_column(manager, texnum as usize, col_val as i32) {
                    render.draw.draw_masked_column(
                        screen,
                        colormap,
                        column,
                        floorclip,
                        ceilingclip,
                        spryscale,
                        sprtopscreen,
                    );
                }
                planes.openings[idx] = i16::MAX;
            }
            spryscale += ds.scalestep;
        }
    }
}

/// Value of a sprite clip reference at column `x`.
fn clip_at(clip: ClipRef, x: i32, render: &Render, planes: &PlaneManager) -> i32 {
    match clip {
        ClipRef::None => 0,
        ClipRef::ScreenHeight => render.screenheightarray[x as usize],
        ClipRef::NegOne => -1,
        ClipRef::Opening(off) => planes.openings[(off + x as isize) as usize] as i32,
    }
}

/// Sidedef index for a seg.
fn sidedef_index(ctx: &WallCtx, seg_idx: usize) -> i32 {
    let seg = ctx.level.segs[seg_idx];
    let line = ctx.level.lines[seg.linedef as usize];
    line.sidenum[seg.side as usize] as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_clip_segs_sets_sentinels() {
        let mut sm = SegManager::new();
        sm.clear_clip_segs(320);
        assert_eq!(sm.solidsegs[0].first, i32::MIN);
        assert_eq!(sm.solidsegs[0].last, -1);
        assert_eq!(sm.solidsegs[1].first, 320);
        assert_eq!(sm.solidsegs[1].last, i32::MAX);
        assert_eq!(sm.newend, 2);
    }

    #[test]
    fn checkcoord_table_shape() {
        assert_eq!(CHECKCOORD.len(), 12);
        assert_eq!(CHECKCOORD[5], [0, 0, 0, 0]);
        assert_eq!(CHECKCOORD[0], [3, 0, 2, 1]);
    }
}
