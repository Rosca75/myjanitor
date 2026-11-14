//! Minimal POSIX tar (ustar + pax `path`/`size` records), written and read in
//! house so `tar.zst` needs only the external `zstd`. Files are streamed in
//! 1 MiB chunks: no size cap and bounded memory. Links are never written
//! (they are listed in the index). The reader is only used to verify our own
//! archives but treats input as hostile (checksums, bounds, no panics).

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use mj_core::hash::Sha256;
use mj_core::{CancelToken, Error, Phase, Progress, ProgressSink, Result};

use super::walk::{Item, Kind};

const BLOCK: usize = 512;
const CHUNK: usize = 1 << 20;
/// Largest size an 11-digit octal field holds.
const OCTAL11_MAX: u64 = 0o77777777777;

fn octal(field: &mut [u8], v: u64) {
    let w = field.len() - 1;
    let s = format!("{v:0w$o}");
    let s = s.as_bytes();
    field[..w].copy_from_slice(&s[s.len() - w..]);
    field[w] = 0;
}

fn pax_record(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut len = body.len() + 1;
    loop {
        let total = len.to_string().len() + body.len();
        if total == len {
            return format!("{len}{body}").into_bytes();
        }
        len = total;
    }
}

/// One 512-byte header. `name` must fit (100 bytes); longer names go in a pax record.
fn header(name: &[u8], mode: u32, size: u64, mtime: i64, typeflag: u8) -> [u8; BLOCK] {
    let mut h = [0u8; BLOCK];
    let n = name.len().min(100);
    h[..n].copy_from_slice(&name[..n]);
    octal(&mut h[100..108], mode as u64);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size.min(OCTAL11_MAX));
    octal(&mut h[136..148], mtime.clamp(0, OCTAL11_MAX as i64) as u64);
    h[156] = typeflag;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    h
}

fn pad(w: &mut impl Write, len: u64) -> std::io::Result<()> {
    let rem = (len % BLOCK as u64) as usize;
    if rem != 0 {
        w.write_all(&[0u8; BLOCK][..BLOCK - rem])?;
    }
    Ok(())
}

fn entry_header(w: &mut impl Write, name: &str, mode: u32, size: u64, mtime: i64, typeflag: u8) -> std::io::Result<()> {
    let mut pax = Vec::new();
    if name.len() > 100 {
        pax.extend(pax_record("path", name));
    }
    if size > OCTAL11_MAX {
        pax.extend(pax_record("size", &size.to_string()));
    }
    if !pax.is_empty() {
        w.write_all(&header(b"././@PaxHeader", 0o644, pax.len() as u64, mtime, b'x'))?;
        w.write_all(&pax)?;
        pad(w, pax.len() as u64)?;
    }
    w.write_all(&header(name.as_bytes(), mode, size, mtime, typeflag))
}

