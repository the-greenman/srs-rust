/// Library surface for srs-cli.
///
/// The primary artifact is the `srs` binary (`src/main.rs`).
/// This library target exists solely to make `payload` accessible to
/// auxiliary binaries (e.g. `generate-schemas`) and integration tests.
pub mod payload;
