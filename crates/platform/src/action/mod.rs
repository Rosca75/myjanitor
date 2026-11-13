//! Write-side platform code. **Owner: W2-E** (except `fs_compress`, owned by W3-B).
//! Only `mj-actions` calls into this module, after its safety checks.

pub mod cloud;
pub mod elevation;
pub mod fs_compress;
pub mod links;
pub mod trash;
