//! A small, bounds-checked reader for the PE (Windows executable) facts the prefix work needs: which kind of
//! image a file is, for which processor, and its version. Nothing here trusts an offset it has not checked
//! against the bytes it was given.

pub const MACHINE_I386: u16 = 0x014c;
pub const MACHINE_AMD64: u16 = 0x8664;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeInfo {
    pub machine: u16,
    pub is_dll: bool,
    /// PE32+ (64-bit optional header).
    pub pe32_plus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X64,
    X86,
    Other,
}

impl PeInfo {
    pub fn arch(&self) -> Arch {
        match self.machine {
            MACHINE_AMD64 => Arch::X64,
            MACHINE_I386 => Arch::X86,
            _ => Arch::Other,
        }
    }
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// The offset of the `PE\0\0` signature, if `d` starts like a PE image.
pub fn pe_offset(d: &[u8]) -> Option<usize> {
    if d.get(..2)? != b"MZ" || d.len() < 0x40 {
        return None;
    }
    let pe = u32_at(d, 0x3c)? as usize;
    // The header sits after the 64-byte DOS header; an offset pointing back into it is not a PE.
    if pe < 0x40 || d.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    Some(pe)
}

/// Parse the headers of `d`. None when it is not a PE image or its headers run past the end.
pub fn parse(d: &[u8]) -> Option<PeInfo> {
    let pe = pe_offset(d)?;
    let coff = pe + 4;
    let machine = u16_at(d, coff)?;
    let size_of_optional = u16_at(d, coff + 16)? as usize;
    let characteristics = u16_at(d, coff + 18)?;
    let opt = coff + 20;
    let mut pe32_plus = false;
    if size_of_optional >= 2 {
        // The whole optional header has to be there for the image to be one.
        d.get(opt..opt.checked_add(size_of_optional)?)?;
        pe32_plus = u16_at(d, opt)? == 0x20b;
    }
    Some(PeInfo { machine, is_dll: characteristics & 0x2000 != 0, pe32_plus })
}

/// A VS_FIXEDFILEINFO file version (major, minor, build, revision), if the image carries a version resource.
pub fn file_version(d: &[u8]) -> Option<(u16, u16, u16, u16)> {
    const SIGNATURE: [u8; 4] = 0xFEEF04BDu32.to_le_bytes();
    let mut from = 0;
    while let Some(rel) = d.get(from..)?.windows(4).position(|w| w == SIGNATURE) {
        let at = from + rel;
        // dwStrucVersion follows the signature; a match without it is just bytes that look alike.
        if u32_at(d, at + 4) == Some(0x0001_0000) {
            let ms = u32_at(d, at + 8)?;
            let ls = u32_at(d, at + 12)?;
            return Some(((ms >> 16) as u16, ms as u16, (ls >> 16) as u16, ls as u16));
        }
        from = at + 1;
    }
    None
}

/// A tiny PE image for tests: a real header layout, no code.
#[cfg(test)]
pub fn test_image(machine: u16, dll: bool, version: Option<(u16, u16, u16, u16)>, filler: usize) -> Vec<u8> {
    let mut d = vec![0u8; 0x40];
    d[0] = b'M';
    d[1] = b'Z';
    d[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    d.resize(0x80, 0);
    d.extend_from_slice(b"PE\0\0");
    d.extend_from_slice(&machine.to_le_bytes());
    d.extend_from_slice(&0u16.to_le_bytes()); // sections
    d.extend_from_slice(&[0u8; 12]); // timestamp, symbol table, symbol count
    d.extend_from_slice(&240u16.to_le_bytes()); // size of optional header
    let ch: u16 = 0x0022 | if dll { 0x2000 } else { 0 };
    d.extend_from_slice(&ch.to_le_bytes());
    let mut opt = vec![0u8; 240];
    let magic: u16 = if machine == MACHINE_AMD64 { 0x20b } else { 0x10b };
    opt[..2].copy_from_slice(&magic.to_le_bytes());
    d.extend_from_slice(&opt);
    d.resize(d.len() + filler, 0x90);
    if let Some((a, b, c, e)) = version {
        d.extend_from_slice(&0xFEEF04BDu32.to_le_bytes());
        d.extend_from_slice(&0x0001_0000u32.to_le_bytes());
        d.extend_from_slice(&(((a as u32) << 16) | b as u32).to_le_bytes());
        d.extend_from_slice(&(((c as u32) << 16) | e as u32).to_le_bytes());
        d.extend_from_slice(&[0u8; 32]);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_kind_arch_and_version() {
        let dll = test_image(MACHINE_AMD64, true, Some((2, 2, 26100, 6106)), 100);
        let info = parse(&dll).unwrap();
        assert!(info.is_dll && info.pe32_plus);
        assert_eq!(info.arch(), Arch::X64);
        assert_eq!(file_version(&dll), Some((2, 2, 26100, 6106)));
        let exe = test_image(MACHINE_I386, false, None, 10);
        let info = parse(&exe).unwrap();
        assert!(!info.is_dll && !info.pe32_plus);
        assert_eq!(info.arch(), Arch::X86);
        assert_eq!(file_version(&exe), None);
    }

    #[test]
    fn truncated_or_wild_headers_are_not_pe() {
        let img = test_image(MACHINE_AMD64, true, None, 0);
        for cut in [0, 1, 0x3f, 0x40, 0x80, 0x84, 0x90, 0x98, 0x99, img.len() - 1] {
            assert!(parse(&img[..cut]).is_none(), "cut at {cut}");
        }
        let mut wild = img.clone();
        wild[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&wild).is_none());
        let mut backwards = img.clone();
        backwards[0x3c..0x40].copy_from_slice(&0x10u32.to_le_bytes());
        assert!(parse(&backwards).is_none());
        assert!(parse(b"MZ").is_none());
        assert!(parse(b"").is_none());
    }

    #[test]
    fn a_version_needs_its_structure_version() {
        let mut d = test_image(MACHINE_AMD64, true, None, 0);
        d.extend_from_slice(&0xFEEF04BDu32.to_le_bytes());
        d.extend_from_slice(&[0u8; 16]);
        assert_eq!(file_version(&d), None);
    }
}
