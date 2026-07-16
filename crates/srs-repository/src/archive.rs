// Archive module — ZipStore implementation tracked in srs-rust#276.
// This file anchors the `zip` workspace dependency and will hold archive
// pack/unpack logic once the attachments RFC (srs#101) is accepted.
//
// Note: `zip` is declared with `default-features = false` (wasm32 constraint, ADR-013).
// This disables the `time` feature; ZipWriter entries will use epoch timestamps
// unless a cross-platform timestamp strategy is added in srs-rust#276.
