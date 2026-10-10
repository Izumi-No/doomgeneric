//! Sprite refresh, port of `r_things.c`.
//!
//! The original shares globals (`vissprites`, `spritelights`, the `mfloorclip`/
//! `mceilingclip` pointers). Here the definitions and the visible-sprite list
//! live in [`SpriteManager`]; sprite insertion during a subsector visit is
//! delegated through the [`SpriteSink`] trait, exactly like `r_bsp`.
//!
//! Deferred: `R_DrawPSprite`/`R_DrawPlayerSprites` (they need the player and
//! psprite state from the gameplay layer), and the low-detail masked variants
//! (drawn with the normal masked primitive, matching `r_segs`).

use alloc::vec;
use alloc::vec::Vec;

use crate::fixed::{fixed_div, fixed_mul, Fixed, FRAC_BITS, FRAC_UNIT};
use crate::info::{FF_FRAMEMASK, FF_FULLBRIGHT, NUMSPRITES, SPRNAMES};
use crate::level::Level;
use crate::p_mobj::{Mobj, MF_SHADOW, MF_TRANSLATION, MF_TRANSSHIFT, Mobjs};
use crate::r_bsp::{ClipRef, SegManager, SpriteSink, SIL_BOTTOM, SIL_TOP};
use crate::r_data::RenderData;
use crate::r_draw::{ColumnKind, SCREENWIDTH};
use crate::r_main::{
    point_on_seg_side, LightOffset, Render, LIGHTLEVELS, LIGHTSCALESHIFT, LIGHTSEGSHIFT,
    MAXLIGHTSCALE,
};
use crate::r_plane::PlaneManager;
use crate::r_setup::TextureNumbers;
use crate::tables::ANG45;
use crate::wad::{LumpName, WadManager};

/// Maximum visible sprites per frame (`MAXVISSPRITES`).
pub const MAXVISSPRITES: usize = 128;
/// `MAXFRAMES` (frame letters `A`..`\`).
pub const MAXFRAMES: usize = 29;

/// Sprites closer than this are ignored (`MINZ`).
const MINZ: Fixed = 4 * FRAC_UNIT;

// ---------------------------------------------------------------------------
// Sprite definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpriteError {
    BadFrame,
    MultipleRotZero,
    RotAndZero,
    TwoLumps,
    MissingRotations,
    MissingFrame,
}

/// `spriteframe_t`: the lump(s) and flip flags for one animation frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteFrame {
    /// If false, rotation 0 is used for every view.
    pub rotate: bool,
    /// Lump index relative to `firstspritelump` for rotations 0-7.
    pub lump: [i32; 8],
    /// Horizontal-flip flag per rotation.
    pub flip: [bool; 8],
}

impl Default for SpriteFrame {
    fn default() -> Self {
        SpriteFrame {
            rotate: false,
            lump: [-1; 8],
            flip: [false; 8],
        }
    }
}

/// `spritedef_t`: the frames present for one sprite name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpriteDef {
    pub frames: Vec<SpriteFrame>,
}

/// All sprite definitions, indexed by `Sprite`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpriteDefs {
    pub defs: Vec<SpriteDef>,
}

/// Mutable tri-state used while building (`R_InstallSpriteLump`'s `sprtemp`).
#[derive(Clone, Copy)]
struct SprTemp {
    /// -1 = unset, 0 = rotation-0 only, 1 = per-rotation.
    rotate: i8,
    lump: [i32; 8],
    flip: [u8; 8],
}

impl Default for SprTemp {
    fn default() -> Self {
        SprTemp {
            rotate: -1,
            lump: [-1; 8],
            flip: [0; 8],
        }
    }
}

