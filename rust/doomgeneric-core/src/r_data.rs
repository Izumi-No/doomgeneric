// Port of doomgeneric/r_data.c (Chocolate Doom variant).
// Texture / flat / sprite / colormap data layer.
// no_std, unsafe-free. Memory is owned Vecs; the WadManager is only
// consulted to fetch lump bytes.

use crate::wad::{LumpName, WadManager};
use alloc::vec;
use alloc::vec::Vec;

pub const NUM_COLORMAPS: usize = 32;
pub const COLORMAP_SIZE: usize = 256;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RDataError {
    MissingLump,
    MissingPatch,
    BadTextureDirectory,
    ColumnWithoutPatch,
    TextureTooBig,
}

type Result<T> = core::result::Result<T, RDataError>;

// mappatch_t (packed): originx, originy, patch, stepdir, colormap (i16 each)
const MAPPATCH_SIZE: usize = 10;
// maptexture_t packed header before the patches[] array
const MAPTEXTURE_HEADER: usize = 22;

#[inline]
fn rd_i16(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([b[o], b[o + 1]])
}

#[inline]
fn rd_i32(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

#[inline]
fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

#[inline]
fn ascii_case_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        let x = if a[i].is_ascii_lowercase() {
            a[i] - b'a' + b'A'
        } else {
            a[i]
        };
        let y = if b[i].is_ascii_lowercase() {
            b[i] - b'a' + b'A'
        } else {
            b[i]
        };
        if x != y {
            return false;
        }
    }
    true
}

// A single patch placement within a composited wall texture.
#[derive(Debug, Clone, Copy, Default)]
pub struct TexPatch {
    pub originx: i32,
    pub originy: i32,
    pub patch: i32, // WAD lump index of the source patch
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub name: [u8; 8],
    pub index: i32,
    pub next: i32, // next texture in hash chain, -1 = end
    pub width: i32,
    pub height: i32,
    pub patchcount: i32,
    pub patches: Vec<TexPatch>,
}

/// Whole rendering data layer. Mirrors the r_data.c globals, owned.
#[derive(Default)]
pub struct RenderData {
    pub num_textures: usize,
    pub textures: Vec<Texture>,
    pub texture_hash: Vec<i32>,
    pub texture_column_lump: Vec<Vec<i16>>,
    pub texture_column_ofs: Vec<Vec<u16>>,
    pub texture_composite: Vec<Option<Vec<u8>>>,
    pub texture_composite_size: Vec<i32>,
    pub texture_width_mask: Vec<i32>,
    pub texture_height: Vec<i32>,
    /// `texturetranslation`: maps a texture number to its current animation
    /// frame (identity unless an animation is set up). Also holds the two extra
    /// slots used for the exit switch.
    pub texture_translation: Vec<i32>,

    pub colormaps: Vec<u8>,

    pub first_flat: usize,
    pub last_flat: usize,
    pub num_flats: usize,
    pub flat_translation: Vec<i32>,

    pub first_sprite_lump: usize,
    pub last_sprite_lump: usize,
    pub num_sprite_lumps: usize,
    pub sprite_width: Vec<i32>,
    pub sprite_offset: Vec<i32>,
    pub sprite_top_offset: Vec<i32>,

    // On-demand cache of lump bytes (patches, flats, colormap...).
    lump_cache: Vec<Option<Vec<u8>>>,
}

/// Offset of a patch column within the patch lump (`patch_t` header is
/// w(2) h(2) left(2) top(2) = 8 bytes, then LONG columnofs[8]).
fn patch_column_offset(lump_cache: &[Option<Vec<u8>>], lump: i32, x_local: i32) -> usize {
    let data = match lump_cache.get(lump as usize) {
        Some(Some(d)) => d,
        _ => return 0,
    };
    if x_local < 0 || x_local >= rd_i16(data, 0) as i32 {
        return 0;
    }
    let e = 8 + 4 * x_local as usize;
    if e + 4 > data.len() {
        return 0;
    }
    rd_u32(data, e) as usize
}