/// Write a tar of `items` to `dst` (must not exist): the index first, then every
/// directory and file. Content is hashed while streaming and must match `hashes`.
pub fn create(
    items: &[Item],
    hashes: &[Option<String>],
    index_bytes: &[u8],
    index_name: &str,
    dst: &Path,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let io = |e| Error::io(dst, e);
    let f = std::fs::OpenOptions::new().write(true).create_new(true).open(dst).map_err(io)?;
    let mut w = BufWriter::new(f);
    let now = mj_core::Timestamp::now().secs();
    entry_header(&mut w, index_name, 0o644, index_bytes.len() as u64, now, b'0').map_err(io)?;
    w.write_all(index_bytes).map_err(io)?;
    pad(&mut w, index_bytes.len() as u64).map_err(io)?;
    let total: u64 = items.iter().map(Item::size).sum();
    let mut done = 0u64;
    let mut buf = vec![0u8; CHUNK];
    for (it, h) in items.iter().zip(hashes) {
        cancel.checkpoint()?;
        let mode = it.mode.unwrap_or(if it.kind == Kind::Dir { 0o755 } else { 0o644 });
        match &it.kind {
            Kind::Link { .. } => {}
            Kind::Dir => entry_header(&mut w, &format!("{}/", it.rel), mode, 0, it.mtime.secs(), b'5').map_err(io)?,
            Kind::File { size } => {
                entry_header(&mut w, &it.rel, mode, *size, it.mtime.secs(), b'0').map_err(io)?;
                let mut src = File::open(&it.abs).map_err(|e| Error::io(&it.abs, e))?;
                let mut sha = Sha256::new();
                let mut n = 0u64;
                loop {
                    cancel.checkpoint()?;
                    let k = src.read(&mut buf).map_err(|e| Error::io(&it.abs, e))?;
                    if k == 0 {
                        break;
                    }
                    n += k as u64;
                    if n > *size {
                        return Err(Error::Changed(it.abs.clone()));
                    }
                    sha.update(&buf[..k]);
                    w.write_all(&buf[..k]).map_err(io)?;
                }
                if n != *size || h.as_deref() != Some(sha.finish_hex().as_str()) {
                    return Err(Error::Changed(it.abs.clone()));
                }
                pad(&mut w, n).map_err(io)?;
                done += n;
                let mut p = Progress::new(Phase::Applying, done, Some(total));
                p.bytes = done;
                progress.report(&p);
            }
        }
    }
    w.write_all(&[0u8; 2 * BLOCK]).map_err(io)?;
    let f = w.into_inner().map_err(|e| Error::io(dst, e.into_error()))?;
    f.sync_all().map_err(io)
}

/// One entry as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub sha256: String,
    /// Content, only for the entry named `keep` in [`read`].
    pub data: Option<Vec<u8>>,
}

fn perr(m: impl Into<String>) -> Error {
    Error::parse("tar", m)
}

fn parse_octal(f: &[u8]) -> Result<u64> {
    let s: Vec<u8> = f.iter().copied().take_while(|b| *b != 0 && *b != b' ').skip_while(|b| *b == b' ').collect();
    if s.is_empty() {
        return Ok(0);
    }
    if !s.iter().all(|b| (b'0'..=b'7').contains(b)) || s.len() > 21 {
        return Err(perr("bad octal field"));
    }
    s.iter()
        .try_fold(0u64, |a, d| a.checked_mul(8).map(|a| a + (d - b'0') as u64))
        .ok_or_else(|| perr("octal overflow"))
}

fn parse_pax(data: &[u8]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let sp = rest.iter().position(|b| *b == b' ').ok_or_else(|| perr("bad pax record"))?;
        let len: usize =
            std::str::from_utf8(&rest[..sp]).ok().and_then(|s| s.parse().ok()).ok_or_else(|| perr("bad pax length"))?;
        if len <= sp + 1 || len > rest.len() || rest[len - 1] != b'\n' {
            return Err(perr("bad pax record"));
        }
        let kv = std::str::from_utf8(&rest[sp + 1..len - 1]).map_err(|_| perr("pax record not UTF-8"))?;
        let (k, v) = kv.split_once('=').ok_or_else(|| perr("bad pax record"))?;
        out.push((k.to_string(), v.to_string()));
        rest = &rest[len..];
    }
    Ok(out)
}