/// `R_InstallSpriteLump`.
#[allow(clippy::too_many_arguments)]
fn install_sprite_lump(
    temp: &mut [SprTemp; MAXFRAMES],
    maxframe: &mut i32,
    first_sprite_lump: usize,
    lump: usize,
    frame: u32,
    rotation: u32,
    flipped: bool,
) -> Result<(), SpriteError> {
    if frame >= MAXFRAMES as u32 || rotation > 8 {
        return Err(SpriteError::BadFrame);
    }
    let frame = frame as usize;
    if frame as i32 > *maxframe {
        *maxframe = frame as i32;
    }

    if rotation == 0 {
        match temp[frame].rotate {
            -1 => {}
            0 => return Err(SpriteError::MultipleRotZero),
            _ => return Err(SpriteError::RotAndZero),
        }
        temp[frame].rotate = 0;
        for r in 0..8 {
            temp[frame].lump[r] = lump as i32 - first_sprite_lump as i32;
            temp[frame].flip[r] = u8::from(flipped);
        }
        return Ok(());
    }

    if temp[frame].rotate == 0 {
        return Err(SpriteError::RotAndZero);
    }
    temp[frame].rotate = 1;
    let rotation = (rotation - 1) as usize;
    if temp[frame].lump[rotation] != -1 {
        return Err(SpriteError::TwoLumps);
    }
    temp[frame].lump[rotation] = lump as i32 - first_sprite_lump as i32;
    temp[frame].flip[rotation] = u8::from(flipped);
    Ok(())
}

/// `R_InitSpriteDefs`: build every sprite definition from the `S_START`..`S_END`
/// sprite lumps. `first_sprite_lump`/`last_sprite_lump` are inclusive lump
/// indexes (the `S_START`/`S_END` markers are excluded).
pub fn init_sprite_defs(
    manager: &WadManager,
    first_sprite_lump: usize,
    last_sprite_lump: usize,
) -> Result<SpriteDefs, SpriteError> {
    let mut defs = Vec::with_capacity(NUMSPRITES);
    if first_sprite_lump > last_sprite_lump {
        return Ok(SpriteDefs { defs });
    }

    for name in SPRNAMES.iter() {
        let want = LumpName::from_bytes(name.as_bytes());
        let mut temp = [SprTemp::default(); MAXFRAMES];
        let mut maxframe: i32 = -1;

        for l in first_sprite_lump..=last_sprite_lump {
            let Some(lname) = manager.lump_name(l) else {
                continue;
            };
            let b = lname.as_bytes();
            if !name_eq4(&want, b) {
                continue;
            }

            let frame = u32::from(b[4]).wrapping_sub(u32::from(b'A'));
            let rotation = u32::from(b[5]).wrapping_sub(u32::from(b'0'));
            install_sprite_lump(
                &mut temp,
                &mut maxframe,
                first_sprite_lump,
                l,
                frame,
                rotation,
                false,
            )?;

            if b[6] != 0 && b[6] != b' ' {
                let frame = u32::from(b[6]).wrapping_sub(u32::from(b'A'));
                let rotation = u32::from(b[7]).wrapping_sub(u32::from(b'0'));
                install_sprite_lump(
                    &mut temp,
                    &mut maxframe,
                    first_sprite_lump,
                    l,
                    frame,
                    rotation,
                    true,
                )?;
            }
        }

        if maxframe < 0 {
            defs.push(SpriteDef::default());
            continue;
        }

        let numframes = (maxframe + 1) as usize;
        for t in temp.iter().take(numframes) {
            match t.rotate {
                -1 => return Err(SpriteError::MissingFrame),
                0 => {}
                1 => {
                    if t.lump[..8].contains(&-1) {
                        return Err(SpriteError::MissingRotations);
                    }
                }
                _ => return Err(SpriteError::MissingFrame),
            }
        }

        let frames = temp[..numframes]
            .iter()
            .map(|t| SpriteFrame {
                rotate: t.rotate == 1,
                lump: t.lump,
                flip: t.flip.map(|f| f != 0),
            })
            .collect();
        defs.push(SpriteDef { frames });
    }

    Ok(SpriteDefs { defs })
}

/// Compare a 4-char sprite prefix case-insensitively.
fn name_eq4(want: &LumpName, b: &[u8]) -> bool {
    let w = want.as_bytes();
    b[..4].eq_ignore_ascii_case(&w[..4])
}

// ---------------------------------------------------------------------------
// Visible sprites
// ---------------------------------------------------------------------------

