//! Scaled-corpus benchmark (srs-rust#1194): how the engine scales with corpus size.
//!
//! Two subcommands:
//!
//! * `gen <src-repo> <dst-repo> <copies>` — write an xN copy of an exploded repository.
//!   Every file under `records/`, `relations/` and `containers/` is cloned per copy index
//!   with every instance/relation/container id remapped to a deterministic UUID (derived
//!   from SHA-256 of copy index + old id, version/variant bits set as for UUIDv5), and the
//!   same remap applied to every reference inside the cloned files. Filenames follow the
//!   canonical rules (a relation's stem is its id; otherwise `{slug}-{id8}` with the new
//!   id8). Everything else (manifest, package, definitions) is copied once, unchanged.
//! * `bench <repo>...` — time core operations against each repository and print a table.
//!
//! `scripts/bench-scale.sh` runs both against the pinned muSrs corpus at x1 and x10
//! (see `docs/benchmarking.md`).
//!
//! Usage: cargo run --release -p srs-repository --example scale_bench -- <gen|bench> ...

use sha2::{Digest, Sha256};
use srs_repository::catalog;
use srs_repository::record_store::{list_records_filtered, RecordListFilter};
use srs_repository::services::get_note_by_id;
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CLONED_ROOTS: [&str; 3] = ["records", "relations", "containers"];
const ITERATIONS: usize = 3;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") if args.len() == 4 => {
            let copies: usize = args[3]
                .parse()
                .unwrap_or_else(|_| die("copies must be a number"));
            generate(Path::new(&args[1]), Path::new(&args[2]), copies);
        }
        Some("bench") if args.len() >= 2 => bench(&args[1..]),
        _ => die("usage: scale_bench gen <src> <dst> <copies> | bench <repo>..."),
    }
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2);
}

// ---------------------------------------------------------------- generator

fn generate(src: &Path, dst: &Path, copies: usize) {
    if copies == 0 {
        die("copies must be >= 1");
    }
    if dst.exists() {
        die("destination already exists");
    }
    let mut files = Vec::new();
    walk(src, &mut files);

    // Every id that identifies a cloned entity.
    let mut ids: Vec<String> = Vec::new();
    for rel in files.iter().filter(|r| is_cloned(r)) {
        let text =
            fs::read_to_string(src.join(rel)).unwrap_or_else(|e| die(&format!("{rel:?}: {e}")));
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| die(&format!("{rel:?}: {e}")));
        collect_ids(&value, &mut ids);
    }
    ids.sort();
    ids.dedup();

    // One id map per copy, built once.
    let maps: Vec<HashMap<&str, String>> = (1..copies)
        .map(|copy| {
            ids.iter()
                .map(|id| (id.as_str(), remap(copy, id)))
                .collect()
        })
        .collect();

    for rel in &files {
        let bytes = fs::read(src.join(rel)).unwrap();
        write_file(&dst.join(rel), &bytes);
        if !is_cloned(rel) {
            continue;
        }
        let text = String::from_utf8(bytes).unwrap();
        for map in &maps {
            let new_text = replace_uuids(&text, map);
            write_file(&dst.join(renamed(rel, &new_text, map)), new_text.as_bytes());
        }
    }
    println!(
        "{} files from {} -> {} (x{copies})",
        files.len(),
        src.display(),
        dst.display()
    );
}

fn is_cloned(rel: &Path) -> bool {
    rel.extension().is_some_and(|e| e == "json")
        && rel
            .components()
            .next()
            .is_some_and(|c| CLONED_ROOTS.iter().any(|r| c.as_os_str() == *r))
}

fn walk(root: &Path, out: &mut Vec<PathBuf>) {
    fn go(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                go(root, &p, out);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    go(root, root, out);
}

fn write_file(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn collect_ids(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, val) in m {
                if matches!(k.as_str(), "instanceId" | "relationId" | "containerId") {
                    if let Some(s) = val.as_str() {
                        out.push(s.to_string());
                    }
                }
                collect_ids(val, out);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_ids(x, out)),
        _ => {}
    }
}

