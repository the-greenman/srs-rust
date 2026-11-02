//! One spelling per diagnostic code (srs-rust#1492): kebab-case, no legacy
//! `SRS0NN-` scheme and no aliases. RFC-spelled codes (RFC-022's
//! `LIFECYCLE_*`) keep the RFC's spelling until the RFC is amended.

use std::fs;
use std::path::Path;

fn rs_sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rs_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_legacy_srs0nn_code_scheme_is_emitted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rs_sources(&root.join("srs-repository/src"), &mut files);
    rs_sources(&root.join("srs-core/src"), &mut files);
    let this = Path::new(file!()).file_name().unwrap().to_owned();
    for f in files {
        if f.file_name() == Some(&this) {
            continue;
        }
        let text = fs::read_to_string(&f).unwrap();
        assert!(
            !text.contains("SRS038-") && !text.contains("SRS043-"),
            "{} still spells a legacy SRS0NN- diagnostic code",
            f.display()
        );
    }
}

#[test]
fn catalog_codes_are_kebab_case() {
    let src =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/catalog.rs")).unwrap();
    let start = src.find("pub mod codes {").expect("codes module");
    let body = &src[start..];
    let end = body.find("\n}\n").expect("end of codes module");
    for line in body[..end].lines().filter(|l| l.contains("pub const")) {
        let value = line.split('"').nth(1).expect("string value");
        assert!(
            !value.is_empty()
                && value
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "catalog code {value:?} is not kebab-case"
        );
    }
}

#[test]
fn relation_validation_codes_are_kebab_case() {
    use srs_core::validation::relation::RelationValidationCode as C;
    for c in [
        C::E1UnknownRelationType,
        C::E1RetiredRelationType,
        C::E1WriteRejected,
        C::E1Conflict,
        C::E2UnknownEndpoint,
        C::E3Irreflexive,
        C::E4TypeConstraint,
    ] {
        let v = c.as_code();
        assert!(v.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "{v}");
    }
}
