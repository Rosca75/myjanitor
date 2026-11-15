//! PDF: tokenizer, object parser, xref/object streams, signature detection
//! (`/ByteRange` + `/Sig`), encryption detection, lossless structural rewrite.
//! Owner: W3-B. (W2-E may detect signed/encrypted PDFs in Wave 2 by scanning
//! for `/Encrypt` and `/ByteRange` in `mj-actions::safety`.)
//!
//! In-house object-level reader and writer (decision D-3B01): no general PDF
//! library is needed for a structural pass, and the hardened subset here
//! (classic and stream cross-references, incremental updates, hybrid files,
//! object streams, FlateDecode with predictors) is fully tested.
//!
//! | module | purpose |
//! |---|---|
//! | [`object`] | value model |
//! | [`lexer`] | tokenizer and direct-object parser |
//! | [`filter`] | FlateDecode, predictors, canonical stream content |
//! | [`document`] | xref chain, indirect objects, object streams, page tree |
//! | [`writer`] | serialiser and single-revision file writer |
//! | [`structural`] | the optimisation pass, refusals, removals, size breakdown |
//! | [`verify`] | output verification against the original |
//! | [`fixture`] | small programmatic PDF builder for tests |

pub mod document;
pub mod filter;
pub mod fixture;
pub mod lexer;
pub mod object;
pub mod structural;
pub mod verify;
pub mod writer;

#[cfg(test)]
mod tests;

pub use document::Document;
pub use structural::{optimise, Breakdown, Options, Outcome, Report, SkipReason};
pub use verify::verify;

/// Cheap check on the first and last bytes of a file: `(is_pdf, encrypted, signed)`.
/// Used by estimates; the pass itself does the authoritative checks.
pub fn quick_flags(head: &[u8], tail: &[u8]) -> (bool, bool, bool) {
    let is_pdf = head.starts_with(b"%PDF-");
    let encrypted = document::contains(tail, b"/Encrypt");
    let signed = document::contains(head, b"/ByteRange")
        || document::contains(tail, b"/ByteRange")
        || document::contains(head, b"/Type/Sig")
        || document::contains(head, b"/Type /Sig");
    (is_pdf, encrypted, signed)
}
