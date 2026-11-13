//! A hardened, minimal reader for Windows Shell Link (`.lnk`) files: only the header and the
//! `LinkInfo` structure's *local base path* are read (MS-SHLLINK 2.1 and 2.3), which is enough to
//! learn where a classic file or folder shortcut points.
//!
//! Hostile input is the normal case: every offset is bounds-checked against the buffer and the
//! declared structure sizes, strings are length-capped, and nothing here can panic or allocate
//! more than the input length. Shortcuts that point at network locations, the shell namespace
//! (no local path) or use a non-ASCII ANSI code page are reported as "no evaluable target" instead
//! of guessing.
//! Owner: W2-D.

/// Largest `.lnk` file worth parsing (real shortcuts are a few KB).
pub const MAX_LNK_BYTES: u64 = 1 << 20;

const HEADER_SIZE: usize = 0x4C;
/// `00021401-0000-0000-C000-000000000046` in file order.
const LINK_CLSID: [u8; 16] =
    [0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];
const HAS_LINK_TARGET_ID_LIST: u32 = 1 << 0;
const HAS_LINK_INFO: u32 = 1 << 1;
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 1 << 0;
const MAX_PATH_CHARS: usize = 32_767;

/// What a shortcut points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LnkTarget {
    /// An absolute local path such as `C:\Users\me\file.txt`.
    Local(String),
    /// Valid shortcut without an evaluable local path (network share, shell item, other code page).
    NotEvaluable(&'static str),
}

fn u16_at(d: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(d.get(at..at.checked_add(2)?)?.try_into().ok()?) as usize)
}

fn u32_at(d: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(d.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize)
}

/// Read a NUL-terminated string at `base + off`, confined to `info` (the LinkInfo bytes).
fn ansi_at(info: &[u8], off: usize) -> Option<Option<String>> {
    let rest = info.get(off..)?;
    let end = rest.iter().position(|&b| b == 0)?;
    let s = &rest[..end];
    if s.len() > MAX_PATH_CHARS {
        return None;
    }
    // A non-ASCII ANSI string needs the machine's code page to decode; refuse to guess.
    Some(if s.is_ascii() { Some(String::from_utf8_lossy(s).into_owned()) } else { None })
}

fn utf16_at(info: &[u8], off: usize) -> Option<String> {
    let rest = info.get(off..)?;
    let mut units: Vec<u16> = Vec::new();
    for pair in rest.chunks_exact(2) {
        let u = u16::from_le_bytes([pair[0], pair[1]]);
        if u == 0 {
            return Some(String::from_utf16_lossy(&units));
        }
        units.push(u);
        if units.len() > MAX_PATH_CHARS {
            return None;
        }
    }
    None // no terminator inside the structure
}

/// True for `X:\...` with no control characters.
fn is_local_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/')
        && !p.chars().any(|c| c.is_control())
}

/// Parse a shortcut. `Err` means the bytes are not a well-formed shell link.
pub fn parse_lnk(d: &[u8]) -> Result<LnkTarget, &'static str> {
    if d.len() < HEADER_SIZE || u32_at(d, 0) != Some(HEADER_SIZE) || d[4..20] != LINK_CLSID {
        return Err("not a Windows shortcut (bad header)");
    }
    let flags = u32_at(d, 0x14).ok_or("truncated header")? as u32;
    let mut at = HEADER_SIZE;
    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        let size = u16_at(d, at).ok_or("truncated target list")?;
        at = at.checked_add(2 + size).filter(|&e| e <= d.len()).ok_or("target list runs past the end")?;
    }
    if flags & HAS_LINK_INFO == 0 {
        return Ok(LnkTarget::NotEvaluable("no LinkInfo (shell item target)"));
    }
    let size = u32_at(d, at).ok_or("truncated LinkInfo")?;
    let end = at.checked_add(size).filter(|&e| e <= d.len() && size >= 0x1C).ok_or("LinkInfo size is invalid")?;
    let info = &d[at..end];
    let header_size = u32_at(info, 4).ok_or("truncated LinkInfo")?;
    if header_size < 0x1C || header_size > info.len() {
        return Err("LinkInfo header size is invalid");
    }
    let info_flags = u32_at(info, 8).ok_or("truncated LinkInfo")? as u32;
    if info_flags & VOLUME_ID_AND_LOCAL_BASE_PATH == 0 {
        return Ok(LnkTarget::NotEvaluable("network location"));
    }
    let base_off = u32_at(info, 16).ok_or("truncated LinkInfo")?;
    let suffix_off = u32_at(info, 24).ok_or("truncated LinkInfo")?;
    // Prefer the UTF-16 copies (LinkInfo header size >= 0x24) when present.
    let (base, suffix) = if header_size >= 0x24 {
        let ub = u32_at(info, 28).ok_or("truncated LinkInfo")?;
        let us = u32_at(info, 32).ok_or("truncated LinkInfo")?;
        if ub != 0 {
            let base = utf16_at(info, ub).ok_or("unreadable local path")?;
            let suffix = if us != 0 { utf16_at(info, us).ok_or("unreadable path suffix")? } else { String::new() };
            (base, suffix)
        } else {
            ansi_pair(info, base_off, suffix_off)?
        }
    } else {
        ansi_pair(info, base_off, suffix_off)?
    };
    let Some(base) = (!base.is_empty()).then_some(base) else {
        return Ok(LnkTarget::NotEvaluable("no local path or non-ASCII code page"));
    };
    let mut full = base;
    if !suffix.is_empty() {
        if !full.ends_with('\\') && !suffix.starts_with('\\') {
            full.push('\\');
        }
        full.push_str(&suffix);
    }
    if is_local_absolute(&full) {
        Ok(LnkTarget::Local(full))
    } else {
        Ok(LnkTarget::NotEvaluable("not a local absolute path"))
    }
}

