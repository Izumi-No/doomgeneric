//! Low level drawing primitives.
//!
//! Port of `r_draw.c`. The original keeps the `dc_*`/`ds_*` parameters in
//! globals and dispatches through function pointers (`colfunc`, `spanfunc`).
//! Here they live in [`Draw`] and the helpers receive the colormap/source
//! slices explicitly, which keeps the module free of `unsafe`.
//!
//! The screen buffer is the raw 320x200 palette-index framebuffer. Row
//! offsets (`ylookup`) are stored as byte offsets relative to the start of
//! that buffer instead of absolute pointers.

use crate::fixed::{Fixed, FRAC_BITS, FRAC_UNIT};

pub const SCREENWIDTH: usize = 320;
pub const SCREENHEIGHT: usize = 200;

pub const SBARHEIGHT: i32 = 32;

pub const MAXWIDTH: usize = 1120;
pub const MAXHEIGHT: usize = 832;

pub const FUZZTABLE: usize = 50;
pub const FUZZOFF: i32 = SCREENWIDTH as i32;

/// Spectre / partial-invisibility dithering offsets (from `r_draw.c`).
pub static FUZZOFFSET: [i32; FUZZTABLE] = [
    FUZZOFF, -FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF,
    FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, -FUZZOFF, -FUZZOFF,
    -FUZZOFF, FUZZOFF, -FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF,
    -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, -FUZZOFF, -FUZZOFF,
    -FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF,
];

/// Which primitive each masked post is drawn with (`colfunc` dispatch).
#[derive(Clone, Copy)]
pub enum ColumnKind<'a> {
    /// `R_DrawColumn`.
    Normal,
    /// `R_DrawFuzzColumn` (spectre/shadow).
    Fuzz,
    /// `R_DrawTranslatedColumn` with a 256-byte translation page.
    Translated(&'a [u8]),
}

/// All mutable drawing state (the `dc_*`/`ds_*` globals plus the view window).
pub struct Draw {
    pub viewwidth: i32,
    pub scaledviewwidth: i32,
    pub viewheight: i32,
    pub viewwindowx: i32,
    pub viewwindowy: i32,
    pub centery: i32,
    pub fuzzpos: i32,

    /// Byte offset of each view row from the start of the screen buffer.
    pub ylookup: [i32; MAXHEIGHT],
    /// Byte offset of each view column (viewwindowx + i).
    pub columnofs: [i32; MAXWIDTH],

    // column state
    pub dc_x: i32,
    pub dc_yl: i32,
    pub dc_yh: i32,
    pub dc_iscale: Fixed,
    pub dc_texturemid: Fixed,

    // span state
    pub ds_y: i32,
    pub ds_x1: i32,
    pub ds_x2: i32,
    pub ds_xfrac: Fixed,
    pub ds_yfrac: Fixed,
    pub ds_xstep: Fixed,
    pub ds_ystep: Fixed,

}

impl Default for Draw {
    fn default() -> Self {
        Self::new()
    }
}

impl Draw {
    pub fn new() -> Self {
        Draw {
            viewwidth: 0,
            scaledviewwidth: 0,
            viewheight: 0,
            viewwindowx: 0,
            viewwindowy: 0,
            centery: 0,
            fuzzpos: 0,
            ylookup: [0; MAXHEIGHT],
            columnofs: [0; MAXWIDTH],
            dc_x: 0,
            dc_yl: 0,
            dc_yh: 0,
            dc_iscale: 0,
            dc_texturemid: 0,
            ds_y: 0,
            ds_x1: 0,
            ds_x2: 0,
            ds_xfrac: 0,
            ds_yfrac: 0,
            ds_xstep: 0,
            ds_ystep: 0,
        }
    }

    /// `R_InitBuffer`: build the lookup tables for a view of `width` x `height`.
    pub fn init_buffer(&mut self, width: i32, height: i32) {
        self.viewwindowx = (SCREENWIDTH as i32 - width) >> 1;
        for i in 0..width {
            self.columnofs[i as usize] = self.viewwindowx + i;
        }

        if width == SCREENWIDTH as i32 {
            self.viewwindowy = 0;
        } else {
            self.viewwindowy = (SCREENHEIGHT as i32 - SBARHEIGHT - height) >> 1;
        }

        for i in 0..height {
            self.ylookup[i as usize] = (i + self.viewwindowy) * SCREENWIDTH as i32;
        }
    }

