//! Discovery evaluation harness (srs-rust#1231): recall@10 / MRR per discovery method on muSrs.
//!
//! Runs ~25 realistic questions (`tests/fixtures/discovery-eval/questions.json`, fixture data, not
//! spec content) against a PINNED muSrs export (`musrs-pinned.srs`, packed with
//! `srs archive pack` from muDemocracy.org commit 6e6cd5b, 886 instances), through the real
//! `discovery_service::find`, and prints a table plus a classification of every miss.
//!
//! One command:  `cargo test -p srs-repository --test discovery_eval -- --nocapture`
//!
//! Adding a method (BM25 #1228, `similar` #1230, all-words #1218): append one entry to
//! [`methods`]. A method maps a query string to instance ids, best first.
//!
//! Layer-1 is unranked (id order), so "recall@10" for it is the first 10 hits as returned;
//! recall@all separates "found but buried" (ranking) from "absent" (not found at all).
//!
//! Miss classes for an expected id that is not in a method's top 10:
//! - `ranking`: returned, but below rank 10;
//! - `phrase`: absent, though every query word occurs in the record (only the contiguous phrase
//!   fails; an all-words method removes these);
//! - `label/tag`: absent, and each missing word is only in the record's label or tags;
//! - `vocabulary-mismatch`: absent, and some query word occurs nowhere in the record. The only
//!   class an embedding layer could fix.

use serde::Deserialize;
use srs_repository::discovery_service::{find, DiscoveryQuery};
use srs_repository::{archive_unpack, FileStore, RepositoryStore};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Fixture {
    corpus: Corpus,
    questions: Vec<Question>,
}

#[derive(Deserialize)]
struct Corpus {
    instances: usize,
    archive: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    id: String,
    #[allow(dead_code)] // the natural-language question; `query` is what an agent would type
    question: String,
    query: String,
    kinds: Vec<String>,
    expected_instance_ids: Vec<String>,
}

type Method = fn(&dyn RepositoryStore, &str) -> Vec<String>;

/// The methods under evaluation, in table order.
fn methods() -> Vec<(&'static str, Method)> {
    vec![("substring (Layer 1)", substring)]
}

fn substring(store: &dyn RepositoryStore, query: &str) -> Vec<String> {
    let q = DiscoveryQuery {
        content_match: Some(query.to_string()),
        ..Default::default()
    };
    find(store, q)
        .expect("find")
        .hits
        .into_iter()
        .map(|h| h.instance_id)
        .collect()
}

/// Per-word lookups for miss classification, via the same `find`.
struct Probe<'a> {
    store: &'a dyn RepositoryStore,
    in_text: HashMap<String, HashSet<String>>,
    in_tag: HashMap<String, HashSet<String>>,
    labels: HashMap<String, String>,
}

impl<'a> Probe<'a> {
    fn new(store: &'a dyn RepositoryStore) -> Self {
        let labels = find(store, DiscoveryQuery::default())
            .expect("find all")
            .hits
            .into_iter()
            .map(|h| (h.instance_id, h.label.to_lowercase()))
            .collect();
        Probe {
            store,
            in_text: HashMap::new(),
            in_tag: HashMap::new(),
            labels,
        }
    }

    fn text_has(&mut self, word: &str, id: &str) -> bool {
        let store = self.store;
        self.in_text
            .entry(word.to_string())
            .or_insert_with(|| substring(store, word).into_iter().collect())
            .contains(id)
    }

    fn label_or_tag_has(&mut self, word: &str, id: &str) -> bool {
        let store = self.store;
        let tagged = self.in_tag.entry(word.to_string()).or_insert_with(|| {
            find(
                store,
                DiscoveryQuery {
                    tag: vec![word.to_string()],
                    ..Default::default()
                },
            )
            .expect("find tag")
            .hits
            .into_iter()
            .map(|h| h.instance_id)
            .collect()
        });
        tagged.contains(id) || self.labels.get(id).is_some_and(|l| l.contains(word))
    }

    fn classify_absent(&mut self, query: &str, id: &str) -> &'static str {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let missing: Vec<&String> = words.iter().filter(|w| !self.text_has(w, id)).collect();
        if missing.is_empty() {
            "phrase"
        } else if missing.iter().all(|w| self.label_or_tag_has(w, id)) {
            "label/tag"
        } else {
            "vocabulary-mismatch"
        }
    }
}

