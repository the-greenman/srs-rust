/// generate-governance-seed — apply RFC-014 migration to a governance seed file.
///
/// Usage:
///   cargo run --release --bin generate-governance-seed -- <input-seed-path> <output-path>
///
/// Reads the raw governance seed from <input-seed-path>, applies migrate_rfc014,
/// and writes the migrated result to <output-path>. Used by the release workflow
/// to bundle a migration-ready seed in srs-bindings-web.tar.gz.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: generate-governance-seed <input-seed-path> <output-path>");
        std::process::exit(1);
    }
    let input_path = &args[1];
    let output_path = &args[2];

    let raw = match std::fs::read_to_string(input_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not read {input_path}: {e}");
            std::process::exit(1);
        }
    };

    let migrated = match srs_repository::srsj_migration_service::migrate_rfc014(&raw) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: RFC-014 migration failed: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = std::fs::write(output_path, migrated) {
        eprintln!("error: could not write {output_path}: {e}");
        std::process::exit(1);
    }

    println!("wrote {output_path}");
}
