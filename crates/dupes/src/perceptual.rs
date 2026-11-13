//! Perceptual hashing and grouping (W2-B): `gray_grid` box averaging, dHash
//! (9x8), pHash (32x32 DCT, 63 coefficients, 0 clamped to 1), BK-tree,
//! Union-Find with rank, leader refinement (scan limit 256), diameter
//! confidence. Thresholds: `ThresholdPercent` (UI, capped at 18) converted once
//! to `HammingBits`. Owner: W2-B.