/// ANSI base path + suffix; an empty base signals "cannot decode".
fn ansi_pair(info: &[u8], base_off: usize, suffix_off: usize) -> Result<(String, String), &'static str> {
    let base = ansi_at(info, base_off).ok_or("local path runs past the structure")?;
    let suffix = if suffix_off != 0 {
        ansi_at(info, suffix_off).ok_or("path suffix runs past the structure")?
    } else {
        Some(String::new())
    };
    match (base, suffix) {
        (Some(b), Some(s)) => Ok((b, s)),
        _ => Ok((String::new(), String::new())),
    }
}

/// Build a minimal valid shortcut for tests (ANSI local base path, optional suffix).
#[cfg(test)]
pub fn build_lnk(base: &str, suffix: &str) -> Vec<u8> {
    let mut d = vec![0u8; HEADER_SIZE];
    d[0..4].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    d[4..20].copy_from_slice(&LINK_CLSID);
    d[0x14..0x18].copy_from_slice(&HAS_LINK_INFO.to_le_bytes());
    let mut info = vec![0u8; 0x1C];
    let base_off = info.len();
    info.extend_from_slice(base.as_bytes());
    info.push(0);
    let suffix_off = info.len();
    info.extend_from_slice(suffix.as_bytes());
    info.push(0);
    let size = info.len() as u32;
    info[0..4].copy_from_slice(&size.to_le_bytes());
    info[4..8].copy_from_slice(&0x1Cu32.to_le_bytes());
    info[8..12].copy_from_slice(&VOLUME_ID_AND_LOCAL_BASE_PATH.to_le_bytes());
    info[16..20].copy_from_slice(&(base_off as u32).to_le_bytes());
    info[24..28].copy_from_slice(&(suffix_off as u32).to_le_bytes());
    d.extend_from_slice(&info);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_base_path_and_suffix() {
        let d = build_lnk("C:\\Users\\me\\Documents", "report.docx");
        assert_eq!(parse_lnk(&d), Ok(LnkTarget::Local("C:\\Users\\me\\Documents\\report.docx".into())));
        let d = build_lnk("D:\\Photos\\a.jpg", "");
        assert_eq!(parse_lnk(&d), Ok(LnkTarget::Local("D:\\Photos\\a.jpg".into())));
    }

    #[test]
    fn network_and_non_local_targets_are_not_evaluable() {
        let mut d = build_lnk("C:\\x", "");
        // Clear the VolumeIDAndLocalBasePath flag: a network (CommonNetworkRelativeLink) target.
        d[HEADER_SIZE + 8] = 0;
        assert!(matches!(parse_lnk(&d), Ok(LnkTarget::NotEvaluable(_))));
        let d = build_lnk("\\\\server\\share\\f", "");
        assert!(matches!(parse_lnk(&d), Ok(LnkTarget::NotEvaluable(_))));
        let d = build_lnk("caf\u{e9}".replace('\u{e9}', "\u{ff}").as_str(), ""); // non-ASCII byte path
        let _ = parse_lnk(&d); // must not panic
    }

    #[test]
    fn utf16_paths_are_preferred() {
        let mut d = build_lnk("C:\\ascii", "");
        // Extend the LinkInfo header to 0x24 and append a UTF-16 base path.
        let li = HEADER_SIZE;
        let mut info = d.split_off(li);
        let mut ext = Vec::new();
        for u in "C:\\caf\u{e9}\\x.txt".encode_utf16() {
            ext.extend_from_slice(&u.to_le_bytes());
        }
        ext.extend_from_slice(&[0, 0]);
        let uni_off = info.len() + 8; // after the 8 extra header bytes inserted below
        let mut head = info[..0x1C].to_vec();
        head.extend_from_slice(&(uni_off as u32).to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
        // Shift the ANSI offsets by 8 (they point after the old 0x1C-byte header).
        for at in [16usize, 24] {
            let v = u32::from_le_bytes(head[at..at + 4].try_into().unwrap()) + 8;
            head[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        head[4..8].copy_from_slice(&0x24u32.to_le_bytes());
        let mut rebuilt = head;
        rebuilt.extend_from_slice(&info.split_off(0x1C));
        rebuilt.extend_from_slice(&ext);
        let size = rebuilt.len() as u32;
        rebuilt[0..4].copy_from_slice(&size.to_le_bytes());
        d.extend_from_slice(&rebuilt);
        assert_eq!(parse_lnk(&d), Ok(LnkTarget::Local("C:\\caf\u{e9}\\x.txt".into())));
    }

    #[test]
    fn garbage_and_truncations_never_panic() {
        let good = build_lnk("C:\\Users\\me\\Documents", "report.docx");
        for cut in 0..good.len() {
            let _ = parse_lnk(&good[..cut]);
        }
        let mut x = 0xdead_beefu32;
        for round in 0..400 {
            let mut v = good.clone();
            for _ in 0..(1 + round % 6) {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let i = (x >> 8) as usize % v.len();
                v[i] = (x >> 24) as u8;
            }
            let _ = parse_lnk(&v);
        }
        assert!(parse_lnk(&[]).is_err());
        assert!(parse_lnk(&[0xff; 200]).is_err());
        assert!(parse_lnk(b"not a shortcut").is_err());
    }

    #[test]
    fn oversized_offsets_are_rejected() {
        let mut d = build_lnk("C:\\a", "");
        d[HEADER_SIZE + 16..HEADER_SIZE + 20].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
        assert!(parse_lnk(&d).is_err());
        let mut d = build_lnk("C:\\a", "");
        d[HEADER_SIZE..HEADER_SIZE + 4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        assert!(parse_lnk(&d).is_err());
    }
}