#[derive(Default)]
struct Totals {
    recall10: f64,
    recall_all: f64,
    rr: f64,
    misses: BTreeMap<&'static str, usize>,
    by_kind: BTreeMap<String, (usize, usize)>, // kind -> (expected ids, found in top 10)
}

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/discovery-eval")
}

/// Floor for the Layer-1 baseline's recall@all (deterministic on the pinned corpus). Raise it, never lower it.
const SUBSTRING_RECALL_ALL_FLOOR: f64 = 0.34;

#[test]
fn discovery_eval_table() {
    let dir = data_dir();
    let fx: Fixture =
        serde_json::from_str(&std::fs::read_to_string(dir.join("questions.json")).unwrap())
            .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = FileStore::new(tmp.path());
    archive_unpack(
        Cursor::new(std::fs::read(dir.join(&fx.corpus.archive)).unwrap()),
        &store,
    )
    .expect("unpack pinned muSrs archive");

    // Not vacuous: the pinned corpus is whole and every expected id exists in it.
    let all: HashSet<String> = find(&store, DiscoveryQuery::default())
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.instance_id)
        .collect();
    assert_eq!(all.len(), fx.corpus.instances, "pinned corpus size");
    assert!(fx.questions.len() >= 20, "question set too small");
    for q in &fx.questions {
        assert!(
            !q.expected_instance_ids.is_empty(),
            "{} has no expected ids",
            q.id
        );
        for id in &q.expected_instance_ids {
            assert!(all.contains(id), "{}: expected id {id} not in corpus", q.id);
        }
    }

    let mut probe = Probe::new(&store);
    let mut results: Vec<(&str, Totals)> = Vec::new();
    for (name, method) in methods() {
        let mut t = Totals::default();
        let mut detail = String::new();
        for q in &fx.questions {
            let ranked = method(&store, &q.query);
            let top10: HashSet<&String> = ranked.iter().take(10).collect();
            let n = q.expected_instance_ids.len() as f64;
            let found10 = q
                .expected_instance_ids
                .iter()
                .filter(|i| top10.contains(i))
                .count();
            let found_all = q
                .expected_instance_ids
                .iter()
                .filter(|i| ranked.contains(i))
                .count();
            let first = ranked
                .iter()
                .position(|r| q.expected_instance_ids.contains(r))
                .map(|p| p + 1);
            t.recall10 += found10 as f64 / n;
            t.recall_all += found_all as f64 / n;
            t.rr += first.map_or(0.0, |p| 1.0 / p as f64);
            for kind in &q.kinds {
                let e = t.by_kind.entry(kind.clone()).or_default();
                e.0 += q.expected_instance_ids.len();
                e.1 += found10;
            }
            let mut classes = Vec::new();
            for id in &q.expected_instance_ids {
                if top10.contains(id) {
                    continue;
                }
                let class = if ranked.contains(id) {
                    "ranking"
                } else {
                    probe.classify_absent(&q.query, id)
                };
                *t.misses.entry(class).or_default() += 1;
                classes.push(class);
            }
            detail += &format!(
                "  {} {:<34} hits={:<4} R@10={}/{} first={:<5} {}\n",
                q.id,
                format!("\"{}\"", q.query),
                ranked.len(),
                found10,
                q.expected_instance_ids.len(),
                first.map_or("-".into(), |p| p.to_string()),
                classes.join(",")
            );
        }
        println!(
            "\n{name}: per question ({} questions)\n{detail}",
            fx.questions.len()
        );
        results.push((name, t));
    }

    let n = fx.questions.len() as f64;
    println!("\nmethod                 recall@10  recall@all  MRR");
    for (name, t) in &results {
        println!(
            "{:<22} {:>9.3}  {:>10.3}  {:.3}",
            name,
            t.recall10 / n,
            t.recall_all / n,
            t.rr / n
        );
    }
    for (name, t) in &results {
        println!("\n{name}: misses at 10 by class: {:?}", t.misses);
        println!(
            "{name}: found@10 / expected by question kind: {:?}",
            t.by_kind
        );
    }

    let substring_all = results[0].1.recall_all / n;
    assert!(
        substring_all >= SUBSTRING_RECALL_ALL_FLOOR,
        "Layer-1 recall@all regressed: {substring_all} < {SUBSTRING_RECALL_ALL_FLOOR}"
    );
}
