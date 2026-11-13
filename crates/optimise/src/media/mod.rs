//! Media optimisers through external ffmpeg (W3-C). Shared process
//! handling is in `crate::external`; the pieces every media optimiser
//! needs (running ffmpeg with progress, parsing its text output) are in
//! [`ffmpeg`], the estimate arithmetic in [`estimate`].

pub mod audio;
pub mod estimate;
pub mod ffmpeg;
pub mod screen;
pub mod video;
