//! Minimal XML pull parser for OOXML parts. Owner: W3-A.
//! Supports elements, attributes, text, CDATA, comments, processing
//! instructions and the predefined entities; no DTDs (rejects `<!DOCTYPE`).

use mj_core::{Error, Result};

/// One parse event. Borrowed from the input where possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlEvent<'a> {
    Start { name: &'a str, attrs: Vec<(&'a str, String)>, self_closing: bool },
    End { name: &'a str },
    Text(String),
    Comment(&'a str),
    ProcessingInstruction(&'a str),
    Eof,
}

/// Pull parser over a UTF-8 document.
pub struct XmlReader<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> XmlReader<'a> {
    /// Parser over `input`.
    pub fn new(input: &'a str) -> Self {
        XmlReader { input, pos: 0 }
    }

    /// Next event.
    pub fn next_event(&mut self) -> Result<XmlEvent<'a>> {
        let _ = (self.input, self.pos);
        Err(Error::not_implemented("XML parser (W3-A)"))
    }
}
