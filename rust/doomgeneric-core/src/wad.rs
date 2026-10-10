//! WAD loading, port of `w_wad.c`.
//!
//! Parsing is pure (`&[u8]` in, `Wad` out) so it is testable and works on any
//! host; the `FileSystem` seam supplies the bytes.

use alloc::vec::Vec;
use core::fmt;

/// An 8-character lump name, uppercased and space-padded for comparisons.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LumpName(pub [u8; 8]);

impl LumpName {
    pub const fn empty() -> Self {
        Self([b' '; 8])
    }

    pub fn from_bytes(s: &[u8]) -> Self {
        let mut b = [b' '; 8];
        for (i, c) in s.iter().take(8).enumerate() {
            // NUL padding in WAD names is equivalent to space padding.
            if *c == 0 {
                b[i] = b' ';
            } else {
                b[i] = c.to_ascii_uppercase();
            }
        }
        Self(b)
    }

    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// NUL/space-trimmed, original-case lookup string is not preserved.
    pub fn trimmed(&self) -> &[u8] {
        let end = self
            .0
            .iter()
            .position(|c| *c == 0 || *c == b' ')
            .unwrap_or(8);
        &self.0[..end]
    }

    /// djb2 string hash over the trimmed name; `W_LumpNameHash`.
    pub fn hash(&self) -> u32 {
        let mut result: u32 = 5381;
        for &c in self.trimmed() {
            result = ((result << 5) ^ result) ^ u32::from(c);
        }
        result
    }
}

impl fmt::Debug for LumpName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.trimmed())
    }
}

impl Default for LumpName {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Lump {
    pub name: LumpName,
    pub pos: usize,
    pub size: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum WadError {
    BadMagic([u8; 4]),
    TruncatedHeader,
    TruncatedDirectory,
    BadLumpRange,
}

impl fmt::Display for WadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// A single parsed WAD file.
#[derive(Clone, Debug)]
pub struct Wad {
    pub is_iwad: bool,
    pub data: Vec<u8>,
    pub lumps: Vec<Lump>,
}

impl Wad {
    pub fn parse(data: Vec<u8>) -> Result<Self, WadError> {
        if data.len() < 12 {
            return Err(WadError::TruncatedHeader);
        }
        let magic = [data[0], data[1], data[2], data[3]];
        let is_iwad = match &magic {
            b"IWAD" => true,
            b"PWAD" => false,
            _ => return Err(WadError::BadMagic(magic)),
        };
        let numlumps = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let diroffset = u32::from_le_bytes(data[8..12].try_into().unwrap());
        let n = numlumps as usize;
        let dir_end = diroffset as usize + n * 16;
        if dir_end > data.len() {
            return Err(WadError::TruncatedDirectory);
        }

        let mut lumps = Vec::with_capacity(n);
        for i in 0..n {
            let e = diroffset as usize + i * 16;
            let pos = u32::from_le_bytes(data[e..e + 4].try_into().unwrap());
            let size = u32::from_le_bytes(data[e + 4..e + 8].try_into().unwrap());
            let name = LumpName::from_bytes(&data[e + 8..e + 16]);
            lumps.push(Lump {
                name,
                pos: pos as usize,
                size: size as usize,
            });
        }

        Ok(Self {
            is_iwad,
            data,
            lumps,
        })
    }

    pub fn num_lumps(&self) -> usize {
        self.lumps.len()
    }

    /// Borrow a lump's bytes (zero-size lumps yield an empty slice).
    pub fn lump_data(&self, i: usize) -> Result<&[u8], WadError> {
        let l = self.lumps.get(i).ok_or(WadError::BadLumpRange)?;
        let start = l.pos;
        let end = l.pos + l.size;
        if end > self.data.len() {
            return Err(WadError::BadLumpRange);
        }
        Ok(&self.data[start..end])
    }
}

/// A set of WADs merged exactly like `W_AddFile`: later files take
/// precedence for name lookups and the lump numbers are concatenated.
#[derive(Clone, Debug, Default)]
pub struct WadManager {
    pub wads: Vec<Wad>,
}

impl WadManager {
    pub fn new() -> Self {
        Self { wads: Vec::new() }
    }

    pub fn add(&mut self, data: Vec<u8>) -> Result<(), WadError> {
        self.wads.push(Wad::parse(data)?);
        Ok(())
    }