impl RenderData {
    fn ensure_lump(&mut self, manager: &WadManager, idx: usize) -> Result<()> {
        if self.lump_cache.len() <= idx {
            return Err(RDataError::MissingLump);
        }
        if self.lump_cache[idx].is_none() {
            let data = manager
                .lump_data(idx)
                .map_err(|_| RDataError::MissingLump)?;
            self.lump_cache[idx] = Some(data.to_vec());
        }
        Ok(())
    }

    /// R_GetColumn: pixel data of texture column `col`.
    pub fn column<'m>(
        &'m mut self,
        manager: &WadManager,
        tex: usize,
        col: i32,
    ) -> Result<&'m [u8]> {
        let mask = self.texture_width_mask[tex];
        let col = (col & mask) as usize;
        let lump = self.texture_column_lump[tex][col];
        let ofs = self.texture_column_ofs[tex][col] as usize;
        if lump > 0 {
            let idx = lump as usize;
            self.ensure_lump(manager, idx)?;
            let data = self.lump_cache[idx].as_ref().unwrap();
            return Ok(&data[ofs..]);
        }
        if self.texture_composite[tex].is_none() {
            self.generate_composite(manager, tex)?;
        }
        let c = self.texture_composite[tex].as_ref().unwrap();
        Ok(&c[ofs..])
    }

    /// `R_GetColumn` minus the 3-byte offset skip, i.e. the start of the post
    /// list (`column_t`), for masked mid-textures. Single-patch columns store
    /// the raw patch column, so stepping back 3 bytes yields the `topdelta`.
    pub fn masked_column<'m>(
        &'m mut self,
        manager: &WadManager,
        tex: usize,
        col: i32,
    ) -> Result<&'m [u8]> {
        let mask = self.texture_width_mask[tex];
        let col = (col & mask) as usize;
        let lump = self.texture_column_lump[tex][col];
        let ofs = self.texture_column_ofs[tex][col] as usize;
        if lump > 0 {
            let idx = lump as usize;
            self.ensure_lump(manager, idx)?;
            let data = self.lump_cache[idx].as_ref().unwrap();
            return Ok(&data[ofs.saturating_sub(3)..]);
        }
        if self.texture_composite[tex].is_none() {
            self.generate_composite(manager, tex)?;
        }
        let c = self.texture_composite[tex].as_ref().unwrap();
        Ok(&c[ofs..])
    }

    /// Flat tile for a floor/ceiling picnum (flat index), 64x64 pixels.
    pub fn flat<'m>(&'m mut self, manager: &WadManager, flat_num: i32) -> Result<&'m [u8]> {
        let idx = self.first_flat + self.flat_translation[flat_num as usize] as usize;
        self.ensure_lump(manager, idx)?;
        let data = self.lump_cache[idx].as_ref().unwrap();
        Ok(data.as_slice())
    }

    /// Raw patch column for a sprite lump: the post list starting at the
    /// column's `topdelta` (`R_DrawVisSprite`'s `patch->columnofs[x]`).
    pub fn patch_column<'m>(
        &'m mut self,
        manager: &WadManager,
        lump: usize,
        x_local: i32,
    ) -> Result<&'m [u8]> {
        self.ensure_lump(manager, lump)?;
        let ofs = patch_column_offset(&self.lump_cache, lump as i32, x_local);
        let data = self.lump_cache[lump].as_ref().unwrap();
        Ok(&data[ofs.min(data.len())..])
    }

    pub fn colormap_ramp(&self, level: usize) -> Option<&[u8]> {
        let o = level * COLORMAP_SIZE;
        self.colormaps.get(o..o + COLORMAP_SIZE)
    }

    /// R_InitTextures + R_InitFlats + R_InitSpriteLumps + R_InitColormaps.
    pub fn init_data(manager: &WadManager) -> Result<RenderData> {
        let num_lumps = manager.num_lumps();
        let mut rd = RenderData {
            num_textures: 0,
            textures: vec![],
            texture_hash: vec![],
            texture_column_lump: vec![],
            texture_column_ofs: vec![],
            texture_composite: vec![],
            texture_composite_size: vec![],
            texture_width_mask: vec![],
            texture_height: vec![],
            texture_translation: vec![],
            colormaps: vec![],
            first_flat: 0,
            last_flat: 0,
            num_flats: 0,
            flat_translation: vec![],
            first_sprite_lump: 0,
            last_sprite_lump: 0,
            num_sprite_lumps: 0,
            sprite_width: vec![],
            sprite_offset: vec![],
            sprite_top_offset: vec![],
            lump_cache: vec![None; num_lumps],
        };
        rd.init_textures(manager)?;
        rd.init_flats(manager)?;
        rd.init_sprite_lumps(manager)?;
        rd.init_colormaps(manager)?;
        Ok(rd)
    }

    fn init_textures(&mut self, manager: &WadManager) -> Result<()> {
        // PNAMES: patch name lookup table.
        let names_lump = manager
            .check_num_for_name(&LumpName::from_bytes(b"PNAMES"))
            .ok_or(RDataError::MissingLump)?;
        let names = manager
            .lump_data(names_lump)
            .map_err(|_| RDataError::MissingLump)?;
        let num_mappatches = rd_i32(names, 0);
        if num_mappatches <= 0 {
            return Err(RDataError::BadTextureDirectory);
        }
        let mut patch_lookup: Vec<i32> = vec![0; num_mappatches as usize];
        for i in 0..num_mappatches as usize {
            let name = LumpName::from_bytes(&names[4 + i * 8..4 + i * 8 + 8]);
            patch_lookup[i] = manager.check_num_for_name(&name).map_or(-1, |v| v as i32);
        }

        let t1 = manager
            .check_num_for_name(&LumpName::from_bytes(b"TEXTURE1"))
            .ok_or(RDataError::MissingLump)?;
        let tex1 = manager.lump_data(t1).map_err(|_| RDataError::MissingLump)?;
        let num1 = rd_i32(tex1, 0);
        let num1 = if num1 < 0 { 0 } else { num1 as usize };
        let maxoff1 = tex1.len();
        let has_tex2 = manager.check_num_for_name(&LumpName::from_bytes(b"TEXTURE2"));
        let num2: usize = match has_tex2 {
            Some(idx) => {
                let d = manager
                    .lump_data(idx)
                    .map_err(|_| RDataError::MissingLump)?;
                let n = rd_i32(d, 0);
                let _ = (d, n);
                if n < 0 {
                    0
                } else {
                    n as usize
                }
            }
            None => 0,
        };
        let num_textures = num1 + num2;
        self.num_textures = num_textures;
        self.textures = vec![];
        self.texture_column_lump = vec![vec![]; num_textures];
        self.texture_column_ofs = vec![vec![]; num_textures];
        self.texture_composite = vec![None; num_textures];
        self.texture_composite_size = vec![0; num_textures];
        self.texture_width_mask = vec![0; num_textures];
        self.texture_height = vec![0; num_textures];
        self.texture_translation = (0..num_textures as i32).collect();

        for i in 0..num_textures {
            let in_second = i >= num1;
            // maxoff is the length of the active texture lump.
            let maxoff = if in_second {
                let idx = has_tex2.ok_or(RDataError::BadTextureDirectory)?;
                manager
                    .lump_data(idx)
                    .map_err(|_| RDataError::MissingLump)?
                    .len()
            } else {
                maxoff1
            };
            let (which, dir_off) = if in_second {
                let idx = has_tex2.ok_or(RDataError::BadTextureDirectory)?;
                let d = manager
                    .lump_data(idx)
                    .map_err(|_| RDataError::MissingLump)?;
                (d, 4 + 4 * (i - num1))
            } else {
                (tex1, 4 + 4 * i)
            };
            if dir_off + 4 > which.len() {
                return Err(RDataError::BadTextureDirectory);
            }
            let off = rd_i32(which, dir_off) as usize;
            if off > maxoff {
                return Err(RDataError::BadTextureDirectory);
            }
            let mt = which.get(off..).ok_or(RDataError::BadTextureDirectory)?;
            let t = parse_maptexture(mt, i as i32, &patch_lookup)?;

            let mut mask = 1;
            while (mask * 2) <= t.width {
                mask <<= 1;
            }
            self.texture_width_mask[i] = mask - 1;
            self.texture_height[i] = t.height << 16;
            self.texture_column_lump[i] = vec![0i16; t.width as usize];
            self.texture_column_ofs[i] = vec![0u16; t.width as usize];
            self.textures.push(t);
        }

        for i in 0..num_textures {
            self.generate_lookup(manager, i)?;
        }

        self.generate_texture_hash_table();
        Ok(())
    }

    fn generate_texture_hash_table(&mut self) {
        let n = self.num_textures;
        self.texture_hash = vec![-1; n];
        for i in 0..n {
            let name = &self.textures[i].name;
            let key = (LumpName(*name).hash() % n as u32) as usize;
            self.textures[i].index = i as i32;
            let mut rover = self.texture_hash[key];
            let mut cur: i32 = -1;
            while rover != -1 {
                cur = rover;
                rover = self.textures[cur as usize].next;
            }
            self.textures[i].next = -1;
            if cur == -1 {
                self.texture_hash[key] = i as i32;
            } else {
                self.textures[cur as usize].next = i as i32;
            }
        }
    }

    fn generate_lookup(&mut self, manager: &WadManager, tex: usize) -> Result<()> {
        let width = self.textures[tex].width;
        if width <= 0 {
            return Err(RDataError::ColumnWithoutPatch);
        }
        let w = width as usize;
        self.texture_composite[tex] = None;
        self.texture_composite_size[tex] = 0;

        let patches = self.textures[tex].patches.clone();
        for p in &patches {
            if p.patch <= 0 {
                return Err(RDataError::MissingPatch);
            }
            self.ensure_lump(manager, p.patch as usize)?;
        }

        let mut patch_count: Vec<u8> = vec![0; w];
        for p in &patches {
            let data = self.lump_cache[p.patch as usize].as_ref().unwrap();
            let pwidth = rd_i16(data, 0) as i32;
            let (mut x1, mut x2) = (p.originx, p.originx + pwidth);
            if x1 < 0 {
                x1 = 0;
            }
            if x2 > width {
                x2 = width;
            }
            for x in x1..x2 {
                let xi = x as usize;
                patch_count[xi] += 1;
                let col_start = patch_column_offset(&self.lump_cache, p.patch, x - p.originx);
                self.texture_column_lump[tex][xi] = p.patch as i16;
                self.texture_column_ofs[tex][xi] = (col_start as u32 + 3) as u16;
            }
        }

        let height = self.textures[tex].height as usize;
        for (x, &count) in patch_count.iter().enumerate() {
            if count == 0 {
                return Err(RDataError::ColumnWithoutPatch);
            }
            if count > 1 {
                self.texture_column_lump[tex][x] = -1;
                self.texture_column_ofs[tex][x] = self.texture_composite_size[tex] as u16;
                let cur = self.texture_composite_size[tex] as usize;
                if cur + height > 0x10000 {
                    return Err(RDataError::TextureTooBig);
                }
                self.texture_composite_size[tex] = (cur + height) as i32;
            }
        }
        Ok(())
    }

    fn generate_composite(&mut self, manager: &WadManager, tex: usize) -> Result<()> {
        let patches = self.textures[tex].patches.clone();
        for p in &patches {
            self.ensure_lump(manager, p.patch as usize)?;
        }
        let size = self.texture_composite_size[tex] as usize;
        let mut block: Vec<u8> = vec![0; size];
        let width = self.textures[tex].width;
        let height = self.textures[tex].height;

        // Work on copies so we can return borrows of the caches freely.
        let lumps = self.texture_column_lump[tex].clone();
        let ofs = self.texture_column_ofs[tex].clone();

        for p in &patches {
            let data = self.lump_cache[p.patch as usize].as_ref().unwrap();
            let pwidth = rd_i16(data, 0) as i32;
            let (mut x1, mut x2) = (p.originx, p.originx + pwidth);
            if x1 < 0 {
                x1 = 0;
            }
            if x2 > width {
                x2 = width;
            }
            for x in x1..x2 {
                let xi = x as usize;
                if lumps[xi] >= 0 {
                    continue;
                }
                let col_start = patch_column_offset(&self.lump_cache, p.patch, x - p.originx);
                let real = self.lump_cache[p.patch as usize].as_ref().unwrap();
                self.draw_column_in_cache(
                    real,
                    &mut block,
                    col_start,
                    ofs[xi] as usize,
                    p.originy,
                    height,
                )?;
            }
        }
        self.texture_composite[tex] = Some(block);
        Ok(())
    }

    /// R_DrawColumnInCache: blit posts of a patch column into the composite
    /// cache block starting at `cacheofs`.
    fn draw_column_in_cache(
        &self,
        patch: &[u8],
        cache: &mut [u8],
        column: usize,
        cacheofs: usize,
        originy: i32,
        cacheheight: i32,
    ) -> Result<()> {
        let mut pos = column;
        loop {
            if pos >= patch.len() {
                return Err(RDataError::BadTextureDirectory);
            }
            let topdelta = patch[pos];
            if topdelta == 0xff {
                break;
            }
            if pos + 1 >= patch.len() {
                return Err(RDataError::BadTextureDirectory);
            }
            let length = patch[pos + 1];
            let mut count = length as i32;
            let source = pos + 3;
            let mut position = originy + topdelta as i32;

            if position < 0 {
                count += position;
                position = 0;
            }
            if position + count > cacheheight {
                count = cacheheight - position;
            }
            if count > 0 {
                let end = source + count as usize;
                let dst = cacheofs + position as usize;
                if end > patch.len() || dst + count as usize > cache.len() {
                    return Err(RDataError::BadTextureDirectory);
                }
                cache[dst..dst + count as usize].copy_from_slice(&patch[source..end]);
            }
            pos = pos + length as usize + 4;
        }
        Ok(())
    }

    fn init_flats(&mut self, manager: &WadManager) -> Result<()> {
        let fs = manager
            .check_num_for_name(&LumpName::from_bytes(b"F_START"))
            .ok_or(RDataError::MissingLump)?;
        let fe = manager
            .check_num_for_name(&LumpName::from_bytes(b"F_END"))
            .ok_or(RDataError::MissingLump)?;
        self.first_flat = fs + 1;
        self.last_flat = fe - 1;
        self.num_flats = self.last_flat - self.first_flat + 1;
        let mut tt: Vec<i32> = (0..self.num_flats as i32).collect();
        tt.push(self.num_flats as i32);
        self.flat_translation = tt;
        Ok(())
    }

    fn init_sprite_lumps(&mut self, manager: &WadManager) -> Result<()> {
        let ss = manager
            .check_num_for_name(&LumpName::from_bytes(b"S_START"))
            .ok_or(RDataError::MissingLump)?;
        let se = manager
            .check_num_for_name(&LumpName::from_bytes(b"S_END"))
            .ok_or(RDataError::MissingLump)?;
        self.first_sprite_lump = ss + 1;
        self.last_sprite_lump = se - 1;
        self.num_sprite_lumps = self.last_sprite_lump - self.first_sprite_lump + 1;
        self.sprite_width = vec![0; self.num_sprite_lumps];
        self.sprite_offset = vec![0; self.num_sprite_lumps];
        self.sprite_top_offset = vec![0; self.num_sprite_lumps];
        for i in 0..self.num_sprite_lumps {
            let idx = self.first_sprite_lump + i;
            self.ensure_lump(manager, idx)?;
            let patch = self.lump_cache[idx].as_ref().unwrap();
            self.sprite_width[i] = (rd_i16(patch, 0) as i32) << 16;
            self.sprite_offset[i] = (rd_i16(patch, 4) as i32) << 16;
            self.sprite_top_offset[i] = (rd_i16(patch, 6) as i32) << 16;
        }
        Ok(())
    }

    fn init_colormaps(&mut self, manager: &WadManager) -> Result<()> {
        let lump = manager
            .check_num_for_name(&LumpName::from_bytes(b"COLORMAP"))
            .ok_or(RDataError::MissingLump)?;
        let data = manager
            .lump_data(lump)
            .map_err(|_| RDataError::MissingLump)?;
        self.colormaps = data.to_vec();
        Ok(())
    }

    /// R_FlatNumForName: WAD lump index minus firstflat.
    pub fn flat_num_for_name(&self, manager: &WadManager, name: &LumpName) -> Result<i32> {
        match manager.check_num_for_name(name) {
            Some(idx) => Ok(idx as i32 - self.first_flat as i32),
            None => Err(RDataError::MissingLump),
        }
    }

    /// R_CheckTextureNumForName: texture index or -1.
    pub fn check_texture_num_for_name(&self, name: &LumpName) -> i32 {
        if name.as_bytes()[0] == b'-' {
            return 0;
        }
        let n = self.num_textures;
        if n == 0 {
            return -1;
        }
        let key = (name.hash() % n as u32) as usize;
        let mut idx = self.texture_hash[key];
        while idx != -1 {
            if ascii_case_eq(&self.textures[idx as usize].name, name.as_bytes()) {
                return self.textures[idx as usize].index;
            }
            idx = self.textures[idx as usize].next;
        }
        -1
    }

    /// R_TextureNumForName (Result instead of I_Error).
    pub fn texture_num_for_name(&self, name: &LumpName) -> Result<i32> {
        let i = self.check_texture_num_for_name(name);
        if i == -1 {
            Err(RDataError::MissingLump)
        } else {
            Ok(i)
        }
    }
}