/// `vissprite_t`: a thing's sprite that might be visible this frame.
#[derive(Clone, Debug)]
pub struct VisSprite {
    pub x1: i32,
    pub x2: i32,
    pub gx: Fixed,
    pub gy: Fixed,
    pub gz: Fixed,
    pub gzt: Fixed,
    pub startfrac: Fixed,
    pub scale: Fixed,
    pub xiscale: Fixed,
    pub texturemid: Fixed,
    pub patch: i32,
    /// Light colormap offset; `None` means a fuzz (shadow) draw.
    pub colormap: Option<LightOffset>,
    pub mobjflags: u32,
}

/// Sprite refresh state (the `vissprites` globals plus the built definitions).
pub struct SpriteManager {
    pub defs: SpriteDefs,
    pub vissprites: Vec<VisSprite>,
    pending_sectors: Vec<usize>,
    validcounts: Vec<i32>,
    /// `translationtables` (three 256-byte player color pages).
    pub translation: Vec<u8>,
    clipbot: [i16; SCREENWIDTH],
    cliptop: [i16; SCREENWIDTH],
}

/// Collects the sectors whose things should be projected (`R_AddSprites`).
impl SpriteSink for SpriteManager {
    fn add_sprites(&mut self, sector: usize) {
        self.pending_sectors.push(sector);
    }
}

impl SpriteManager {
    pub fn new(defs: SpriteDefs, num_sectors: usize) -> Self {
        SpriteManager {
            defs,
            vissprites: Vec::new(),
            pending_sectors: Vec::new(),
            validcounts: vec![0; num_sectors],
            translation: crate::r_draw::init_translation_tables().to_vec(),
            clipbot: [0; SCREENWIDTH],
            cliptop: [0; SCREENWIDTH],
        }
    }

    /// `R_ClearSprites` (called at frame start).
    pub fn clear_sprites(&mut self) {
        self.vissprites.clear();
        self.pending_sectors.clear();
    }

    /// Build all `R_ProjectSprite`s for the sectors collected during the BSP
    /// walk, deduplicating sectors by `validcount`.
    #[allow(clippy::too_many_arguments)]
    pub fn project_sprites(
        &mut self,
        level: &Level,
        mobjs: &Mobjs,
        render: &Render,
        sprite_width: &[i32],
        sprite_offset: &[i32],
        sprite_top_offset: &[i32],
    ) {
        let pending = core::mem::take(&mut self.pending_sectors);
        for &sec in &pending {
            if sec >= self.validcounts.len() || sec >= level.sectors.len() {
                continue;
            }
            if self.validcounts[sec] == render.validcount {
                continue;
            }
            self.validcounts[sec] = render.validcount;

            let lightnum = ((level.sectors[sec].lightlevel >> LIGHTSEGSHIFT) as i32)
                + render.extralight;
            let row = if lightnum < 0 {
                0
            } else if lightnum as usize >= LIGHTLEVELS {
                LIGHTLEVELS - 1
            } else {
                lightnum as usize
            };
            let lights = render.scalelight[row];

            let mut cur = mobjs.sector_head(sec);
            while let Some(id) = cur {
                let Some(thing) = mobjs.get(id) else { break };
                cur = thing.snext;
                self.project_sprite(
                    thing,
                    render,
                    sprite_width,
                    sprite_offset,
                    sprite_top_offset,
                    &lights,
                );
            }
        }
    }