/// Read every entry of a tar file, hashing contents (streamed). The entry named
/// `keep` is also returned whole (capped at 1 GiB).
pub fn read(path: &Path, keep: &str, cancel: &CancelToken) -> Result<Vec<TarEntry>> {
    let mut r = BufReader::new(File::open(path).map_err(|e| Error::io(path, e))?);
    let mut out = Vec::new();
    let mut pending_path: Option<String> = None;
    let mut pending_size: Option<u64> = None;
    let mut h = [0u8; BLOCK];
    let mut buf = vec![0u8; CHUNK];
    loop {
        cancel.checkpoint()?;
        r.read_exact(&mut h).map_err(|_| perr("truncated archive (no end marker)"))?;
        if h.iter().all(|b| *b == 0) {
            return Ok(out);
        }
        let stored = parse_octal(&h[148..156])?;
        let mut sum: u64 = h.iter().map(|b| *b as u64).sum();
        sum = sum - h[148..156].iter().map(|b| *b as u64).sum::<u64>() + 8 * 32;
        if sum != stored {
            return Err(perr("header checksum mismatch"));
        }
        let size = pending_size.take().map_or_else(|| parse_octal(&h[124..136]), Ok)?;
        let ty = h[156];
        let raw_name = {
            let n = h[..100].iter().position(|b| *b == 0).unwrap_or(100);
            let p = h[345..500].iter().position(|b| *b == 0).unwrap_or(155);
            let base = String::from_utf8_lossy(&h[..n]).into_owned();
            if &h[257..262] == b"ustar" && p > 0 {
                format!("{}/{}", String::from_utf8_lossy(&h[345..345 + p]), base)
            } else {
                base
            }
        };
        let blocks_len = size.div_ceil(BLOCK as u64) * BLOCK as u64;
        let mut sha = Sha256::new();
        let keep_this = matches!(ty, b'0' | 0) && pending_path.as_deref().unwrap_or(&raw_name) == keep;
        let small_meta = matches!(ty, b'x' | b'g' | b'L');
        if small_meta && size > 1 << 20 {
            return Err(perr("oversized metadata record"));
        }
        let mut kept = Vec::new();
        let mut left = blocks_len;
        let mut content_left = size;
        while left > 0 {
            let k = left.min(CHUNK as u64) as usize;
            r.read_exact(&mut buf[..k]).map_err(|_| perr("truncated entry"))?;
            let c = content_left.min(k as u64) as usize;
            sha.update(&buf[..c]);
            if keep_this || small_meta {
                if kept.len() + c > 1 << 30 {
                    return Err(perr("entry too large to keep"));
                }
                kept.extend_from_slice(&buf[..c]);
            }
            content_left -= c as u64;
            left -= k as u64;
        }
        match ty {
            b'x' => {
                for (k, v) in parse_pax(&kept)? {
                    match k.as_str() {
                        "path" => pending_path = Some(v),
                        "size" => pending_size = Some(v.parse().map_err(|_| perr("bad pax size"))?),
                        _ => {}
                    }
                }
            }
            b'L' => pending_path = Some(String::from_utf8_lossy(&kept).trim_end_matches('\0').to_string()),
            b'g' => {}
            b'0' | 0 | b'5' => {
                let name = pending_path.take().unwrap_or(raw_name);
                out.push(TarEntry {
                    is_dir: ty == b'5',
                    name,
                    size,
                    sha256: sha.finish_hex(),
                    data: keep_this.then_some(kept),
                });
            }
            other => return Err(perr(format!("unexpected entry type {}", other as char))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pax_lengths_are_self_consistent() {
        let r = pax_record("path", "a");
        assert_eq!(r, b"9 path=a\n");
        let long = "x".repeat(300);
        let r = pax_record("path", &long);
        let len: usize = std::str::from_utf8(&r[..3]).unwrap().parse().unwrap();
        assert_eq!(len, r.len());
        assert_eq!(parse_pax(&r).unwrap(), vec![("path".into(), long)]);
    }

    #[test]
    fn header_checksum_and_octal() {
        let h = header(b"a.txt", 0o644, 5, 10, b'0');
        assert_eq!(parse_octal(&h[124..136]).unwrap(), 5);
        assert_eq!(parse_octal(&h[136..148]).unwrap(), 10);
        let sum: u64 =
            h.iter().map(|b| *b as u64).sum::<u64>() - h[148..156].iter().map(|b| *b as u64).sum::<u64>() + 256;
        assert_eq!(parse_octal(&h[148..156]).unwrap(), sum);
    }
}
