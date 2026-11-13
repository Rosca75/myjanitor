//! Shared OOXML package rewriting for the three Office optimisers (W3-A):
//! read with `mj_formats::zip`, rewrite parts, repack with `ZipWriter`,
//! keep `docProps/custom.xml` sensitivity labels intact (SAF-08), never
//! convert to 97-2003 formats.
