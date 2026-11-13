//! PDF: tokenizer, object parser, xref/object streams, signature detection
//! (`/ByteRange` + `/Sig`), encryption detection, lossless structural rewrite.
//! Owner: W3-B. (W2-E may detect signed/encrypted PDFs in Wave 2 by scanning
//! for `/Encrypt` and `/ByteRange` in `mj-actions::safety`.)
