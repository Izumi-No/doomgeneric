//! Map/level loading, port of `p_setup.c`.
//!
//! The C code keeps interconnected objects behind pointers; here we keep an
//! arena of `Vec`s and reference objects by index. `Level::load` mirrors the
//! lump parsing order of `P_SetupLevel` and produces geometry in the same
//! state the play/render modules expect.

use alloc::vec::Vec;

use crate::doomdef::*;
use crate::fixed::fixed_div;
use crate::wad::{LumpName, WadError, WadManager};

#[derive(Clone, Copy, Debug, Default)]
pub struct Vertex {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Sector {
    pub floorheight: i32,
    pub ceilingheight: i32,
    pub floorpic: LumpName,
    pub ceilingpic: LumpName,
    pub lightlevel: i16,
    pub special: i16,
    pub tag: i16,
    /// Lines bordering this sector (`P_GroupLines`).
    pub lines: Vec<u32>,
    pub soundorg: (i32, i32),
    /// `validcount` for traversals (`P_RecursiveSound`/…).
    pub validcount: i32,
    /// `0`, `1`, `2`, `-1`: sectors crossed by a sound (`P_RecursiveSound`).
    pub soundtraversed: i32,
    /// Thing that made the sound (`P_CheckMeleeRange` targets).
    pub soundtarget: Option<usize>,
    /// Thinker running a special on this sector (`sector_t::specialdata`).
    pub specialdata: Option<crate::p_tick::ThinkerId>,
}

impl Sector {
    /// `sector_t::special` field for the map special number.
    pub fn special_num(&self) -> i16 {
        self.special
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Side {
    pub textureoffset: i32,
    pub rowoffset: i32,
    pub toptexture: LumpName,
    pub bottomtexture: LumpName,
    pub midtexture: LumpName,
    /// Front sector index.
    pub sector: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SlopeType {
    #[default]
    Horizontal,
    Vertical,
    Positive,
    Negative,
}

impl SlopeType {
    pub(crate) fn from_vectors(dx: i32, dy: i32) -> Self {
        if dx == 0 {
            Self::Vertical
        } else if dy == 0 {
            Self::Horizontal
        } else if fixed_div(dy, dx) > 0 {
            Self::Positive
        } else {
            Self::Negative
        }
    }
}

pub const BBOX_LEFT: usize = 0;
pub const BBOX_RIGHT: usize = 1;
pub const BBOX_BOTTOM: usize = 2;
pub const BBOX_TOP: usize = 3;

#[derive(Clone, Copy, Debug, Default)]
pub struct Line {
    pub v1: u32,
    pub v2: u32,
    pub dx: i32,
    pub dy: i32,
    pub flags: u16,
    pub special: i16,
    pub tag: i16,
    pub sidenum: [i16; 2],
    pub bbox: [i32; 4],
    pub slopetype: SlopeType,
    pub frontsector: Option<usize>,
    pub backsector: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Subsector {
    /// Filled during load from the first seg's front side.
    pub sector: usize,
    pub numlines: i16,
    pub firstline: i16,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Node {
    pub x: i32,
    pub y: i32,
    pub dx: i32,
    pub dy: i32,
    pub children: [u16; 2],
    pub bbox: [[i32; 4]; 2],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Seg {
    pub v1: u32,
    pub v2: u32,
    pub offset: i32,
    pub angle: u32,
    pub linedef: u32,
    pub side: u16,
    pub frontsector: usize,
    pub backsector: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Thing {
    pub x: i16,
    pub y: i16,
    pub angle: i16,
    pub ty: i16,
    pub options: i16,
}

#[derive(Clone, Debug, Default)]
pub struct Blockmap {
    pub orgx: i32,
    pub orgy: i32,
    pub width: i32,
    pub height: i32,
    /// 16-bit lists as stored: [bmaporgx, bmaporgy, width, height] then per
    /// block a count followed by line indexes terminated by 0xffff.
    pub data: Vec<i16>,
}

#[derive(Clone, Debug, Default)]
pub struct Level {
    pub vertexes: Vec<Vertex>,
    pub sectors: Vec<Sector>,
    pub sides: Vec<Side>,
    pub lines: Vec<Line>,
    pub subsectors: Vec<Subsector>,
    pub nodes: Vec<Node>,
    pub segs: Vec<Seg>,
    pub blockmap: Blockmap,
    pub reject: Vec<u8>,
    pub things: Vec<Thing>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LevelError {
    Wad(WadError),
    TruncatedLump,
}

impl From<WadError> for LevelError {
    fn from(e: WadError) -> Self {
        Self::Wad(e)
    }
}

fn i16u(d: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([d[i], d[i + 1]])
}

/// Vanilla Doom constants used by `P_GroupLines`.
const MAX_RADIUS: i32 = 32 << 16;
const MAP_BLOCK_SHIFT: u32 = 23;

impl Level {
    pub fn load(
        manager: &WadManager,
        map_lump: usize,
        commercial: bool,
    ) -> Result<Self, LevelError> {
        // note: most of this ordering is important (see P_SetupLevel)
        let vertexes = load_vertexes(manager.lump_data(map_lump + ML_VERTEXES)?)?;
        let mut sectors = load_sectors(manager.lump_data(map_lump + ML_SECTORS)?)?;
        let sides = load_sides(manager.lump_data(map_lump + ML_SIDEDEFS)?)?;
        let lines = load_linedefs(
            manager.lump_data(map_lump + ML_LINEDEFS)?,
            &vertexes,
            &sides,
        )?;
        let mut subsectors = load_subsectors(manager.lump_data(map_lump + ML_SSECTORS)?)?;
        let nodes = load_nodes(manager.lump_data(map_lump + ML_NODES)?)?;
        let segs = load_segs(manager.lump_data(map_lump + ML_SEGS)?, &lines, &sides)?;
        resolve_subsector_sectors(&mut subsectors, &segs);
        let blockmap = load_blockmap(manager.lump_data(map_lump + ML_BLOCKMAP)?)?;
        let totallines: u32 = lines
            .iter()
            .map(|l| {
                let mut n = 0;
                if l.frontsector.is_some() {
                    n += 1;
                }
                if l.backsector.is_some() && l.backsector != l.frontsector {
                    n += 1;
                }
                n
            })
            .sum();
        let reject = load_reject(
            manager.lump_data(map_lump + ML_REJECT)?,
            sectors.len(),
            totallines,
        );
        let things = load_things(manager.lump_data(map_lump + ML_THINGS)?, commercial)?;

        group_lines(&mut sectors, &lines, &vertexes, &blockmap);

        Ok(Self {
            vertexes,
            sectors,
            sides,
            lines,
            subsectors,
            nodes,
            segs,
            blockmap,
            reject,
            things,
        })
    }
}

fn load_vertexes(lump: &[u8]) -> Result<Vec<Vertex>, LevelError> {
    if !lump.len().is_multiple_of(4) {
        return Err(LevelError::TruncatedLump);
    }
    let n = lump.len() / 4;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = i * 4;
        out.push(Vertex {
            x: (i16u(lump, b) as i16 as i32) << 16,
            y: (i16u(lump, b + 2) as i16 as i32) << 16,
        });
    }
    Ok(out)
}

fn load_sectors(lump: &[u8]) -> Result<Vec<Sector>, LevelError> {
    if !lump.len().is_multiple_of(26) {
        return Err(LevelError::TruncatedLump);
    }
    let n = lump.len() / 26;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = i * 26;
        out.push(Sector {
            floorheight: (i16u(lump, b) as i16 as i32) << 16,
            ceilingheight: (i16u(lump, b + 2) as i16 as i32) << 16,
            floorpic: LumpName::from_bytes(&lump[b + 4..b + 12]),
            ceilingpic: LumpName::from_bytes(&lump[b + 12..b + 20]),
            lightlevel: i16u(lump, b + 20) as i16,
            special: i16u(lump, b + 22) as i16,
            tag: i16u(lump, b + 24) as i16,
            ..Sector::default()
        });
    }
    Ok(out)
}

fn load_sides(lump: &[u8]) -> Result<Vec<Side>, LevelError> {
    if !lump.len().is_multiple_of(30) {
        return Err(LevelError::TruncatedLump);
    }
    let n = lump.len() / 30;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = i * 30;
        out.push(Side {
            textureoffset: (i16u(lump, b) as i16 as i32) << 16,
            rowoffset: (i16u(lump, b + 2) as i16 as i32) << 16,
            toptexture: LumpName::from_bytes(&lump[b + 4..b + 12]),
            bottomtexture: LumpName::from_bytes(&lump[b + 12..b + 20]),
            midtexture: LumpName::from_bytes(&lump[b + 20..b + 28]),
            sector: i16u(lump, b + 28) as usize,
        });
    }
    Ok(out)
}

fn load_linedefs(
    lump: &[u8],
    vertexes: &[Vertex],
    sides: &[Side],
) -> Result<Vec<Line>, LevelError> {
    if !lump.len().is_multiple_of(14) {
        return Err(LevelError::TruncatedLump);
    }
    let n = lump.len() / 14;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = i * 14;
        let v1i = i16u(lump, b) as u32;
        let v2i = i16u(lump, b + 2) as u32;
        let v1 = vertexes.get(v1i as usize).copied().unwrap_or_default();
        let v2 = vertexes.get(v2i as usize).copied().unwrap_or_default();
        let dx = v2.x - v1.x;
        let dy = v2.y - v1.y;
        let mut bbox = [0i32; 4];
        if dx >= 0 {
            bbox[BBOX_LEFT] = v1.x;
            bbox[BBOX_RIGHT] = v2.x;
        } else {
            bbox[BBOX_LEFT] = v2.x;
            bbox[BBOX_RIGHT] = v1.x;
        }
        if dy >= 0 {
            bbox[BBOX_BOTTOM] = v1.y;
            bbox[BBOX_TOP] = v2.y;
        } else {
            bbox[BBOX_BOTTOM] = v2.y;
            bbox[BBOX_TOP] = v1.y;
        }

        let sn0 = i16u(lump, b + 10) as i16;
        let sn1 = i16u(lump, b + 12) as i16;
        let frontsector = (sn0 >= 0).then(|| sides[sn0 as usize].sector);
        let backsector = (sn1 >= 0).then(|| sides[sn1 as usize].sector);

        out.push(Line {
            v1: v1i,
            v2: v2i,
            dx,
            dy,
            flags: i16u(lump, b + 4),
            special: i16u(lump, b + 6) as i16,
            tag: i16u(lump, b + 8) as i16,
            sidenum: [sn0, sn1],
            bbox,
            slopetype: SlopeType::from_vectors(dx, dy),
            frontsector,
            backsector,
        });
    }
    Ok(out)
}

fn load_subsectors(lump: &[u8]) -> Result<Vec<Subsector>, LevelError> {
    if !lump.len().is_multiple_of(4) {
        return Err(LevelError::TruncatedLump);
    }
    let n = lump.len() / 4;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = i * 4;
        out.push(Subsector {
            numlines: i16u(lump, b) as i16,
            firstline: i16u(lump, b + 2) as i16,
            ..Subsector::default()
        });
    }
    Ok(out)
}

fn load_nodes(lump: &[u8]) -> Result<Vec<Node>, LevelError> {
    if !lump.len().is_multiple_of(28) {
        return Err(LevelError::TruncatedLump);
    }
    let n = lump.len() / 28;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = i * 28;
        let mut bbox = [[0i32; 4]; 2];
        for (j, bb) in bbox.iter_mut().enumerate() {
            for (k, cell) in bb.iter_mut().enumerate() {
                *cell = (i16u(lump, b + 8 + j * 8 + k * 2) as i16 as i32) << 16;
            }
        }
        out.push(Node {
            x: (i16u(lump, b) as i16 as i32) << 16,
            y: (i16u(lump, b + 2) as i16 as i32) << 16,
            dx: (i16u(lump, b + 4) as i16 as i32) << 16,
            dy: (i16u(lump, b + 6) as i16 as i32) << 16,
            children: [i16u(lump, b + 24), i16u(lump, b + 26)],
            bbox,
        });
    }
    Ok(out)
}

fn load_segs(lump: &[u8], lines: &[Line], sides: &[Side]) -> Result<Vec<Seg>, LevelError> {
    if !lump.len().is_multiple_of(12) {
        return Err(LevelError::TruncatedLump);
    }
    let n = lump.len() / 12;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = i * 12;
        let v1 = i16u(lump, b) as u32;
        let v2 = i16u(lump, b + 2) as u32;
        let angle = (i16u(lump, b + 4) as u32) << 16;
        let offset = (i16u(lump, b + 6) as i16 as i32) << 16;
        let linedef = i16u(lump, b + 8) as u32;
        // Some rebuilt WADs carry garbage in the seg side byte (miniseg-style
        // entries); only the low bit is meaningful.
        let side = i16u(lump, b + 10) & 1;

        let ldef = lines.get(linedef as usize).copied().unwrap_or_default();
        let sidenum = ldef.sidenum[side as usize];
        let front = if sidenum >= 0 {
            sides[sidenum as usize].sector
        } else {
            0
        };
        let mut backsector = None;
        if ldef.flags & ML_TWOSIDED != 0 {
            let back = ldef.sidenum[(side as usize) ^ 1];
            if back >= 0 {
                backsector = Some(sides[back as usize].sector);
            }
        }

        out.push(Seg {
            v1,
            v2,
            offset,
            angle,
            linedef,
            side,
            frontsector: front,
            backsector,
        });
    }
    Ok(out)
}

fn resolve_subsector_sectors(subsectors: &mut [Subsector], segs: &[Seg]) {
    for ss in subsectors.iter_mut() {
        if let Some(first) = segs.get(ss.firstline as usize) {
            ss.sector = first.frontsector;
        }
    }
}

fn load_blockmap(lump: &[u8]) -> Result<Blockmap, LevelError> {
    if !lump.len().is_multiple_of(2) {
        return Err(LevelError::TruncatedLump);
    }
    if lump.len() < 8 {
        return Ok(Blockmap::default());
    }
    let mut data = Vec::with_capacity(lump.len() / 2);
    for i in 0..lump.len() / 2 {
        data.push(i16u(lump, i * 2) as i16);
    }
    Ok(Blockmap {
        orgx: (data[0] as i32) << 16,
        orgy: (data[1] as i32) << 16,
        width: data[2] as i32,
        height: data[3] as i32,
        data,
    })
}

/// `P_LoadReject`: load directly when big enough, otherwise pad with the
/// simulated vanilla "overflow" garbage.
fn load_reject(lump: &[u8], numsectors: usize, totallines: u32) -> Vec<u8> {
    let minlength = (numsectors * numsectors).div_ceil(8);
    if lump.len() >= minlength {
        return lump.to_vec();
    }
    let mut out = alloc::vec![0u8; minlength];
    out[..lump.len()].copy_from_slice(lump);
    pad_reject(&mut out[lump.len()..], totallines);
    out
}

fn pad_reject(tail: &mut [u8], totallines: u32) {
    let size = ((totallines * 4 + 3) & !3) + 24;
    let header: [u32; 4] = [size, 0, 50, 0x1d4a11];
    for (i, byte) in tail.iter_mut().enumerate() {
        *byte = ((header[i / 4] >> ((i % 4) * 8)) & 0xff) as u8;
    }
}

const COMMERCIAL_HIDDEN: [i16; 10] = [68, 64, 88, 89, 69, 67, 71, 65, 66, 84];

fn load_things(lump: &[u8], commercial: bool) -> Result<Vec<Thing>, LevelError> {
    if !lump.len().is_multiple_of(10) {
        return Err(LevelError::TruncatedLump);
    }
    let n = lump.len() / 10;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = i * 10;
        let ty = i16u(lump, b + 6) as i16;
        if !commercial && COMMERCIAL_HIDDEN.contains(&ty) {
            break; // vanilla stops processing the rest of the lump here
        }
        out.push(Thing {
            x: i16u(lump, b) as i16,
            y: i16u(lump, b + 2) as i16,
            angle: i16u(lump, b + 4) as i16,
            ty,
            options: i16u(lump, b + 8) as i16,
        });
    }
    Ok(out)
}

/// `P_GroupLines` (minus the subsector->sector pass, already done):
/// per-sector line lists and sector bounding boxes / sound origins.
fn group_lines(sectors: &mut [Sector], lines: &[Line], vertexes: &[Vertex], bmap: &Blockmap) {
    // count lines per sector then size the lists (P_GroupLines)
    let mut counts = alloc::vec![0usize; sectors.len()];
    for line in lines {
        if let Some(f) = line.frontsector {
            counts[f] += 1;
        }
        if let Some(b) = line.backsector {
            if line.frontsector != Some(b) {
                counts[b] += 1;
            }
        }
    }
    for (s, &n) in counts.iter().enumerate() {
        sectors[s].lines = alloc::vec![0u32; n];
    }

    // assign lines to sectors
    let mut cursor = alloc::vec![0usize; sectors.len()];
    for (i, line) in lines.iter().enumerate() {
        if let Some(f) = line.frontsector {
            sectors[f].lines[cursor[f]] = i as u32;
            cursor[f] += 1;
        }
        if let Some(b) = line.backsector {
            if line.frontsector != Some(b) {
                sectors[b].lines[cursor[b]] = i as u32;
                cursor[b] += 1;
            }
        }
    }

    // bounding boxes and sound origins (M_ClearBox + M_AddToBox)
    let mut bbox = [0i32; 4];
    for sector in sectors.iter_mut() {
        bbox[BBOX_LEFT] = i32::MAX;
        bbox[BBOX_RIGHT] = i32::MIN;
        bbox[BBOX_BOTTOM] = i32::MAX;
        bbox[BBOX_TOP] = i32::MIN;
        for &li in &sector.lines {
            let l = &lines[li as usize];
            let (x1, y1) = (vertexes[l.v1 as usize].x, vertexes[l.v1 as usize].y);
            let (x2, y2) = (vertexes[l.v2 as usize].x, vertexes[l.v2 as usize].y);
            add_to_box(&mut bbox, x1, y1);
            add_to_box(&mut bbox, x2, y2);
        }

        // sound org = middle of the bounding box
        sector.soundorg.0 = (bbox[BBOX_RIGHT] + bbox[BBOX_LEFT]) / 2;
        sector.soundorg.1 = (bbox[BBOX_TOP] + bbox[BBOX_BOTTOM]) / 2;

        if bmap.width == 0 || bmap.height == 0 {
            continue;
        }
        // adjust bounding box to map blocks (blockbox; kept as comments since
        // the play code queries block ranges through the blockmap directly)
        let _box_top = (bbox[BBOX_TOP] - bmap.orgy + MAX_RADIUS) >> MAP_BLOCK_SHIFT;
        let _box_bottom = (bbox[BBOX_BOTTOM] - bmap.orgy - MAX_RADIUS) >> MAP_BLOCK_SHIFT;
        let _box_right = (bbox[BBOX_RIGHT] - bmap.orgx + MAX_RADIUS) >> MAP_BLOCK_SHIFT;
        let _box_left = (bbox[BBOX_LEFT] - bmap.orgx - MAX_RADIUS) >> MAP_BLOCK_SHIFT;
    }
}

fn add_to_box(box_: &mut [i32; 4], x: i32, y: i32) {
    if x < box_[BBOX_LEFT] {
        box_[BBOX_LEFT] = x;
    }
    if x > box_[BBOX_RIGHT] {
        box_[BBOX_RIGHT] = x;
    }
    if y < box_[BBOX_BOTTOM] {
        box_[BBOX_BOTTOM] = y;
    }
    if y > box_[BBOX_TOP] {
        box_[BBOX_TOP] = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i16le(v: i16) -> [u8; 2] {
        v.to_le_bytes()
    }

    fn u16le(v: u16) -> [u8; 2] {
        v.to_le_bytes()
    }

    fn build_wad(lumps: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut dir = Vec::new();
        let mut pos = 12usize;
        for (name, payload) in lumps {
            data.extend_from_slice(payload);
            let mut name8 = [b' '; 8];
            for (i, c) in name.bytes().take(8).enumerate() {
                name8[i] = c;
            }
            let mut e = Vec::new();
            e.extend_from_slice(&(pos as u32).to_le_bytes());
            e.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            e.extend_from_slice(&name8);
            dir.push(e);
            pos += payload.len();
        }
        let mut wad = Vec::new();
        wad.extend_from_slice(b"IWAD");
        wad.extend_from_slice(&(lumps.len() as u32).to_le_bytes());
        wad.extend_from_slice(&((12 + data.len()) as u32).to_le_bytes());
        wad.extend_from_slice(&data);
        for e in dir {
            wad.extend_from_slice(&e);
        }
        wad
    }

    fn square_room(commercial: bool) -> Level {
        // 4 vertexes: (0,0) (100,0) (100,100) (0,100)
        let mut vertices = Vec::new();
        for (x, y) in [(0i16, 0i16), (100, 0), (100, 100), (0, 100)] {
            vertices.extend_from_slice(&i16le(x));
            vertices.extend_from_slice(&i16le(y));
        }

        // 1 sector, flat names 8 chars each
        let mut sector = Vec::new();
        sector.extend_from_slice(&i16le(0)); // floorheight
        sector.extend_from_slice(&i16le(128)); // ceilingheight
        sector.extend_from_slice(b"FLOOR4_8");
        sector.extend_from_slice(b"CEIL3_5 ");
        sector.extend_from_slice(&i16le(160)); // lightlevel
        sector.extend_from_slice(&i16le(0)); // special
        sector.extend_from_slice(&i16le(0)); // tag

        // 4 sidedefs all facing sector 0
        let mut sidedefs = Vec::new();
        for _ in 0..4 {
            sidedefs.extend_from_slice(&i16le(0)); // textureoffset
            sidedefs.extend_from_slice(&i16le(0)); // rowoffset
            sidedefs.extend_from_slice(b"DOORRED ");
            sidedefs.extend_from_slice(b"DOORRED ");
            sidedefs.extend_from_slice(b"DOORRED ");
            sidedefs.extend_from_slice(&i16le(0)); // sector index
        }

        // 4 linedefs: square outline, 1-sided
        let edges: [(u16, u16); 4] = [(0, 1), (1, 2), (2, 3), (3, 0)];
        let mut linedefs = Vec::new();
        for (i, (a, b)) in edges.iter().enumerate() {
            linedefs.extend_from_slice(&u16le(*a));
            linedefs.extend_from_slice(&u16le(*b));
            linedefs.extend_from_slice(&u16le(0)); // flags
            linedefs.extend_from_slice(&u16le(0)); // special
            linedefs.extend_from_slice(&u16le(0)); // tag
            linedefs.extend_from_slice(&i16le(i as i16)); // sidenum[0]
            linedefs.extend_from_slice(&i16le(-1)); // sidenum[1]
        }

        // 1 subsector, 4 segs
        let mut subsector = Vec::new();
        subsector.extend_from_slice(&i16le(4)); // numsegs
        subsector.extend_from_slice(&i16le(0)); // firstseg

        // 4 segs: v1,v2, angle (BAMS), offset, linedef idx, side 0
        let mut segs = Vec::new();
        for (i, (a, b)) in edges.iter().enumerate() {
            segs.extend_from_slice(&u16le(*a));
            segs.extend_from_slice(&u16le(*b));
            segs.extend_from_slice(&u16le(0)); // angle
            segs.extend_from_slice(&u16le(0)); // offset
            segs.extend_from_slice(&u16le(i as u16)); // linedef
            segs.extend_from_slice(&u16le(0)); // side
        }

        // empty nodes handled by the canonical lump (NODES, vec![] below)

        // player start + an Arachnotron (type 68, dropped when !commercial)
        let mut things = Vec::new();
        things.extend_from_slice(&i16le(50));
        things.extend_from_slice(&i16le(50));
        things.extend_from_slice(&i16le(0)); // angle
        things.extend_from_slice(&u16le(1)); // type: player 1
        things.extend_from_slice(&i16le(0)); // options
        things.extend_from_slice(&i16le(10));
        things.extend_from_slice(&i16le(10));
        things.extend_from_slice(&i16le(0));
        things.extend_from_slice(&u16le(68)); // Arachnotron
        things.extend_from_slice(&i16le(0));

        // reject: too small on purpose -> triggers padding
        let reject: Vec<u8> = crate::alloc::vec![0u8; 1];

        // blockmap: header + one empty block list
        let mut blockmap = Vec::new();
        blockmap.extend_from_slice(&i16le(0)); // xorg
        blockmap.extend_from_slice(&i16le(0)); // yorg
        blockmap.extend_from_slice(&i16le(1)); // width
        blockmap.extend_from_slice(&i16le(1)); // height
        blockmap.extend_from_slice(&i16le(0)); // count
        blockmap.extend_from_slice(&i16le(-1)); // terminator

        // canonical ML_* order (MAP01 at index 0)
        let lumps = [
            ("MAP01", crate::alloc::vec![]),
            ("THINGS", things),
            ("LINEDEFS", linedefs),
            ("SIDEDEFS", sidedefs),
            ("VERTEXES", vertices),
            ("SEGS", segs),
            ("SSECTORS", subsector),
            ("NODES", Vec::new()),
            ("SECTORS", sector),
            ("REJECT", reject),
            ("BLOCKMAP", blockmap),
        ];
        let mut wm = WadManager::new();
        wm.add(build_wad(&lumps)).unwrap();
        Level::load(&wm, 0, commercial).unwrap()
    }

    #[test]
    fn loads_square_room_shareware() {
        let level = square_room(false);
        assert_eq!(level.vertexes.len(), 4);
        let v = level.vertexes[1];
        assert_eq!(v.x, 100 << 16);
        assert_eq!(v.y, 0);
        assert_eq!(level.sectors.len(), 1);
        assert_eq!(level.sectors[0].ceilingheight, 128 << 16);
        assert_eq!(level.sides.len(), 4);
        assert_eq!(level.lines.len(), 4);
        // every line faces sector 0 with no back side
        for l in &level.lines {
            assert_eq!(l.frontsector, Some(0));
            assert_eq!(l.backsector, None);
            assert_eq!(l.sidenum[1], -1);
        }
        // axis-aligned edges -> horizontal/vertical slopes
        assert_eq!(level.lines[0].slopetype, SlopeType::Horizontal);
        assert_eq!(level.lines[1].slopetype, SlopeType::Vertical);
        // geometry (dx/dy/bbox)
        assert_eq!(level.lines[1].dx, 0);
        assert_eq!(level.lines[1].dy, 100 << 16);
        assert_eq!(level.lines[0].bbox, [0, 100 << 16, 0, 0]);
        // subsector inherits sector from its first seg
        assert_eq!(level.subsectors.len(), 1);
        assert_eq!(level.subsectors[0].sector, 0);
        assert_eq!(level.subsectors[0].numlines, 4);
        // sector line list built by P_GroupLines
        assert_eq!(level.sectors[0].lines.len(), 4);
        assert_eq!(
            level.sectors[0].soundorg,
            ((100 << 16) / 2, (100 << 16) / 2)
        );
        // reject padded for 1 sector
        assert_eq!(level.reject.len(), 1);
        // things: monsters hidden in non-commercial are dropped
        assert_eq!(level.things.len(), 1);
        assert_eq!(level.things[0].ty, 1);
        // blockmap header
        assert_eq!(level.blockmap.width, 1);
        assert_eq!(level.blockmap.height, 1);
        assert_eq!(level.blockmap.orgx, 0);
    }

    #[test]
    fn loads_commercial_things() {
        let level = square_room(true);
        assert_eq!(level.things.len(), 2);
        assert_eq!(level.things[1].ty, 68);
    }

    #[test]
    fn rejected_lump_is_padded() {
        // 8 sectors -> minlength (64+7)/8 = 8; 1-byte lump pads 7 bytes.
        let min = 64usize.div_ceil(8);
        let r = load_reject(&[0u8], 8, 4);
        assert_eq!(r.len(), min);
        assert_eq!(r[0], 0);
        // PadRejectArray header: ((4*4+3)&~3)+24 = 40 = 0x28 in LE bytes
        assert_eq!(r[1], 0x28);
        assert_eq!(r[2], 0);
        assert_eq!(r[3], 0);
        // only 7 pad bytes fit; the rest of the header never lands
        assert_eq!(r[7], 0);
    }
}
