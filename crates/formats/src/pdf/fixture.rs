//! A tiny programmatic PDF builder for tests: objects are given as raw body
//! text so tests can build anything, malformed files included. Not used by
//! the optimiser itself.

use super::filter;

/// An object body (what goes between `n 0 obj` and `endobj`).
#[derive(Debug, Clone)]
pub struct RawObj {
    pub num: u32,
    pub body: Vec<u8>,
    pub is_stream: bool,
}

/// Collects objects and writes them in one of three layouts.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    pub version: String,
    pub objects: Vec<RawObj>,
}

/// A stream body: `<<dict /Length n>>stream\n data \nendstream`.
pub fn stream_body(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut b = format!("<<{dict} /Length {}>>stream\n", data.len()).into_bytes();
    b.extend_from_slice(data);
    b.extend_from_slice(b"\nendstream");
    b
}

/// A FlateDecode stream body at a fast (poor) level, so the pass has something to gain.
pub fn flate_body(dict: &str, data: &[u8]) -> Vec<u8> {
    stream_body(&format!("{dict} /Filter /FlateDecode"), &filter::deflate(data, 1))
}

impl Builder {
    /// Empty builder writing `%PDF-1.4`.
    pub fn new() -> Self {
        Builder { version: "1.4".into(), objects: Vec::new() }
    }

    /// Add a non-stream object.
    pub fn obj(&mut self, num: u32, body: &str) -> &mut Self {
        self.objects.push(RawObj { num, body: body.as_bytes().to_vec(), is_stream: false });
        self
    }

    /// Add a stream object from a full body (see [`stream_body`]).
    pub fn stream(&mut self, num: u32, body: Vec<u8>) -> &mut Self {
        self.objects.push(RawObj { num, body, is_stream: true });
        self
    }

    /// A one-page document: catalog 1, pages 2, page 3, content 4 (uncompressed), font 5.
    pub fn simple_page(content: &[u8]) -> Self {
        let mut b = Builder::new();
        b.obj(1, "<< /Type /Catalog /Pages 2 0 R >>")
            .obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
            .obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>")
            .stream(4, stream_body("", content))
            .obj(5, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        b
    }

    fn header(&self) -> Vec<u8> {
        let mut out = format!("%PDF-{}\n", self.version).into_bytes();
        out.extend_from_slice(b"%\xe2\xe3\xcf\xd3\n");
        out
    }

    fn put(out: &mut Vec<u8>, o: &RawObj) -> u64 {
        let at = out.len() as u64;
        out.extend_from_slice(format!("{} 0 obj\n", o.num).as_bytes());
        out.extend_from_slice(&o.body);
        out.extend_from_slice(b"\nendobj\n");
        at
    }

    /// Classic cross-reference table. `trailer` is the dictionary body (`/Root 1 0 R`).
    pub fn classic(&self, trailer: &str) -> Vec<u8> {
        let mut out = self.header();
        let mut offs = Vec::new();
        for o in &self.objects {
            offs.push((o.num, Self::put(&mut out, o)));
        }
        let size = offs.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for n in 1..size {
            match offs.iter().find(|(m, _)| *m == n) {
                Some((_, off)) => out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes()),
                None => out.extend_from_slice(b"0000000000 00000 f \n"),
            }
        }
        out.extend_from_slice(format!("trailer\n<< /Size {size} {trailer} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        out
    }

    /// Cross-reference stream; with `objstm`, every non-stream object goes into one object stream.
    pub fn xref_stream(&self, trailer: &str, objstm: bool) -> Vec<u8> {
        let mut out = self.header();
        let max = self.objects.iter().map(|o| o.num).max().unwrap_or(0);
        let stm_num = max + 1;
        let xref_num = max + 2;
        let mut entries = vec![(0u8, 0u64, 0u64); xref_num as usize + 1];
        let mut packed = Vec::new();
        for o in &self.objects {
            if objstm && !o.is_stream {
                entries[o.num as usize] = (2, stm_num as u64, packed.len() as u64);
                packed.push(o);
            } else {
                entries[o.num as usize] = (1, Self::put(&mut out, o), 0);
            }
        }
        if objstm {
            let mut head = String::new();
            let mut body = Vec::new();
            for o in &packed {
                head.push_str(&format!("{} {} ", o.num, body.len()));
                body.extend_from_slice(&o.body);
                body.push(b'\n');
            }
            let first = head.len();
            let mut data = head.into_bytes();
            data.extend_from_slice(&body);
            let s = RawObj {
                num: stm_num,
                body: flate_body(&format!("/Type /ObjStm /N {} /First {first}", packed.len()), &data),
                is_stream: true,
            };
            entries[stm_num as usize] = (1, Self::put(&mut out, &s), 0);
        }
        let at = out.len() as u64;
        entries[xref_num as usize] = (1, at, 0);
        // PNG "Up" predictor on 1+4+2 byte rows, as Acrobat writes them.
        let mut rows = Vec::new();
        let mut prev = [0u8; 7];
        for (t, f2, f3) in &entries {
            let mut row = [0u8; 7];
            row[0] = *t;
            row[1..5].copy_from_slice(&(*f2 as u32).to_be_bytes());
            row[5..7].copy_from_slice(&(*f3 as u16).to_be_bytes());
            rows.push(2u8);
            for i in 0..7 {
                rows.push(row[i].wrapping_sub(prev[i]));
            }
            prev = row;
        }
        let x = RawObj {
            num: xref_num,
            body: stream_body(
                &format!(
                    "/Type /XRef /Size {} /W [1 4 2] {trailer} /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >>",
                    xref_num + 1
                ),
                &filter::deflate(&rows, 6),
            ),
            is_stream: true,
        };
        Self::put(&mut out, &x);
        out.extend_from_slice(format!("startxref\n{at}\n%%EOF\n").as_bytes());
        out
    }
}

/// Append a classic incremental update to `base` (whose last xref is at `prev`).
pub fn incremental_update(base: &[u8], objects: &[(u32, &str)], size: u32, trailer: &str) -> Vec<u8> {
    let prev = last_startxref(base);
    let mut out = base.to_vec();
    let mut offs = Vec::new();
    for (n, body) in objects {
        offs.push((*n, out.len()));
        out.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n");
    for (n, off) in &offs {
        out.extend_from_slice(format!("{n} 1\n{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Prev {prev} {trailer} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

/// Offset written after the last `startxref`.
pub fn last_startxref(data: &[u8]) -> u64 {
    let s = String::from_utf8_lossy(data);
    let Some(i) = s.rfind("startxref") else {
        return 0;
    };
    s[i + 9..].split_whitespace().next().and_then(|t| t.parse().ok()).unwrap_or(0)
}
