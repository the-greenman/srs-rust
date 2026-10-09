//! Scaled-corpus benchmark (srs-rust#1194): how the engine scales with corpus size.
//!
//! Takes any valid directory repository (default: the vendored `exploded-basic`
//! fixture), writes an ×N copy of its instances under a temp dir, and times catalog
//! build and the core read/write paths against each scale, printing one table.
//!
//! Usage (always `--release`; debug timings are meaningless):
//!   cargo run --release -p srs-repository --example scaled_bench -- \
//!       [--source <repo>] [--scales 1,10] [--iters 5]
//!   cargo run --release -p srs-repository --example scaled_bench -- \
//!       --emit <empty-dir> --scale 10 [--source <repo>]     # corpus only, no timing
//!
//! Copy `k` (k ≥ 1) of an instance gets a deterministic id derived from
//! SHA-256(`srs-scaled-bench/{k}/{old id}`), version/variant bits set so it is a
//! well-formed UUID. Every reference to a remapped id in a cloned file is rewritten
//! (relations and container members still resolve), and filenames keep their
//! canonical stems (`{slug}-{id8}` / `{relationId}`). Copy 0 is the source,
//! untouched. The manifest's identity instance is never cloned, nor anything that
//! references it. Type/field/package definitions are shared, not cloned.

use sha2::{Digest, Sha256};
use srs_repository::catalog;
use srs_repository::record_store::{list_records_filtered, RecordListFilter};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Reserved instance roots that hold cloneable files.
const INSTANCE_ROOTS: [&str; 3] = ["records", "relations", "containers"];

fn main() {
    let args = parse_args();
    let source = args.source.clone().unwrap_or_else(default_source);
    if let Some(dir) = &args.emit {
        let scale = args.scale.unwrap_or(10);
        generate(&source, dir, scale);
        println!("wrote ×{scale} corpus to {}", dir.display());
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    println!("source: {}", source.display());
    println!("iterations per cell: {} (median)\n", args.iters);
    let mut table: Vec<(String, Vec<Duration>)> = Vec::new();
    let mut counts = Vec::new();
    for &scale in &args.scales {
        let dir = tmp.path().join(format!("x{scale}"));
        let files = generate(&source, &dir, scale);
        counts.push(files);
        let cells = bench_scale(&dir, args.iters);
        for (i, (name, d)) in cells.into_iter().enumerate() {
            if table.len() <= i {
                table.push((name, Vec::new()));
            }
            table[i].1.push(d);
        }
    }
    print!("{:<44}", "operation (median ms)");
    for s in &args.scales {
        print!("{:>12}", format!("×{s}"));
    }
    println!();
    print!("{:<44}", "instance files");
    for c in &counts {
        print!("{c:>12}");
    }
    println!();
    for (name, ds) in &table {
        print!("{name:<44}");
        for d in ds {
            print!("{:>12.2}", d.as_secs_f64() * 1000.0);
        }
        println!();
    }
}

fn bench_scale(dir: &Path, iters: usize) -> Vec<(String, Duration)> {
    let probe = probe_record_id(dir);
    let mut out = Vec::new();

    out.push((
        "catalog::build_checked (cold, in-process)".to_string(),
        median(iters, || {
            let store = FileStore::new(dir);
            let t = Instant::now();
            catalog::build_checked(&store).expect("build_checked");
            t.elapsed()
        }),
    ));
    out.push((
        "find_instance + load_record_by_id (cold)".to_string(),
        median(iters, || {
            let store = FileStore::new(dir);
            let t = Instant::now();
            store.find_instance(&probe).expect("find").expect("present");
            store.load_record_by_id(&probe).expect("load");
            t.elapsed()
        }),
    ));
    out.push((
        "write then read, same store (memo invalidation)".to_string(),
        median(iters, || {
            let store = FileStore::new(dir);
            let mut record = store.load_record_by_id(&probe).expect("load");
            store.catalog().expect("warm catalog");
            record.updated_at = Some(chrono::Utc::now().to_rfc3339());
            let t = Instant::now();
            store.save_record(&record).expect("save");
            store.find_instance(&probe).expect("find").expect("present");
            t.elapsed()
        }),
    ));
    out.push((
        "list_records_filtered (cold)".to_string(),
        median(iters, || {
            let store = FileStore::new(dir);
            let t = Instant::now();
            list_records_filtered(&store, RecordListFilter::default()).expect("list");
            t.elapsed()
        }),
    ));
    out.push((
        "validate_repository (cold, = `repo validate`)".to_string(),
        median(iters, || {
            let store = FileStore::new(dir);
            let t = Instant::now();
            let report = validate_repository(&store).expect("validate");
            let d = t.elapsed();
            let errors = report
                .diagnostics
                .iter()
                .filter(|d| format!("{:?}", d.severity).to_lowercase().contains("error"))
                .count();
            assert_eq!(errors, 0, "generated corpus must validate with 0 errors");
            d
        }),
    ));
    out
}

fn median(iters: usize, mut f: impl FnMut() -> Duration) -> Duration {
    let mut v: Vec<Duration> = (0..iters.max(1)).map(|_| f()).collect();
    v.sort();
    v[v.len() / 2]
}

fn probe_record_id(dir: &Path) -> String {
    let store = FileStore::new(dir);
    let refs = store
        .list_instances(&srs_repository::index::InstanceQuery {
            tier: Some(2),
            tag: None,
        })
        .expect("list_instances");
    refs.last()
        .expect("corpus has a record")
        .instance_id
        .clone()
}

// ---- corpus generator -------------------------------------------------------

/// Write an ×`scale` copy of `source` into `dest`; returns the instance-file count.
fn generate(source: &Path, dest: &Path, scale: usize) -> usize {
    copy_tree(source, dest);
    let identity = identity_id(source);

    // Cloneable files: (relative path, text, ids defined, ids referenced).
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    for root in INSTANCE_ROOTS {
        collect(&source.join(root), Path::new(root), &mut files);
    }
    files.retain(|(_, text)| identity.as_deref().is_none_or(|id| !text.contains(id)));

    let defined: Vec<String> = files.iter().filter_map(|(_, t)| defined_id(t)).collect();
    let mut written = files.len() + identity.map_or(0, |_| 1);
    for k in 1..scale {
        let map: BTreeMap<String, String> = defined
            .iter()
            .map(|id| (id.clone(), remap(k, id)))
            .collect();
        for (rel, text) in &files {
            let mut new_text = text.clone();
            for (old, new) in &map {
                new_text = new_text.replace(old.as_str(), new);
            }
            let name = rename(rel.file_name().unwrap().to_str().unwrap(), &map);
            let target = dest.join(rel.parent().unwrap()).join(name);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, new_text).unwrap();
            written += 1;
        }
    }
    written
}