    /// `R_ProjectSprite`.
    #[allow(clippy::too_many_arguments)]
    fn project_sprite(
        &mut self,
        thing: &Mobj,
        render: &Render,
        sprite_width: &[i32],
        sprite_offset: &[i32],
        sprite_top_offset: &[i32],
        lights: &[LightOffset],
    ) {
        let tr_x = thing.x - render.viewx;
        let tr_y = thing.y - render.viewy;

        let mut gxt = fixed_mul(tr_x, render.viewcos);
        let mut gyt = -fixed_mul(tr_y, render.viewsin);
        let tz = gxt - gyt;

        if tz < MINZ {
            return;
        }
        let xscale = fixed_div(render.projection, tz);

        gxt = -fixed_mul(tr_x, render.viewsin);
        gyt = fixed_mul(tr_y, render.viewcos);
        let mut tx = -(gyt + gxt);

        if tx.abs() > (tz << 2) {
            return;
        }

        let sprite = thing.sprite as usize;
        let Some(def) = self.defs.defs.get(sprite) else {
            return;
        };
        let fnum = thing.frame & FF_FRAMEMASK;
        if fnum < 0 || fnum as usize >= def.frames.len() {
            return;
        }
        let frame = def.frames[fnum as usize];

        let (lump, flip) = if frame.rotate {
            let ang = render.point_to_angle(thing.x, thing.y);
            let rot = (ang
                .wrapping_sub(thing.angle)
                .wrapping_add((ANG45 / 2).wrapping_mul(9))
                >> 29) as usize
                & 7;
            (frame.lump[rot], frame.flip[rot])
        } else {
            (frame.lump[0], frame.flip[0])
        };

        if lump < 0 {
            return;
        }
        let lump = lump as usize;
        if lump >= sprite_width.len()
            || lump >= sprite_offset.len()
            || lump >= sprite_top_offset.len()
        {
            return;
        }

        tx -= sprite_offset[lump];
        let x1 = (render.centerxfrac.wrapping_add(fixed_mul(tx, xscale))) >> FRAC_BITS;
        if x1 > render.viewwidth() {
            return;
        }

        tx += sprite_width[lump];
        let x2 = ((render.centerxfrac.wrapping_add(fixed_mul(tx, xscale))) >> FRAC_BITS) - 1;
        if x2 < 0 {
            return;
        }

        let gzt = thing.z + sprite_top_offset[lump];
        let mut vis = VisSprite {
            x1: x1.max(0),
            x2: x2.min(render.viewwidth() - 1),
            gx: thing.x,
            gy: thing.y,
            gz: thing.z,
            gzt,
            startfrac: 0,
            scale: xscale << render.detailshift,
            xiscale: 0,
            texturemid: gzt - render.viewz,
            patch: lump as i32,
            colormap: Some(0),
            mobjflags: thing.flags,
        };

        let iscale = fixed_div(FRAC_UNIT, xscale);
        if flip {
            vis.startfrac = sprite_width[lump] - 1;
            vis.xiscale = -iscale;
        } else {
            vis.startfrac = 0;
            vis.xiscale = iscale;
        }
        if vis.x1 > x1 {
            vis.startfrac += vis.xiscale * (vis.x1 - x1);
        }

        if thing.flags & MF_SHADOW != 0 {
            vis.colormap = None;
        } else if render.fixedcolormap != 0 {
            vis.colormap = Some(render.fixedcolormap as usize);
        } else if thing.frame & FF_FULLBRIGHT != 0 {
            vis.colormap = Some(0);
        } else {
            let mut index = (xscale >> (LIGHTSCALESHIFT - render.detailshift as u32)) as usize;
            if index >= MAXLIGHTSCALE {
                index = MAXLIGHTSCALE - 1;
            }
            vis.colormap = Some(lights[index]);
        }

        if self.vissprites.len() < MAXVISSPRITES {
            self.vissprites.push(vis);
        }
    }

    /// `R_SortVisSprites`: ascending scale (back to front).
    fn sort_vissprites(&mut self) {
        self.vissprites.sort_by_key(|v| v.scale);
    }