    /// `R_DrawColumn`.
    pub fn draw_column(&mut self, screen: &mut [u8], colormap: &[u8], source: &[u8]) {
        let mut count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let mut dest =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[self.dc_x as usize]) as usize;
        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        // Vanilla reads the column with a bare index; slices taller than the
        // texture would read out of bounds there (UB the C gets away with).
        // The port wraps, which matches the vertical tiling look.
        let slen = source.len().max(1);
        loop {
            let idx = ((frac >> FRAC_BITS) as usize).wrapping_sub(0) % slen;
            screen[dest] = colormap[source[idx] as usize];
            dest += SCREENWIDTH;
            frac = frac.wrapping_add(fracstep);

            count -= 1;
            if count < 0 {
                break;
            }
        }
    }

    /// `R_DrawMaskedColumn`: draw posts of a masked column (`column_t` format).
    ///
    /// `column` starts at the first post's `topdelta` byte and runs to the
    /// `0xff` terminator. `mfloorclip`/`mceilingclip` are the sprite clip
    /// bounds for the current column (`dc_x`); `spryscale`/`sprtopscreen` are
    /// the sprite projection.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_masked_column(
        &mut self,
        screen: &mut [u8],
        colormap: &[u8],
        column: &[u8],
        mfloorclip: i32,
        mceilingclip: i32,
        spryscale: Fixed,
        sprtopscreen: Fixed,
    ) {
        self.draw_masked_column_kind(
            screen,
            colormap,
            &[],
            column,
            mfloorclip,
            mceilingclip,
            spryscale,
            sprtopscreen,
            ColumnKind::Normal,
        );
    }

    /// `R_DrawMaskedColumn` with a selectable per-post primitive: normal,
    /// translated (player colors) or fuzz (spectre shadow). Sprites use this;
    /// masked mid-textures only need [`Self::draw_masked_column`].
    #[allow(clippy::too_many_arguments)]
    pub fn draw_masked_column_kind(
        &mut self,
        screen: &mut [u8],
        colormap: &[u8],
        colormaps: &[u8],
        column: &[u8],
        mfloorclip: i32,
        mceilingclip: i32,
        spryscale: Fixed,
        sprtopscreen: Fixed,
        kind: ColumnKind<'_>,
    ) {
        let basetexturemid = self.dc_texturemid;
        let mut pos = 0usize;

        while pos < column.len() && column[pos] != 0xff {
            let topdelta = column[pos] as i32;
            if pos + 1 >= column.len() {
                break;
            }
            let length = column[pos + 1] as i32;

            let topscreen = sprtopscreen.wrapping_add(spryscale.wrapping_mul(topdelta));
            let bottomscreen = topscreen.wrapping_add(spryscale.wrapping_mul(length));

            let mut yl = (topscreen + FRAC_UNIT - 1) >> FRAC_BITS;
            let mut yh = (bottomscreen - 1) >> FRAC_BITS;

            if yh >= mfloorclip {
                yh = mfloorclip - 1;
            }
            if yl <= mceilingclip {
                yl = mceilingclip + 1;
            }

            if yl <= yh {
                let start = pos + 3;
                let end = start.saturating_add(length as usize).min(column.len());
                if start < end {
                    self.dc_yl = yl;
                    self.dc_yh = yh;
                    self.dc_texturemid = basetexturemid.wrapping_sub(topdelta << FRAC_BITS);
                    let src = &column[start..end];
                    match kind {
                        ColumnKind::Normal => self.draw_column(screen, colormap, src),
                        ColumnKind::Fuzz => self.draw_fuzz_column(screen, colormaps),
                        ColumnKind::Translated(tr) => {
                            self.draw_translated_column(screen, colormap, tr, src)
                        }
                    }
                }
            }

            pos += length as usize + 4;
        }

        self.dc_texturemid = basetexturemid;
    }

    /// Low detail (`R_DrawColumnLow`): every source pixel fills two columns.
    pub fn draw_column_low(&mut self, screen: &mut [u8], colormap: &[u8], source: &[u8]) {
        let mut count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let x = self.dc_x << 1;
        let mut dest = (self.ylookup[self.dc_yl as usize] + self.columnofs[x as usize]) as usize;
        let mut dest2 =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[x as usize + 1]) as usize;
        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        loop {
            let slen = source.len().max(1);
            let c = colormap[source[((frac >> FRAC_BITS) as usize) % slen] as usize];
            screen[dest] = c;
            screen[dest2] = c;
            dest += SCREENWIDTH;
            dest2 += SCREENWIDTH;
            frac = frac.wrapping_add(fracstep);

            count -= 1;
            if count < 0 {
                break;
            }
        }
    }

    /// `R_DrawFuzzColumn`: spectre dithering using colormap #6.
    pub fn draw_fuzz_column(&mut self, screen: &mut [u8], colormaps: &[u8]) {
        if self.dc_yl == 0 {
            self.dc_yl = 1;
        }
        if self.dc_yh == self.viewheight - 1 {
            self.dc_yh = self.viewheight - 2;
        }

        let mut count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let mut dest =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[self.dc_x as usize]) as usize;
        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        loop {
            let src = (dest as i32 + FUZZOFFSET[self.fuzzpos as usize]) as usize;
            screen[dest] = colormaps[6 * 256 + screen[src] as usize];

            self.fuzzpos += 1;
            if self.fuzzpos == FUZZTABLE as i32 {
                self.fuzzpos = 0;
            }

            dest += SCREENWIDTH;
            frac = frac.wrapping_add(fracstep);

            count -= 1;
            if count < 0 {
                break;
            }
        }
    }

    /// Low detail `R_DrawFuzzColumnLow`.
    pub fn draw_fuzz_column_low(&mut self, screen: &mut [u8], colormaps: &[u8]) {
        if self.dc_yl == 0 {
            self.dc_yl = 1;
        }
        if self.dc_yh == self.viewheight - 1 {
            self.dc_yh = self.viewheight - 2;
        }

        let mut count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let x = self.dc_x << 1;
        let mut dest = (self.ylookup[self.dc_yl as usize] + self.columnofs[x as usize]) as usize;
        let mut dest2 =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[x as usize + 1]) as usize;
        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        loop {
            let off = FUZZOFFSET[self.fuzzpos as usize];
            let src = (dest as i32 + off) as usize;
            let src2 = (dest2 as i32 + off) as usize;
            screen[dest] = colormaps[6 * 256 + screen[src] as usize];
            screen[dest2] = colormaps[6 * 256 + screen[src2] as usize];

            self.fuzzpos += 1;
            if self.fuzzpos == FUZZTABLE as i32 {
                self.fuzzpos = 0;
            }

            dest += SCREENWIDTH;
            dest2 += SCREENWIDTH;
            frac = frac.wrapping_add(fracstep);

            count -= 1;
            if count < 0 {
                break;
            }
        }
    }

    /// `R_DrawTranslatedColumn`: remap source through `translation`.
    pub fn draw_translated_column(
        &mut self,
        screen: &mut [u8],
        colormap: &[u8],
        translation: &[u8],
        source: &[u8],
    ) {
        let mut count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let mut dest =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[self.dc_x as usize]) as usize;
        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        loop {
            let idx = (frac >> FRAC_BITS) as usize;
            screen[dest] = colormap[translation[source[idx] as usize] as usize];
            dest += SCREENWIDTH;
            frac = frac.wrapping_add(fracstep);

            count -= 1;
            if count < 0 {
                break;
            }
        }
    }

    /// Low detail `R_DrawTranslatedColumnLow`.
    pub fn draw_translated_column_low(
        &mut self,
        screen: &mut [u8],
        colormap: &[u8],
        translation: &[u8],
        source: &[u8],
    ) {
        let mut count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let x = self.dc_x << 1;
        let mut dest = (self.ylookup[self.dc_yl as usize] + self.columnofs[x as usize]) as usize;
        let mut dest2 =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[x as usize + 1]) as usize;
        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        loop {
            let idx = (frac >> FRAC_BITS) as usize;
            let c = colormap[translation[source[idx] as usize] as usize];
            screen[dest] = c;
            screen[dest2] = c;
            dest += SCREENWIDTH;
            dest2 += SCREENWIDTH;
            frac = frac.wrapping_add(fracstep);

            count -= 1;
            if count < 0 {
                break;
            }
        }
    }

    /// `R_DrawSpan`: a horizontal floor/ceiling slice, texture coords packed.
    pub fn draw_span(&mut self, screen: &mut [u8], colormap: &[u8], source: &[u8]) {
        let xf = self.ds_xfrac as u32;
        let yf = self.ds_yfrac as u32;
        let mut position = ((xf << 10) & 0xffff_0000) | ((yf >> 6) & 0x0000_ffff);
        let step = (((self.ds_xstep as u32) << 10) & 0xffff_0000)
            | (((self.ds_ystep as u32) >> 6) & 0x0000_ffff);

        let mut dest =
            (self.ylookup[self.ds_y as usize] + self.columnofs[self.ds_x1 as usize]) as usize;
        let mut count = self.ds_x2 - self.ds_x1;

        loop {
            let ytemp = (position >> 4) & 0x0fc0;
            let xtemp = position >> 26;
            let spot = (xtemp | ytemp) as usize;
            screen[dest] = colormap[source[spot] as usize];
            dest += 1;
            position = position.wrapping_add(step);

            count -= 1;
            if count < 0 {
                break;
            }
        }
    }

    /// Low detail `R_DrawSpanLow`.
    pub fn draw_span_low(&mut self, screen: &mut [u8], colormap: &[u8], source: &[u8]) {
        let xf = self.ds_xfrac as u32;
        let yf = self.ds_yfrac as u32;
        let mut position = ((xf << 10) & 0xffff_0000) | ((yf >> 6) & 0x0000_ffff);
        let step = (((self.ds_xstep as u32) << 10) & 0xffff_0000)
            | (((self.ds_ystep as u32) >> 6) & 0x0000_ffff);

        let mut count = self.ds_x2 - self.ds_x1;

        let x1 = self.ds_x1 << 1;
        let mut dest = (self.ylookup[self.ds_y as usize] + self.columnofs[x1 as usize]) as usize;

        loop {
            let ytemp = (position >> 4) & 0x0fc0;
            let xtemp = position >> 26;
            let spot = (xtemp | ytemp) as usize;
            let c = colormap[source[spot] as usize];
            screen[dest] = c;
            screen[dest + 1] = c;
            dest += 2;
            position = position.wrapping_add(step);

            count -= 1;
            if count < 0 {
                break;
            }
        }
    }
}