fn identity_id(source: &Path) -> Option<String> {
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(source.join("manifest.json")).ok()?).ok()?;
    m["container"]["identityInstanceId"]
        .as_str()
        .map(str::to_string)
}

fn defined_id(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    ["instanceId", "relationId", "containerId"]
        .iter()
        .find_map(|k| v[*k].as_str().map(str::to_string))
}

fn remap(copy: usize, old: &str) -> String {
    let mut h = Sha256::new();
    h.update(format!("srs-scaled-bench/{copy}/{old}"));
    let d = h.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&d[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let x = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &x[..8],
        &x[8..12],
        &x[12..16],
        &x[16..20],
        &x[20..]
    )
}

/// Canonical filenames embed the full id (relations) or its first 8 chars (`{slug}-{id8}`).
fn rename(name: &str, map: &BTreeMap<String, String>) -> String {
    let mut n = name.to_string();
    for (old, new) in map {
        n = n.replace(old.as_str(), new).replace(&old[..8], &new[..8]);
    }
    n
}

fn collect(dir: &Path, rel: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.map(|e| e.unwrap()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let r = rel.join(e.file_name());
        if p.is_dir() {
            collect(&p, &r, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push((r, std::fs::read_to_string(&p).unwrap()));
        }
    }
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let (s, d) = (e.path(), dst.join(e.file_name()));
        if s.is_dir() {
            copy_tree(&s, &d);
        } else {
            std::fs::copy(&s, &d).unwrap();
        }
    }
}

// ---- args -------------------------------------------------------------------

struct Args {
    source: Option<PathBuf>,
    scales: Vec<usize>,
    iters: usize,
    emit: Option<PathBuf>,
    scale: Option<usize>,
}

fn default_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/exploded-basic")
}

fn parse_args() -> Args {
    let mut a = Args {
        source: None,
        scales: vec![1, 10],
        iters: 5,
        emit: None,
        scale: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || {
            it.next()
                .unwrap_or_else(|| die(&format!("{flag} needs a value")))
        };
        match flag.as_str() {
            "--source" => a.source = Some(val().into()),
            "--scales" => {
                a.scales = val()
                    .split(',')
                    .map(|s| s.parse().unwrap_or_else(|_| die("bad --scales")))
                    .collect()
            }
            "--iters" => a.iters = val().parse().unwrap_or_else(|_| die("bad --iters")),
            "--emit" => a.emit = Some(val().into()),
            "--scale" => a.scale = Some(val().parse().unwrap_or_else(|_| die("bad --scale"))),
            other => die(&format!("unknown flag {other}")),
        }
    }
    a
}

fn die(msg: &str) -> ! {
    eprintln!("scaled_bench: {msg}");
    std::process::exit(2);
}
