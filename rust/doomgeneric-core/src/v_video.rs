//! Video primitives, port of `v_video.c`.
//!
//! The engine renders into indexed-color byte screens (like vanilla's
//! `screens[]`); presenting to the host is a palette pass
//! ([`present_screen`]). Patches (the WAD's column-encoded images) are
//! decoded straight from the lump bytes, so no copy of the WAD lives here.

use crate::Pixel;
use crate::wad::{LumpName, WadManager};

/// `SCREENWIDTH`/`SCREENHEIGHT`/`ST_HEIGHT`.
pub const SCREENWIDTH: usize = 320;
pub const SCREENHEIGHT: usize = 200;
pub const ST_HEIGHT: usize = 32;

/// Patch header and post decoding. A patch is a header plus one column
/// pointer per `width`; columns are runs of posts
/// (`topdelta, length, pixels..., padding`).
pub struct Patch<'a> {
    pub width: i32,
    pub height: i32,
    pub leftoffset: i32,
    pub topoffset: i32,
    data: &'a [u8],
}

impl Patch<'_> {
    /// Parse the header of a patch lump (malformed lumps are clipped to the
    /// empty patch rather than erroring, matching vanilla's trust).
    pub fn from_lump(data: &[u8]) -> Patch<'_> {
        if data.len() < 8 {
            return Patch {
                width: 0,
                height: 0,
                leftoffset: 0,
                topoffset: 0,
                data: &[],
            };
        }
        let w = i16::from_le_bytes([data[0], data[1]]) as i32;
        let h = i16::from_le_bytes([data[2], data[3]]) as i32;
        let l = i16::from_le_bytes([data[4], data[5]]) as i32;
        let t = i16::from_le_bytes([data[6], data[7]]) as i32;
        let _ = (w, h, l, t);
        Patch {
            width: w,
            height: h,
            leftoffset: l,
            topoffset: t,
            data,
        }
    }
}

/// `V_DrawPatch`: blit `patch` at `(x, y)` (top-left corner) into an indexed
/// screen, clipping to its bounds like the original.
pub fn draw_patch(
    screen: &mut [u8],
    screen_width: i32,
    screen_height: i32,
    patch: &Patch<'_>,
    x: i32,
    y: i32,
) {
    if patch.width == 0 || patch.data.is_empty() {
        return;
    }
    let colx = x - patch.leftoffset;
    let topy = y - patch.topoffset;

    for col in 0..patch.width {
        let x1 = colx + col;
        if x1 < 0 || x1 >= screen_width {
            continue;
        }
        let ofs_base = 8 + 4 * col as usize;
        if ofs_base + 4 > patch.data.len() {
            continue;
        }
        let col_ofs =
            u32::from_le_bytes(patch.data[ofs_base..ofs_base + 4].try_into().unwrap()) as usize;
        if col_ofs >= patch.data.len() {
            continue;
        }

        // Walk the posts of this column.
        let mut p = col_ofs;
        loop {
            if p + 2 > patch.data.len() {
                break;
            }
            let topdelta = patch.data[p] as i32;
            if topdelta == 0xff {
                break;
            }
            let length = patch.data[p + 1] as usize;
            // Post header: topdelta, length, one pad byte, then the pixels
            // (the vanilla `columnofs[x] + 3` convention).
            let pixel_base = p + 3;
            if pixel_base + length > patch.data.len() {
                break;
            }
            for i in 0..length {
                let y1 = topy + topdelta + i as i32;
                if y1 >= 0 && y1 < screen_height {
                    let idx = y1 as usize * screen_width as usize + x1 as usize;
                    if idx < screen.len() {
                        screen[idx] = patch.data[pixel_base + i];
                    }
                }
            }
            p = pixel_base + length + 1; // one pad byte after the pixels
        }
    }
}

/// Load the `PLAYPAL` palette (768 bytes, 256 RGB triples).
pub fn load_palette(manager: &WadManager) -> Option<[u8; 768]> {
    let data = manager
        .lump_data_by_name(&LumpName::from_bytes(b"PLAYPAL"))
        .ok()?;
    let mut out = [0u8; 768];
    if data.len() < 768 {
        return None;
    }
    out.copy_from_slice(&data[..768]);
    Some(out)
}

/// `I_SetPalette` + present: map an indexed screen to host pixels.
pub fn present_screen(screen: &[u8], palette: &[u8; 768], out: &mut [Pixel]) {
    for (i, &idx) in screen.iter().enumerate() {
        let base = idx as usize * 3;
        let r = palette[base] as u32;
        let g = palette[base + 1] as u32;
        let b = palette[base + 2] as u32;
        if let Some(slot) = out.get_mut(i) {
            *slot = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        }
    }
}

/// `V_MarkRect`-style full clear of an indexed screen (`I_FinishUpdate`
/// path uses black).
pub fn clear_screen(screen: &mut [u8], color: u8) {
    screen.fill(color);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1x4 patch: one column with a single post of 4 pixels at (0,0).
    fn tiny_patch() -> alloc::vec::Vec<u8> {
        let mut v = alloc::vec::Vec::new();
        v.extend_from_slice(&1i16.to_le_bytes()); // width
        v.extend_from_slice(&4i16.to_le_bytes()); // height
        v.extend_from_slice(&0i16.to_le_bytes()); // leftoffset
        v.extend_from_slice(&0i16.to_le_bytes()); // topoffset
        v.extend_from_slice(&12u32.to_le_bytes()); // columnofs[0] -> 12
        // Post: topdelta 0, length 4, pad, pixels, pad, terminator.
        v.extend_from_slice(&[0, 4, 0, 10, 11, 12, 13, 0, 0xff]);
        v
    }

    #[test]
    fn draw_patch_blits_and_clips() {
        let data = tiny_patch();
        let patch = Patch::from_lump(&data);
        let mut screen = [0u8; 16];
        draw_patch(&mut screen, 4, 4, &patch, 0, 0);
        // One column wide, four tall: pixels run down the first column.
        assert_eq!(&screen[..4], &[10, 0, 0, 0]);
        assert_eq!(screen[4], 11);
        assert_eq!(screen[8], 12);
        assert_eq!(screen[12], 13);

        // Out-of-bounds draws are fully clipped.
        let mut screen = [9u8; 16];
        draw_patch(&mut screen, 4, 4, &patch, -4, 0);
        assert!(screen.iter().all(|&p| p == 9));
    }

    #[test]
    fn palette_present_maps_rgb() {
        let mut palette = [0u8; 768];
        palette[0..3].copy_from_slice(&[1, 2, 3]);
        palette[3..6].copy_from_slice(&[0xff, 0x80, 0x00]);
        let screen = [0u8, 1, 1, 0];
        let mut out = [0u32; 4];
        present_screen(&screen, &palette, &mut out);
        assert_eq!(out[0], 0xFF01_0203);
        assert_eq!(out[1], 0xFFFF_8000);
    }
}