/// Deterministic UUID for (copy index, old id): SHA-256 truncated, v5 version/variant bits.
fn remap(copy: usize, old: &str) -> String {
    let mut h = Sha256::new();
    h.update((copy as u64).to_le_bytes());
    h.update(old.as_bytes());
    let d = h.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&d[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn is_uuid_at(b: &[u8], i: usize) -> bool {
    if i + 36 > b.len() {
        return false;
    }
    b[i..i + 36].iter().enumerate().all(|(j, c)| match j {
        8 | 13 | 18 | 23 => *c == b'-',
        _ => c.is_ascii_hexdigit(),
    })
}

fn replace_uuids(text: &str, map: &HashMap<&str, String>) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut last) = (0, 0);
    while i < b.len() {
        if is_uuid_at(b, i) {
            if let Some(new) = map.get(&text[i..i + 36]) {
                out.push_str(&text[last..i]);
                out.push_str(new);
                i += 36;
                last = i;
                continue;
            }
            i += 36;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Rename a cloned file: a relation's stem is its (new) id; any other file keeps its slug and
/// takes the first 8 characters of its own (new) id, read from the remapped content, as suffix.
fn renamed(rel: &Path, new_text: &str, map: &HashMap<&str, String>) -> PathBuf {
    let stem = rel.file_stem().unwrap().to_string_lossy().to_string();
    let new_stem = if stem.len() == 36 && is_uuid_at(stem.as_bytes(), 0) {
        map.get(stem.as_str()).cloned().unwrap_or(stem)
    } else {
        let value: serde_json::Value = serde_json::from_str(new_text).unwrap();
        let id = ["instanceId", "containerId", "relationId"]
            .iter()
            .find_map(|k| value.get(k).and_then(|v| v.as_str()))
            .unwrap_or_else(|| die(&format!("{rel:?}: no id to name the clone after")));
        match stem.rsplit_once('-') {
            Some((slug, _)) => format!("{slug}-{}", &id[..8]),
            None => format!("{stem}-{}", &id[..8]),
        }
    };
    rel.with_file_name(format!("{new_stem}.json"))
}

// ---------------------------------------------------------------- benchmark

fn best_of<T>(mut f: impl FnMut() -> T) -> Duration {
    (0..ITERATIONS)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed()
        })
        .min()
        .unwrap()
}

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

fn bench(repos: &[String]) {
    let rows = [
        "catalog::build_checked",
        "find_instance (cold store, incl. catalog)",
        "get_note_by_id (cold store, incl. catalog)",
        "write + read (save_note, find_instance)",
        "list_records_filtered (all)",
        "validate_repository",
    ];
    let mut cols: Vec<(String, usize, Vec<Duration>)> = Vec::new();
    for repo in repos {
        // Work on a copy: the write+read row mutates the repository.
        let work = tempdir_copy(Path::new(repo));
        let store = || FileStore::new(&work);
        let cat = catalog::build(&store()).unwrap_or_else(|e| die(&format!("{repo}: {e}")));
        if cat.has_fatal() {
            let mut by_code: std::collections::BTreeMap<&str, usize> = Default::default();
            for d in &cat.diagnostics {
                *by_code.entry(d.code).or_default() += 1;
            }
            eprintln!("{:?}", cat.diagnostics.iter().take(3).collect::<Vec<_>>());
            die(&format!(
                "{repo}: catalog has fatal diagnostics: {by_code:?}"
            ));
        }
        let entities = cat.instances.len() + cat.relations.len() + cat.containers.len();
        let note_id = cat
            .instances
            .iter()
            .find(|e| e.tier == Some(0))
            .map(|e| e.id.clone())
            .unwrap_or_else(|| die("corpus has no note to read"));

        let mut times = Vec::new();
        times.push(best_of(|| catalog::build_checked(&store()).unwrap()));
        times.push(best_of(|| {
            let s = store();
            s.find_instance(&note_id).unwrap()
        }));
        times.push(best_of(|| {
            let s = store();
            get_note_by_id(&s, &note_id).unwrap()
        }));
        times.push(best_of(|| {
            let s = store();
            let note = s.load_note_by_id(&note_id).unwrap();
            s.save_note(&note).unwrap();
            s.find_instance(&note_id).unwrap()
        }));
        times.push(best_of(|| {
            list_records_filtered(&store(), RecordListFilter::default()).unwrap()
        }));
        let report = validate_repository(&store()).unwrap();
        if !report.is_ok() {
            die(&format!(
                "{repo}: validate_repository reports {} error(s)",
                report.summary.errors
            ));
        }
        times.push(best_of(|| validate_repository(&store()).unwrap()));
        cols.push((repo.clone(), entities, times));
        let _ = fs::remove_dir_all(&work);
    }

    println!("Best of {ITERATIONS}, cold FileStore per iteration, milliseconds.\n");
    print!("{:<42}", "operation");
    for (repo, entities, _) in &cols {
        let name = Path::new(repo).file_name().unwrap().to_string_lossy();
        print!(" {:>16}", format!("{name} ({entities})"));
    }
    println!();
    for (i, row) in rows.iter().enumerate() {
        print!("{row:<42}");
        for (_, _, t) in &cols {
            print!(" {:>16}", ms(t[i]));
        }
        println!();
    }
}

fn tempdir_copy(src: &Path) -> PathBuf {
    let dst = std::env::temp_dir().join(format!(
        "scale-bench-{}-{}",
        std::process::id(),
        src.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_dir_all(&dst);
    let mut files = Vec::new();
    walk(src, &mut files);
    for rel in files {
        write_file(&dst.join(&rel), &fs::read(src.join(&rel)).unwrap());
    }
    dst
}
