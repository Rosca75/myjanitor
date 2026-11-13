//! Windows WOF compression. Owner: W3-B.

use std::path::Path;

use mj_core::{Error, FsCompressAlgo, Result};

pub fn compress(path: &Path, algo: FsCompressAlgo) -> Result<()> {
    let _ = (path, algo);
    Err(Error::not_implemented("WOF compression (W3-B)"))
}

pub fn decompress(path: &Path) -> Result<()> {
    let _ = path;
    Err(Error::not_implemented("WOF decompression (W3-B)"))
}

pub fn state(path: &Path) -> Result<Option<FsCompressAlgo>> {
    let _ = path;
    Err(Error::not_implemented("WOF state (W3-B)"))
}