    /// `R_DrawMasked`: draw all vissprites then any remaining masked walls.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_masked(
        &mut self,
        seg: &mut SegManager,
        render: &mut Render,
        planes: &mut PlaneManager,
        level: &Level,
        tn: &TextureNumbers,
        data: &mut RenderData,
        manager: &WadManager,
        screen: &mut [u8],
        colormaps: &[u8],
    ) {
        self.sort_vissprites();

        for i in 0..self.vissprites.len() {
            self.draw_sprite(
                i, seg, render, planes, level, tn, data, manager, screen, colormaps,
            );
        }

        for i in (0..seg.ds_p).rev() {
            if seg.drawsegs[i].maskedtexturecol.is_some() {
                let (x1, x2) = (seg.drawsegs[i].x1, seg.drawsegs[i].x2);
                seg.render_masked_seg_range(
                    i, x1, x2, render, planes, level, tn, data, manager, screen, colormaps,
                );
            }
        }
    }

    /// `R_DrawSprite`: clip a vissprite against the drawsegs and draw it.
    #[allow(clippy::too_many_arguments)]
    fn draw_sprite(
        &mut self,
        spr_index: usize,
        seg: &mut SegManager,
        render: &mut Render,
        planes: &mut PlaneManager,
        level: &Level,
        tn: &TextureNumbers,
        data: &mut RenderData,
        manager: &WadManager,
        screen: &mut [u8],
        colormaps: &[u8],
    ) {
        let spr = self.vissprites[spr_index].clone();

        for x in spr.x1..=spr.x2 {
            self.clipbot[x as usize] = -2;
            self.cliptop[x as usize] = -2;
        }

        for i in (0..seg.ds_p).rev() {
            let ds = seg.drawsegs[i].clone();

            if ds.x1 > spr.x2
                || ds.x2 < spr.x1
                || (ds.silhouette == 0 && ds.maskedtexturecol.is_none())
            {
                continue;
            }

            let r1 = ds.x1.max(spr.x1);
            let r2 = ds.x2.min(spr.x2);

            let (scale, lowscale) = if ds.scale1 > ds.scale2 {
                (ds.scale1, ds.scale2)
            } else {
                (ds.scale2, ds.scale1)
            };

            let segd = level.segs[ds.curline as usize];
            let v1 = level.vertexes[segd.v1 as usize];
            let v2 = level.vertexes[segd.v2 as usize];
            let on_side = point_on_seg_side(spr.gx, spr.gy, &v1, &v2) != 0;

            if scale < spr.scale || (lowscale < spr.scale && !on_side) {
                if ds.maskedtexturecol.is_some() {
                    seg.render_masked_seg_range(
                        i, r1, r2, render, planes, level, tn, data, manager, screen, colormaps,
                    );
                }
                continue;
            }

            let mut silhouette = ds.silhouette;
            if spr.gz >= ds.bsilheight {
                silhouette &= !SIL_BOTTOM;
            }
            if spr.gzt <= ds.tsilheight {
                silhouette &= !SIL_TOP;
            }

            for x in r1..=r2 {
                let xi = x as usize;
                if silhouette & SIL_BOTTOM != 0 && self.clipbot[xi] == -2 {
                    self.clipbot[xi] = clip_ref_value(ds.sprbottomclip, x, render, planes);
                }
                if silhouette & SIL_TOP != 0 && self.cliptop[xi] == -2 {
                    self.cliptop[xi] = clip_ref_value(ds.sprtopclip, x, render, planes);
                }
            }
        }

        let viewheight = render.viewheight() as i16;
        for x in spr.x1..=spr.x2 {
            let xi = x as usize;
            if self.clipbot[xi] == -2 {
                self.clipbot[xi] = viewheight;
            }
            if self.cliptop[xi] == -2 {
                self.cliptop[xi] = -1;
            }
        }

        draw_vis_sprite(
            &spr,
            spr.x1,
            spr.x2,
            render,
            data,
            manager,
            screen,
            colormaps,
            &self.translation,
            &self.clipbot,
            &self.cliptop,
        );
    }

    /// `R_DrawPlayerSprites`: draw the display player's weapon/flash
    /// overlays (`R_DrawPSprite` for each active psprite slot).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_player_sprites(
        &mut self,
        player: &crate::d_player::Player,
        lightnum: i32,
        render: &mut Render,
        data: &mut RenderData,
        manager: &WadManager,
        screen: &mut [u8],
        colormaps: &[u8],
    ) {
        // `spritelights` from R_SetupFrame: the light row of the player's
        // sector (+ extralight), clamped to the table.
        let row = if lightnum < 0 {
            0
        } else if lightnum as usize >= LIGHTLEVELS {
            LIGHTLEVELS - 1
        } else {
            lightnum as usize
        };
        const BASEYCENTER: Fixed = 100; // SCREENHEIGHT/2
        let viewwidth = render.viewwidth();
        for psp in player.psprites.iter() {
            let Some(state_id) = psp.state else { continue };
            let st = crate::info::STATES[state_id as usize];
            // Vanilla's `if (!psp->state) return;`.
            let Some(sprdef) = self.defs.defs.get(st.sprite as usize) else {
                continue;
            };
            let frame = st.frame & FF_FRAMEMASK;
            let Some(sprframe) = sprdef.frames.get(frame as usize) else {
                continue;
            };
            if sprframe.lump[0] < 0 {
                continue;
            }
            let lump = sprframe.lump[0];
            let flip = sprframe.flip[0];

            // calculate edges of the shape (the sprite_* arrays are indexed
            // by the relative lump, like `R_ProjectSprite`).
            let mut tx = psp.sx - 160 * FRAC_UNIT;
            tx -= data.sprite_offset.get(lump as usize).copied().unwrap_or(0);
            let x1 = (render.centerxfrac + fixed_mul(tx, render.pspritescale)) >> FRAC_BITS;

            // off the right side
            if x1 > viewwidth {
                continue;
            }

            tx += data.sprite_width.get(lump as usize).copied().unwrap_or(0);
            let x2 = ((render.centerxfrac + fixed_mul(tx, render.pspritescale)) >> FRAC_BITS) - 1;

            // off the left side
            if x2 < 0 {
                continue;
            }

            let vis = VisSprite {
                x1: x1.max(0),
                x2: x2.min(viewwidth - 1),
                gx: 0,
                gy: 0,
                gz: 0,
                gzt: 0,
                startfrac: if flip {
                    data.sprite_width.get(lump as usize).copied().unwrap_or(0) - 1
                } else {
                    0
                },
                scale: render.pspritescale << render.detailshift,
                xiscale: if flip {
                    0i32.wrapping_sub(render.pspriteiscale)
                } else {
                    render.pspriteiscale
                },
                texturemid: (BASEYCENTER << FRAC_BITS)
                    + FRAC_UNIT / 2
                    - (psp.sy - data.sprite_top_offset.get(lump as usize).copied().unwrap_or(0)),
                patch: lump,
                colormap: Some(0),
                mobjflags: 0,
            };
            let mut vis = vis;
            if vis.x1 > x1 {
                vis.startfrac += vis.xiscale * (vis.x1 - x1);
            }

            // shadow draw / fixed color / full bright / local light
            let invis = player.powers[crate::doomdef::PowerType::Invisibility as usize];
            if invis > 4 * 32 || invis & 8 != 0 {
                vis.colormap = None;
            } else if render.fixedcolormap != 0 {
                vis.colormap = Some(render.fixedcolormap as usize);
            } else if st.frame & crate::info::FF_FULLBRIGHT != 0 {
                vis.colormap = Some(0);
            } else {
                vis.colormap = Some(render.scalelight[row][MAXLIGHTSCALE - 1]);
            }

            // The weapon never clips against drawsegs: clip to the view.
            for x in vis.x1..=vis.x2 {
                let xi = x as usize;
                self.clipbot[xi] = render.viewheight() as i16;
                self.cliptop[xi] = -1;
            }

            draw_vis_sprite(
                &vis,
                vis.x1,
                vis.x2,
                render,
                data,
                manager,
                screen,
                colormaps,
                &self.translation,
                &self.clipbot,
                &self.cliptop,
            );
        }
    }
}

