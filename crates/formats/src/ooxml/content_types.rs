//! `[Content_Types].xml`: defaults by extension and overrides by part name.

use std::ops::Range;

use mj_core::Result;

use super::tree::Tree;
use crate::xml::{as_utf8, Edits};

/// Name of the content-types part.
pub const PART: &str = "[Content_Types].xml";

/// Parsed content types.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentTypes {
    /// (extension, content type).
    pub defaults: Vec<(String, String)>,
    /// (part name without the leading `/`, content type, element span).
    pub overrides: Vec<(String, String, Range<usize>)>,
}

impl ContentTypes {
    /// Parse the part.
    pub fn parse(bytes: &[u8]) -> Result<ContentTypes> {
        let x = as_utf8(bytes)?;
        let t = Tree::parse(x.text)?;
        let mut ct = ContentTypes::default();
        for (_, e) in t.by_local("Default") {
            if let (Some(ext), Some(ty)) = (e.plain("Extension"), e.plain("ContentType")) {
                ct.defaults.push((ext.to_ascii_lowercase(), ty.to_string()));
            }
        }
        for (_, e) in t.by_local("Override") {
            if let (Some(p), Some(ty)) = (e.plain("PartName"), e.plain("ContentType")) {
                ct.overrides.push((p.trim_start_matches('/').to_string(), ty.to_string(), e.span.clone()));
            }
        }
        Ok(ct)
    }

    /// Content type of a part (override first, then the extension default). Part
    /// names compare case-insensitively (OPC).
    pub fn type_of(&self, part: &str) -> Option<&str> {
        if let Some((_, t, _)) = self.overrides.iter().find(|(p, _, _)| p.eq_ignore_ascii_case(part)) {
            return Some(t);
        }
        let file = part.rsplit('/').next().unwrap_or(part);
        let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
        self.defaults.iter().find(|(e, _)| *e == ext).map(|(_, t)| t.as_str())
    }

    /// True when `part` has its own override.
    pub fn has_override(&self, part: &str) -> bool {
        self.overrides.iter().any(|(p, _, _)| p.eq_ignore_ascii_case(part))
    }
}

/// Remove the overrides of `parts` from the content-types part bytes.
pub fn remove_overrides(bytes: &[u8], parts: &[String]) -> Result<Vec<u8>> {
    let x = as_utf8(bytes)?;
    let ct = ContentTypes::parse(bytes)?;
    let mut e = Edits::new();
    for (p, _, span) in &ct.overrides {
        if parts.iter().any(|q| q.eq_ignore_ascii_case(p)) {
            e.delete(span.clone());
        }
    }
    crate::xml::apply_edits(&x, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_removal() {
        let b = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="PNG" ContentType="image/png"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="slide"/></Types>"#;
        let ct = ContentTypes::parse(b).unwrap();
        assert_eq!(ct.type_of("ppt/media/a.png"), Some("image/png"));
        assert_eq!(ct.type_of("PPT/Slides/Slide1.xml"), Some("slide"));
        assert_eq!(ct.type_of("x.bin"), None);
        assert_eq!(ct.type_of("noext"), None);
        let out = remove_overrides(b, &["ppt/slides/slide1.xml".into()]).unwrap();
        let ct = ContentTypes::parse(&out).unwrap();
        assert!(ct.overrides.is_empty());
        assert_eq!(ct.type_of("ppt/slides/slide1.xml"), Some("application/xml"));
    }
}
