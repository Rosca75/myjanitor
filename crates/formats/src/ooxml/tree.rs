//! A flat element index over one XML part: every element with its full byte
//! span, resolved namespace, attributes and parent, built from the lossless
//! tokenizer. Used for the small structural parts (presentation, masters,
//! relationships, styles, workbook); large sheets are streamed instead.

use std::ops::Range;

use mj_core::Result;

use crate::xml::{Edits, TokenKind, Tokenizer};

/// Relationships namespace (transitional and strict).
pub const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const REL_NS_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";

/// True for the relationships namespace in either conformance class.
pub fn is_rel_ns(ns: Option<&str>) -> bool {
    matches!(ns, Some(REL_NS) | Some(REL_NS_STRICT))
}

/// One attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub ns: Option<String>,
    pub value: String,
    pub value_span: Range<usize>,
    pub span: Range<usize>,
}

/// One element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub local: String,
    pub ns: Option<String>,
    /// From `<` of the start tag to `>` of the end tag (or of the empty tag).
    pub span: Range<usize>,
    /// The start (or empty) tag alone.
    pub start_tag: Range<usize>,
    /// Between the start and end tags (empty for empty tags).
    pub inner: Range<usize>,
    pub attrs: Vec<Attr>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub depth: usize,
    /// Text directly inside (decoded, concatenated).
    pub text: String,
}

impl Element {
    /// Attribute by local name (any namespace, or none).
    pub fn attr(&self, local: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| crate::xml::split_qname(&a.name).1 == local && !a.name.starts_with("xmlns"))
    }

    /// Attribute value by unprefixed name.
    pub fn plain(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_str())
    }

    /// Attributes in the relationships namespace (`r:id`, `r:embed`...).
    pub fn rel_attrs(&self) -> impl Iterator<Item = &Attr> {
        self.attrs.iter().filter(|a| is_rel_ns(a.ns.as_deref()))
    }
}

/// All elements of a document in document order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tree {
    pub elements: Vec<Element>,
}

impl Tree {
    /// Parse `text` (UTF-8, BOM already split off or kept: spans index `text`).
    pub fn parse(text: &str) -> Result<Tree> {
        let mut t = Tokenizer::new(text)?;
        let mut els: Vec<Element> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        while let Some(tok) = t.next_token()? {
            match tok.kind {
                TokenKind::StartTag | TokenKind::EmptyTag => {
                    let ns = t.element_namespace(&tok).map(str::to_string);
                    let attrs = tok
                        .attrs
                        .iter()
                        .map(|a| Attr {
                            name: a.name.to_string(),
                            ns: t.attr_namespace(a).map(str::to_string),
                            value: a.value().into_owned(),
                            value_span: a.value_span.clone(),
                            span: a.span.clone(),
                        })
                        .collect();
                    let idx = els.len();
                    let parent = stack.last().copied();
                    if let Some(p) = parent {
                        els[p].children.push(idx);
                    }
                    let empty = tok.kind == TokenKind::EmptyTag;
                    els.push(Element {
                        name: tok.name.to_string(),
                        local: tok.local_name().to_string(),
                        ns,
                        span: tok.span.clone(),
                        start_tag: tok.span.clone(),
                        inner: tok.span.end..tok.span.end,
                        attrs,
                        parent,
                        children: Vec::new(),
                        depth: tok.depth,
                        text: String::new(),
                    });
                    if !empty {
                        stack.push(idx);
                    }
                }
                TokenKind::EndTag => {
                    if let Some(i) = stack.pop() {
                        els[i].span.end = tok.span.end;
                        els[i].inner.end = tok.span.start;
                    }
                }
                TokenKind::Text | TokenKind::CData => {
                    if let Some(&i) = stack.last() {
                        els[i].text.push_str(&tok.text());
                    }
                }
                _ => {}
            }
        }
        Ok(Tree { elements: els })
    }

    /// The root element.
    pub fn root(&self) -> Option<&Element> {
        self.elements.first()
    }

    /// Elements with this local name, in document order.
    pub fn by_local<'a>(&'a self, local: &'a str) -> impl Iterator<Item = (usize, &'a Element)> + 'a {
        self.elements.iter().enumerate().filter(move |(_, e)| e.local == local)
    }

    /// Element children of `i` with this local name.
    pub fn children_named<'a>(&'a self, i: usize, local: &'a str) -> impl Iterator<Item = usize> + 'a {
        self.elements[i].children.iter().copied().filter(move |&c| self.elements[c].local == local)
    }

    /// Elements carrying a relationships-namespace attribute equal to `rid`.
    pub fn referencing(&self, rid: &str) -> Vec<usize> {
        self.elements
            .iter()
            .enumerate()
            .filter(|(_, e)| e.rel_attrs().any(|a| a.value == rid))
            .map(|(i, _)| i)
            .collect()
    }

    /// Add edits that remove element `i`; when that leaves an `ext` (or then an `extLst`)
    /// without element children, the empty wrapper goes too (an empty `ext` is invalid).
    pub fn remove_element(&self, i: usize, edits: &mut Edits, removed: &mut Vec<usize>) {
        let mut target = i;
        while let Some(p) = self.elements[target].parent {
            let parent = &self.elements[p];
            let others = parent.children.iter().filter(|&&c| c != target && !removed.contains(&c)).count();
            if others == 0 && (parent.local == "ext" || parent.local == "extLst") {
                target = p;
            } else {
                break;
            }
        }
        if removed.iter().any(|&r| self.contains(r, target)) {
            return;
        }
        removed.push(target);
        edits.delete(self.elements[target].span.clone());
    }

    /// True when `outer` is `inner` or one of its ancestors.
    pub fn contains(&self, outer: usize, inner: usize) -> bool {
        let mut x = Some(inner);
        while let Some(i) = x {
            if i == outer {
                return true;
            }
            x = self.elements[i].parent;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_and_removal_with_ext_collapse() {
        let s = format!(
            "<p:sld xmlns:p='urn:p' xmlns:r='{REL_NS}'><p:cSld/><p:extLst><p:ext uri='x'><c:rel xmlns:c='urn:c' r:id='rId3'/></p:ext></p:extLst><a r:embed='rId4'/></p:sld>"
        );
        let t = Tree::parse(&s).unwrap();
        let refs = t.referencing("rId3");
        assert_eq!(refs.len(), 1);
        let mut e = Edits::new();
        let mut removed = Vec::new();
        t.remove_element(refs[0], &mut e, &mut removed);
        let out = e.apply(&s).unwrap();
        assert_eq!(out, format!("<p:sld xmlns:p='urn:p' xmlns:r='{REL_NS}'><p:cSld/><a r:embed='rId4'/></p:sld>"));
        assert_eq!(t.referencing("rId4").len(), 1);
        let root = t.root().unwrap();
        assert_eq!(root.local, "sld");
        assert_eq!(&s[root.span.clone()], s.as_str());
    }
}