/// `R_DrawVisSprite` + `R_DrawMaskedColumn` for one vissprite.
#[allow(clippy::too_many_arguments)]
fn draw_vis_sprite(
    vis: &VisSprite,
    x1: i32,
    x2: i32,
    render: &mut Render,
    data: &mut RenderData,
    manager: &WadManager,
    screen: &mut [u8],
    colormaps: &[u8],
    translation: &[u8],
    mfloorclip: &[i16],
    mceilingclip: &[i16],
) {
    let lump = (vis.patch + data.first_sprite_lump as i32) as usize;
    let colormap: &[u8] = match vis.colormap {
        Some(off) => colormaps.get(off..).unwrap_or(&[]),
        None => &[],
    };
    let kind = if vis.colormap.is_none() {
        ColumnKind::Fuzz
    } else if vis.mobjflags & MF_TRANSLATION != 0 {
        let off = (((vis.mobjflags & MF_TRANSLATION) >> (MF_TRANSSHIFT - 8)) as usize)
            .saturating_sub(256);
        ColumnKind::Translated(translation.get(off..off + 256).unwrap_or(&[]))
    } else {
        ColumnKind::Normal
    };

    render.draw.dc_iscale = vis.xiscale.abs() >> render.detailshift;
    render.draw.dc_texturemid = vis.texturemid;
    let spryscale = vis.scale;
    let sprtopscreen = render
        .centeryfrac
        .wrapping_sub(fixed_mul(vis.texturemid, spryscale));

    let mut frac = vis.startfrac;
    for dc_x in x1..=x2 {
        let texturecolumn = frac >> FRAC_BITS;
        if let Ok(column) = data.patch_column(manager, lump, texturecolumn) {
            render.draw.dc_x = dc_x;
            let fc = mfloorclip.get(dc_x as usize).copied().unwrap_or(-1) as i32;
            let cc = mceilingclip.get(dc_x as usize).copied().unwrap_or(0) as i32;
            render.draw.draw_masked_column_kind(
                screen, colormap, colormaps, column, fc, cc, spryscale, sprtopscreen, kind,
            );
        }
        frac = frac.wrapping_add(vis.xiscale);
    }
}