    pub fn num_lumps(&self) -> usize {
        self.wads.iter().map(|w| w.lumps.len()).sum()
    }

    fn find_lump(&self, idx: usize) -> Option<(&Wad, &Lump)> {
        let mut n = idx;
        for w in &self.wads {
            if n < w.lumps.len() {
                return Some((w, &w.lumps[n]));
            }
            n -= w.lumps.len();
        }
        None
    }

    /// `W_CheckNumForName`: highest (latest-file) match wins.
    pub fn check_num_for_name(&self, name: &LumpName) -> Option<usize> {
        let mut flat = self.num_lumps();
        for w in self.wads.iter().rev() {
            for l in w.lumps.iter().rev() {
                flat -= 1;
                if &l.name == name {
                    return Some(flat);
                }
            }
        }
        None
    }

    pub fn get_num_for_name(&self, name: &LumpName) -> Option<usize> {
        self.check_num_for_name(name)
    }

    pub fn lump_length(&self, i: usize) -> Option<usize> {
        self.find_lump(i).map(|(_, l)| l.size)
    }

    /// Name of the flat lump at index `i` (`lumpinfo[i].name`).
    pub fn lump_name(&self, i: usize) -> Option<LumpName> {
        self.find_lump(i).map(|(_, l)| l.name)
    }

    pub fn lump_data(&self, i: usize) -> Result<&[u8], WadError> {
        let (w, l) = self.find_lump(i).ok_or(WadError::BadLumpRange)?;
        let start = l.pos;
        let end = l.pos + l.size;
        if end > w.data.len() {
            return Err(WadError::BadLumpRange);
        }
        Ok(&w.data[start..end])
    }

    pub fn lump_data_by_name(&self, name: &LumpName) -> Result<&[u8], WadError> {
        let i = self.get_num_for_name(name).ok_or(WadError::BadLumpRange)?;
        self.lump_data(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_wad(iwad: bool, lumps: &[(&str, &[u8])]) -> Vec<u8> {
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
        wad.extend_from_slice(if iwad { b"IWAD" } else { b"PWAD" });
        wad.extend_from_slice(&(lumps.len() as u32).to_le_bytes());
        wad.extend_from_slice(&((12 + data.len()) as u32).to_le_bytes());
        wad.extend_from_slice(&data);
        for e in dir {
            wad.extend_from_slice(&e);
        }
        wad
    }

    #[test]
    fn parses_directory_and_data() {
        let bytes = build_wad(true, &[("PLAYPAL", &[1, 2, 3]), ("MAP01", &[9; 8])]);
        let wad = Wad::parse(bytes.clone()).unwrap();
        assert!(wad.is_iwad);
        assert_eq!(wad.num_lumps(), 2);
        assert_eq!(wad.lump_data(0).unwrap(), &[1, 2, 3]);
        assert_eq!(wad.lump_data(1).unwrap(), &[9; 8]);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut b = build_wad(true, &[]);
        b[0] = b'X';
        assert!(matches!(Wad::parse(b), Err(WadError::BadMagic(_))));
    }

    #[test]
    fn name_hash_matches_djb2() {
        let n = LumpName::from_bytes(b"playpal");
        assert_eq!(n.hash(), 0x8d79_cf7c);
    }

    #[test]
    fn later_wad_takes_precedence() {
        let iwad = build_wad(true, &[("PLAYPAL", &[1])]);
        let pwad = build_wad(false, &[("PLAYPAL", &[2])]);
        let mut m = WadManager::new();
        m.add(iwad).unwrap();
        m.add(pwad).unwrap();
        assert_eq!(m.num_lumps(), 2);
        let i = m
            .check_num_for_name(&LumpName::from_bytes(b"PLAYPAL"))
            .unwrap();
        assert_eq!(m.lump_data(i).unwrap(), &[2]);
        assert_eq!(m.lump_data(0).unwrap(), &[1]);
    }

    #[test]
    fn name_match_is_case_insensitive() {
        let bytes = build_wad(true, &[("PLAYPAL", &[7])]);
        let mut m = WadManager::new();
        m.add(bytes).unwrap();
        let i = m
            .get_num_for_name(&LumpName::from_bytes(b"playpal"))
            .unwrap();
        assert_eq!(m.lump_data(i).unwrap(), &[7]);
    }
}
