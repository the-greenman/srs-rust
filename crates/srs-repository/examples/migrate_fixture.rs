//! Dev tool: rewrite an srs-rust fixture or seed to the RFC-038 final storage
//! format via the real migration services — never by hand (srs-rust#783
//! Phase 6). Directory repositories run the registered chain in order
//! (`field-type` + `rfc039-carrier` when below data-model revision 2, then
//! `rfc038-storage`); `.srsj` documents go through `migrate_srsj`.
//!
//! Usage: cargo run -p srs-repository --example migrate_fixture -- <path>...

use srs_repository::rfc038_storage_migration_service::{
    migrate_srsj, migrate_storage, StorageMigrationOptions,
};
use srs_repository::store::FileStore;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: migrate_fixture <fixture-path>...");
        std::process::exit(2);
    }
    for path in &args {
        if path.ends_with(".srsj") {
            migrate_document(path);
        } else {
            migrate_directory(path);
        }
    }
}

fn migrate_document(path: &str) {
    let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("{path}: read failed: {e}");
        std::process::exit(1);
    });
    let (migrated, result) = migrate_srsj(&content).unwrap_or_else(|e| {
        eprintln!("{path}: migrate_srsj failed: {e}");
        std::process::exit(1);
    });
    std::fs::write(path, &migrated).unwrap_or_else(|e| {
        eprintln!("{path}: write failed: {e}");
        std::process::exit(1);
    });
    println!(
        "{path}: relations exploded {}, properties stripped {:?}, version bumped {}",
        result.relations_exploded, result.manifest_properties_stripped, result.srsj_version_bumped
    );
}

fn migrate_directory(path: &str) {
    let store = FileStore::new(path);
    let revision = srs_repository::field_type_migration_service::data_model_revision(&store)
        .unwrap_or_else(|e| {
            eprintln!("{path}: cannot read data-model revision: {e}");
            std::process::exit(1);
        });
    if revision < 2 {
        // Pre-RFC-039 fixture: bring it to the carrier generation first —
        // `rfc038-storage` refuses anything below revision 2.
        srs_repository::field_type_migration_service::migrate_field_types(&store).unwrap_or_else(
            |e| {
                eprintln!("{path}: field-type migration failed: {e}");
                std::process::exit(1);
            },
        );
        srs_repository::rfc039_carrier_migration_service::migrate_carrier(&store).unwrap_or_else(
            |e| {
                eprintln!("{path}: rfc039-carrier migration failed: {e}");
                std::process::exit(1);
            },
        );
    }
    let result = migrate_storage(
        &store,
        &StorageMigrationOptions {
            allow_non_atomic: true,
        },
    )
    .unwrap_or_else(|e| {
        eprintln!("{path}: rfc038-storage failed: {e}");
        std::process::exit(1);
    });
    println!(
        "{path}: relations exploded {}, collections removed {:?}, properties stripped {:?}",
        result.relations_exploded, result.collections_removed, result.manifest_properties_stripped
    );
}