/// Value of a sprite clip reference at column `x`.
#[inline]
fn clip_ref_value(clip: ClipRef, x: i32, render: &Render, planes: &PlaneManager) -> i16 {
    match clip {
        ClipRef::None => 0,
        ClipRef::ScreenHeight => render.screenheightarray[x as usize] as i16,
        ClipRef::NegOne => -1,
        ClipRef::Opening(off) => planes.openings[(off + x as isize) as usize],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::FRAC_UNIT;
    use crate::p_mobj::Mobj;
    use crate::r_main::FIELDOFVIEW;

    fn wad(lumps: &[&str]) -> WadManager {
        let mut data = Vec::new();
        let mut dir = Vec::new();
        let mut pos = 12usize;
        for name in lumps {
            let payload = [0u8; 8];
            data.extend_from_slice(&payload);
            let mut n = [b' '; 8];
            for (i, c) in name.bytes().take(8).enumerate() {
                n[i] = c;
            }
            let mut e = Vec::new();
            e.extend_from_slice(&(pos as u32).to_le_bytes());
            e.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            e.extend_from_slice(&n);
            dir.push(e);
            pos += payload.len();
        }
        let mut w = Vec::new();
        w.extend_from_slice(b"PWAD");
        w.extend_from_slice(&(lumps.len() as u32).to_le_bytes());
        w.extend_from_slice(&((12 + data.len()) as u32).to_le_bytes());
        w.extend_from_slice(&data);
        for e in dir {
            w.extend_from_slice(&e);
        }
        let mut m = WadManager::new();
        m.add(w).unwrap();
        m
    }

    fn basic_render() -> Render {
        let mut r = Render::new();
        r.set_view_size(11, 0);
        r.execute_set_view_size();
        r.viewx = 0;
        r.viewy = 0;
        r.viewz = 41 * FRAC_UNIT;
        r.viewangle = 0;
        r.viewcos = FRAC_UNIT;
        r.viewsin = 0;
        r.validcount = 1;
        let _ = FIELDOFVIEW;
        r
    }

    fn manual_defs() -> SpriteDefs {
        let mut defs: Vec<SpriteDef> = Vec::new();
        for i in 0..NUMSPRITES {
            if i == crate::info::Sprite::SPR_TROO as usize {
                defs.push(SpriteDef {
                    frames: vec![SpriteFrame {
                        rotate: false,
                        lump: [0; 8],
                        flip: [false; 8],
                    }],
                });
            } else {
                defs.push(SpriteDef::default());
            }
        }
        SpriteDefs { defs }
    }

    #[test]
    fn build_single_rotation_frame() {
        let m = wad(&["S_START", "TROOA0", "S_END"]);
        let defs = init_sprite_defs(&m, 1, 1).unwrap();
        let troo = &defs.defs[crate::info::Sprite::SPR_TROO as usize];
        assert_eq!(troo.frames.len(), 1);
        assert!(!troo.frames[0].rotate);
        assert_eq!(troo.frames[0].lump[0], 0);
        assert_eq!(troo.frames[0].lump[7], 0);
    }

    #[test]
    fn build_eight_rotations() {
        let names = [
            "S_START", "POSSA1", "POSSA2", "POSSA3", "POSSA4", "POSSA5", "POSSA6", "POSSA7",
            "POSSA8", "S_END",
        ];
        let m = wad(&names);
        let defs = init_sprite_defs(&m, 1, 8).unwrap();
        let poss = &defs.defs[crate::info::Sprite::SPR_POSS as usize];
        assert_eq!(poss.frames.len(), 1);
        assert!(poss.frames[0].rotate);
        for r in 0..8 {
            assert_eq!(poss.frames[0].lump[r], r as i32);
        }
    }

    #[test]
    fn missing_rotations_is_error() {
        // POSSA1..POSSA7 (missing 8) must fail.
        let names = [
            "S_START", "POSSA1", "POSSA2", "POSSA3", "POSSA4", "POSSA5", "POSSA6", "POSSA7",
            "S_END",
        ];
        let m = wad(&names);
        assert_eq!(
            init_sprite_defs(&m, 1, 7),
            Err(SpriteError::MissingRotations)
        );
    }

    #[test]
    fn projects_thing_in_front() {
        let render = basic_render();
        let defs = manual_defs();
        let mut sm = SpriteManager::new(defs, 1);

        // A mobj 64 units in front of the viewer, centered.
        let mut thing = Mobj::from_type(crate::info::MobjType::MT_PLAYER, 64 << 16, 0, 0);
        thing.sprite = crate::info::Sprite::SPR_TROO;
        thing.frame = 0;
        thing.sector = 0;
        thing.z = 0;

        let width = [32 << 16];
        let offset = [16 << 16];
        let top = [56 << 16];
        let lights = [0usize; MAXLIGHTSCALE];
        sm.project_sprite(&thing, &render, &width, &offset, &top, &lights);
        assert_eq!(sm.vissprites.len(), 1, "sprite should be projected");
        let v = &sm.vissprites[0];
        assert!(v.x1 >= 0 && v.x2 < render.viewwidth());
        assert!(v.x1 < v.x2);
    }

    #[test]
    fn thing_behind_is_ignored() {
        let render = basic_render();
        let mut sm = SpriteManager::new(manual_defs(), 1);
        let mut thing = Mobj::from_type(crate::info::MobjType::MT_PLAYER, -(64 << 16), 0, 0);
        thing.sprite = crate::info::Sprite::SPR_TROO;
        let width = [32 << 16];
        let offset = [16 << 16];
        let top = [56 << 16];
        let lights = [0usize; MAXLIGHTSCALE];
        sm.project_sprite(&thing, &render, &width, &offset, &top, &lights);
        assert!(sm.vissprites.is_empty());
    }

    #[test]
    fn proj_dedups_sector_by_validcount() {
        let render = basic_render();
        let mut sm = SpriteManager::new(manual_defs(), 1);
        sm.add_sprites(0);
        sm.add_sprites(0);
        // dumb container: sector 0 has no mobjs, so this only checks it doesn't
        // double-count anything / panic.
        let level = Level::default();
        let mobjs = Mobjs::new(1);
        let width = [32 << 16];
        let offset = [16 << 16];
        let top = [56 << 16];
        sm.project_sprites(&level, &mobjs, &render, &width, &offset, &top);
        assert!(sm.vissprites.is_empty());
    }
}