/// `R_InitTranslationTables`: green ramp -> gray/brown/red for player sprites.
pub fn init_translation_tables() -> [u8; 768] {
    let mut t = [0u8; 768];
    for i in 0..256usize {
        if (0x70..=0x7f).contains(&i) {
            t[i] = (0x60 + (i & 0xf)) as u8;
            t[i + 256] = (0x40 + (i & 0xf)) as u8;
            t[i + 512] = (0x20 + (i & 0xf)) as u8;
        } else {
            t[i] = i as u8;
            t[i + 256] = i as u8;
            t[i + 512] = i as u8;
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::FRAC_UNIT;

    fn identity() -> [u8; 256] {
        let mut c = [0u8; 256];
        for (i, v) in c.iter_mut().enumerate() {
            *v = i as u8;
        }
        c
    }

    fn screen() -> [u8; SCREENWIDTH * SCREENHEIGHT] {
        [0u8; SCREENWIDTH * SCREENHEIGHT]
    }

    #[test]
    fn init_buffer_geometry() {
        let mut d = Draw::new();
        d.init_buffer(SCREENWIDTH as i32, SCREENHEIGHT as i32);
        assert_eq!(d.viewwindowx, 0);
        assert_eq!(d.viewwindowy, 0);
        assert_eq!(d.columnofs[0], 0);
        assert_eq!(d.columnofs[319], 319);
        assert_eq!(d.ylookup[0], 0);
        assert_eq!(d.ylookup[199], 199 * SCREENWIDTH as i32);
    }

    #[test]
    fn column_straight_copy() {
        let mut d = Draw::new();
        d.init_buffer(SCREENWIDTH as i32, SCREENHEIGHT as i32);
        d.centery = 0;
        d.dc_x = 0;
        d.dc_yl = 0;
        d.dc_yh = 3;
        d.dc_iscale = FRAC_UNIT;
        d.dc_texturemid = 0;

        let cm = identity();
        let src: [u8; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
        let mut s = screen();
        d.draw_column(&mut s, &cm, &src);

        assert_eq!(s[0], 10);
        assert_eq!(s[SCREENWIDTH], 20);
        assert_eq!(s[2 * SCREENWIDTH], 30);
        assert_eq!(s[3 * SCREENWIDTH], 40);
    }

    #[test]
    fn column_scaled() {
        let mut d = Draw::new();
        d.init_buffer(SCREENWIDTH as i32, SCREENHEIGHT as i32);
        d.centery = 0;
        d.dc_x = 0;
        d.dc_yl = 0;
        d.dc_yh = 3;
        // 0.5 in 16.16 -> every source pixel is drawn twice.
        d.dc_iscale = FRAC_UNIT >> 1;
        d.dc_texturemid = 0;

        let cm = identity();
        let src: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut s = screen();
        d.draw_column(&mut s, &cm, &src);

        assert_eq!(s[0], 1);
        assert_eq!(s[SCREENWIDTH], 1);
        assert_eq!(s[2 * SCREENWIDTH], 2);
        assert_eq!(s[3 * SCREENWIDTH], 2);
    }

    #[test]
    fn translated_column() {
        let mut d = Draw::new();
        d.init_buffer(SCREENWIDTH as i32, SCREENHEIGHT as i32);
        d.centery = 0;
        d.dc_x = 0;
        d.dc_yl = 0;
        d.dc_yh = 2;
        d.dc_iscale = FRAC_UNIT;
        d.dc_texturemid = 0;

        let cm = identity();
        let mut tr = [0u8; 256];
        for (i, v) in tr.iter_mut().enumerate() {
            *v = (i as u8).wrapping_add(1);
        }
        let src: [u8; 4] = [0, 1, 2, 3];
        let mut s = screen();
        d.draw_translated_column(&mut s, &cm, &tr, &src);
        assert_eq!(s[0], 1);
        assert_eq!(s[SCREENWIDTH], 2);
        assert_eq!(s[2 * SCREENWIDTH], 3);
    }

    #[test]
    fn span_straight_copy() {
        let mut d = Draw::new();
        d.init_buffer(SCREENWIDTH as i32, SCREENHEIGHT as i32);
        d.ds_y = 0;
        d.ds_x1 = 0;
        d.ds_x2 = 4;
        d.ds_xfrac = 0;
        d.ds_yfrac = 0;
        d.ds_xstep = FRAC_UNIT;
        d.ds_ystep = 0;

        let cm = identity();
        let mut src = [0u8; 64 * 64];
        for (i, v) in src.iter_mut().enumerate() {
            *v = i as u8;
        }
        let mut s = screen();
        d.draw_span(&mut s, &cm, &src);
        assert_eq!(&s[0..5], &[0, 1, 2, 3, 4]);
    }

    #[test]
    fn fuzz_copies_neighbour() {
        let mut d = Draw::new();
        d.init_buffer(SCREENWIDTH as i32, SCREENHEIGHT as i32);
        d.viewheight = SCREENHEIGHT as i32;
        d.centery = 0;
        d.dc_x = 0;
        d.dc_yl = 1;
        d.dc_yh = 3;
        d.fuzzpos = 0;

        let mut cm = [0u8; 8 * 256];
        // Colormap #6 of the fuzz: identity for this test.
        for i in 0..256 {
            cm[6 * 256 + i] = i as u8;
        }

        let mut s = screen();
        // Fill a band so the neighbour lookup has known values.
        for y in 0..5 {
            for x in 0..4 {
                s[y * SCREENWIDTH + x] = 99;
            }
        }
        d.draw_fuzz_column(&mut s, &cm);
        assert_eq!(s[SCREENWIDTH], 99);
        assert_eq!(s[2 * SCREENWIDTH], 99);
    }

    #[test]
    fn translation_tables_shape() {
        let t = init_translation_tables();
        assert_eq!(t[0x00], 0x00);
        assert_eq!(t[0x70], 0x60);
        assert_eq!(t[0x70 + 256], 0x40);
        assert_eq!(t[0x70 + 512], 0x20);
        assert_eq!(t[0x7f], 0x6f);
        assert_eq!(t[0x80], 0x80);
    }
}