fn parse_maptexture(mt: &[u8], index: i32, patch_lookup: &[i32]) -> Result<Texture> {
    if mt.len() < MAPTEXTURE_HEADER {
        return Err(RDataError::BadTextureDirectory);
    }
    let mut name = [0u8; 8];
    name.copy_from_slice(LumpName::from_bytes(&mt[0..8]).as_bytes());
    let width = rd_i16(mt, 12) as i32;
    let height = rd_i16(mt, 14) as i32;
    let patchcount = rd_i16(mt, 20) as i32;
    if width <= 0 || height <= 0 || patchcount < 0 {
        return Err(RDataError::BadTextureDirectory);
    }
    let mut patches = Vec::with_capacity(patchcount as usize);
    for j in 0..patchcount as usize {
        let o = MAPTEXTURE_HEADER + j * MAPPATCH_SIZE;
        if o + MAPPATCH_SIZE > mt.len() {
            return Err(RDataError::BadTextureDirectory);
        }
        let pname_idx = rd_i16(mt, o + 4) as usize;
        let patch = if pname_idx < patch_lookup.len() {
            patch_lookup[pname_idx]
        } else {
            -1
        };
        if patch == -1 {
            return Err(RDataError::MissingPatch);
        }
        patches.push(TexPatch {
            originx: rd_i16(mt, o) as i32,
            originy: rd_i16(mt, o + 2) as i32,
            patch,
        });
    }
    Ok(Texture {
        name,
        index,
        next: -1,
        width,
        height,
        patchcount,
        patches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wad::Wad;

    fn le16(v: i16) -> [u8; 2] {
        v.to_le_bytes()
    }
    fn le32(v: i32) -> [u8; 4] {
        v.to_le_bytes()
    }

    /// Build a patch lump. Each column is a list of (topdelta, pixels).
    /// Layout matches the format the C walkers expect:
    ///   td, len, pad, pixels(len), pad, next-post...
    fn patch_bytes(size: i16, cols: &[(i16, i16, &[u8])]) -> Vec<u8> {
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&le16(size)); // width
        data.extend_from_slice(&le16(size)); // height
        data.extend_from_slice(&[0, 0]); // leftoffset
        data.extend_from_slice(&[0, 0]); // topoffset
        // columnofs[width]
        let ncols = size.max(0) as usize;
        let ofs_base = data.len();
        data.resize(ofs_base + 4 * ncols, 0);
        let mut ofs_list: Vec<usize> = Vec::new();
        for (td, .., px) in cols {
            ofs_list.push(data.len());
            data.push(*td as u8);
            data.push(px.len() as u8);
            data.push(0); // the `stuff` pad before the pixels
            data.extend_from_slice(px);
            data.push(0); // trailing pad
        }
        let term = data.len();
        data.push(0xff);
        for x in 0..ncols {
            let o = ofs_list.get(x).copied().unwrap_or(term);
            data[ofs_base + 4 * x..ofs_base + 4 * x + 4].copy_from_slice(&(o as u32).to_le_bytes());
        }
        data
    }

    fn wad_new(lumps: &[(&[u8; 8], &[u8])]) -> Vec<u8> {
        let n = lumps.len() as u32;
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"PWAD");
        out.extend_from_slice(&n.to_le_bytes());
        let dir_start: u32 = 12;
        // directory starts right after the header (parse expects dir at infotableofs)
        out.extend_from_slice(&dir_start.to_le_bytes());
        for _ in 0..n {
            out.extend_from_slice(&[0u8; 16]);
        }
        let mut entries: Vec<(usize, [u8; 8])> = Vec::new();
        for (name, data) in lumps {
            entries.push((out.len(), **name));
            out.extend_from_slice(data);
        }
        for (i, (pos, name)) in entries.iter().enumerate() {
            let o = dir_start as usize + i * 16;
            out[o..o + 4].copy_from_slice(&((*pos as u32) - dir_start).to_le_bytes());
            out[o + 8..o + 16].copy_from_slice(name);
        }
        // fix sizes and absolute positions
        for (i, (pos, name)) in entries.iter().enumerate() {
            let o = dir_start as usize + i * 16;
            let size = lumps[i].1.len() as u32;
            out[o..o + 4].copy_from_slice(&(*pos as u32).to_le_bytes());
            out[o + 4..o + 8].copy_from_slice(&size.to_le_bytes());
            out[o + 8..o + 16].copy_from_slice(name);
        }
        out
    }

    fn test_wad() -> Vec<u8> {
        // patch 2x4, both columns identical posts topdelta 0 with pixels.
        let px: Vec<u8> = vec![9, 10, 11, 12];
        let patch = patch_bytes(2, &[(0, 0, &px), (0, 0, &px)]);

        let mut pnames = le32(1).to_vec();
        pnames.extend_from_slice(b"TSTPAT\0\0");

        let mut tex_def: Vec<u8> = Vec::new();
        tex_def.extend_from_slice(b"TSTTEX1\0");
        tex_def.extend_from_slice(&le32(0));
        tex_def.extend_from_slice(&le16(2));
        tex_def.extend_from_slice(&le16(4));
        tex_def.extend_from_slice(&le32(0));
        tex_def.extend_from_slice(&le16(1));
        tex_def.extend_from_slice(&le16(0)); // originx
        tex_def.extend_from_slice(&le16(0)); // originy
        tex_def.extend_from_slice(&le16(0)); // pname idx
        tex_def.extend_from_slice(&le16(0));
        tex_def.extend_from_slice(&le16(0));
        let mut texture1 = le32(1).to_vec();
        texture1.extend_from_slice(&le32(8));
        texture1.extend_from_slice(&tex_def);

        let mut colormap: Vec<u8> = Vec::new();
        for _ in 0..32 {
            for i in 0..256 {
                colormap.push(i as u8);
            }
        }

        let mut flat = Vec::new();
        for y in 0..64 {
            for x in 0..64 {
                flat.push((x + y) as u8);
            }
        }

        wad_new(&[
            (b"PNAMES\0\0", &pnames),
            (b"TEXTURE1", &texture1),
            (b"COLORMAP", &colormap),
            (b"S_START\0", b""),
            (b"TSTPAT\0\0", &patch),
            (b"S_END\0\0\0", b""),
            (b"F_START\0", b""),
            (b"FLAT1\0\0\0", &flat),
            (b"F_END\0\0\0", b""),
        ])
    }

    fn manager() -> (WadManager, Vec<u8>) {
        let w = Wad::parse(test_wad()).unwrap();
        let mut m = WadManager::new();
        m.add(w.data.clone()).unwrap();
        (m, w.data)
    }

    #[test]
    fn init_and_texture_lookup() {
        let (m, _) = manager();
        let rd = RenderData::init_data(&m).unwrap();
        assert_eq!(rd.num_flats, 1);
        assert_eq!(rd.num_sprite_lumps, 1);
        assert_eq!(rd.colormaps.len(), 32 * 256);
        let t = rd
            .texture_num_for_name(&LumpName::from_bytes(b"TSTTEX1\0"))
            .unwrap();
        assert_eq!(t, 0);
        assert_eq!(rd.texture_width_mask[0], 1); // width 2 -> mask 1
        assert_eq!(rd.texture_height[0], 4 << 16);
        assert_eq!(
            rd.check_texture_num_for_name(&LumpName::from_bytes(b"NOPE")),
            -1
        );
        assert_eq!(
            rd.check_texture_num_for_name(&LumpName::from_bytes(b"-??????")),
            0
        );
        assert_eq!(
            rd.flat_num_for_name(&m, &LumpName::from_bytes(b"FLAT1"))
                .unwrap(),
            0
        );
    }

    #[test]
    fn single_patch_column_pixels() {
        let (m, _) = manager();
        let mut rd = RenderData::init_data(&m).unwrap();
        // This texture has 1 patch, columns are single-patch (lump > 0),
        // so the composite stays absent.
        let c0 = rd.column(&m, 0, 0).unwrap()[..4].to_vec();
        let c1 = rd.column(&m, 0, 1).unwrap()[..4].to_vec();
        assert_eq!(c0, [9, 10, 11, 12]);
        assert_eq!(c1, [9, 10, 11, 12]);
        assert!(rd.texture_composite[0].is_none());
    }

    #[test]
    fn composite_texture() {
        // Two patches overlapping: columns get lump == -1 (composite).
        let px2: Vec<u8> = vec![0x11, 0x33, 0x55, 0x77];
        let patch_a = patch_bytes(2, &[(0, 0, &px2), (0, 0, &px2)]);
        let px3: Vec<u8> = vec![0x22, 0x44];
        let patch_b = patch_bytes(2, &[(0, 0, &px3), (0, 0, &px3)]);

        let mut pnames = le32(2).to_vec();
        pnames.extend_from_slice(b"PTA\0\0\0\0\0");
        pnames.extend_from_slice(b"PTB\0\0\0\0\0");

        let mut tex_def: Vec<u8> = Vec::new();
        tex_def.extend_from_slice(b"TSTTEX2\0");
        tex_def.extend_from_slice(&le32(0));
        tex_def.extend_from_slice(&le16(2)); // width
        tex_def.extend_from_slice(&le16(4)); // height
        tex_def.extend_from_slice(&le32(0));
        tex_def.extend_from_slice(&le16(2)); // 2 patches
                                             // patch A at (0,0), patch B at (0,2)
        for (px, py, pi) in [(0, 0, 0), (0, 2, 1)] {
            tex_def.extend_from_slice(&le16(px));
            tex_def.extend_from_slice(&le16(py));
            tex_def.extend_from_slice(&le16(pi));
            tex_def.extend_from_slice(&le16(0));
            tex_def.extend_from_slice(&le16(0));
        }
        let mut texture1 = le32(1).to_vec();
        texture1.extend_from_slice(&le32(8));
        texture1.extend_from_slice(&tex_def);

        let mut colormap: Vec<u8> = Vec::new();
        for _ in 0..32 {
            for i in 0..256 {
                colormap.push(i as u8);
            }
        }

        let wad = wad_new(&[
            (b"PNAMES\0\0", &pnames),
            (b"TEXTURE1", &texture1),
            (b"COLORMAP", &colormap),
            (b"S_START\0", b""),
            (b"PTA\0\0\0\0\0", &patch_a),
            (b"PTB\0\0\0\0\0", &patch_b),
            (b"S_END\0\0\0", b""),
            (b"F_START\0", b""),
            (b"FLAT1\0\0\0", &[0u8; 4096]),
            (b"F_END\0\0\0", b""),
        ]);
        let w = Wad::parse(wad).unwrap();
        let mut m = WadManager::new();
        m.add(w.data.clone()).unwrap();
        let mut rd = RenderData::init_data(&m).unwrap();
        assert_eq!(rd.num_textures, 1);
        assert_eq!(rd.texture_column_lump[0], vec![-1, -1]);
        // composite size = 2 columns * height
        assert_eq!(rd.texture_composite_size[0], 8);
        let c0 = rd.column(&m, 0, 0).unwrap()[..4].to_vec();
        // col 0: patch A row0->[11,33,55,77], patch B row2->[22,44]
        // composite column: height 4 strip
        assert_eq!(c0, [0x11, 0x33, 0x22, 0x44]);
        let c1 = rd.column(&m, 0, 1).unwrap()[..4].to_vec();
        assert_eq!(c1, [0x11, 0x33, 0x22, 0x44]);
    }
}
