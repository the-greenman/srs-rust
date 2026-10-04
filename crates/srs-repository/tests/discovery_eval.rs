//! Discovery evaluation harness (srs-rust#1231): recall@10 / MRR per discovery method on muSrs.
//!
//! Runs ~25 realistic questions (`tests/fixtures/discovery-eval/questions.json`, fixture data, not
//! spec content) against a PINNED muSrs export (`musrs-pinned.srs`, packed with
//! `srs archive pack` from muDemocracy.org commit 4c45f35f, after titles #263, 886 instances), through the real
//! `discovery_service::find`, and prints a table plus a classification of every miss.
//!
//! One command:  `cargo test -p srs-repository --test discovery_eval -- --nocapture`
//!
//! Adding a method (`similar` #1230; all-words #1218 and BM25 #1228 are rows already): append one entry to
//! [`methods`]. A method maps a query string to instance ids, best first.
//!
//! Layer-1 is unranked (id order), so "recall@10" for it is the first 10 hits as returned;
//! recall@all separates "found but buried" (ranking) from "absent" (not found at all).
//!
//! Question kinds (`kinds`): `vocab` = the record uses other words than the query; `untitled` = a
//! claim/problem whose label is only an id (C-13, P-10); `run-report-noise` = the query words also
//! occur in long run-reports; `tag` = answer is defined by a tag.
//!
//! Layer-1 rows (substring, all-words) are unranked: never quote them as a ranking number.
//!
//! Miss classes for an expected id that is not in a method's top 10:
//! - `ranking`: returned, but below rank 10;
//! - `phrase`: absent, though every query word occurs in the record (only the contiguous phrase
//!   fails; an all-words method removes these);
//! - `label/tag`: absent, and each missing word is only in the record's label or tags;
//! - `vocabulary-mismatch`: absent, and some query word occurs nowhere in the record. The class
//!   that stemming, synonyms or an embedding layer would have to fix.

use serde::Deserialize;
use srs_repository::discovery_service::{find, similar, DiscoveryQuery, FindPage};
use srs_repository::text_projection::{
    build_field_text_index, normalize, project_note_text, project_text,
};
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
#[serde(rename_all = "camelCase")]
struct Corpus {
    instances: usize,
    archive: String,
    archive_bytes: usize,
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
    vec![
        ("substring (phrase, unranked: id order)", phrase),
        ("all-words (Layer 1, unranked: id order)", all_words),
        ("BM25 (all-words candidates, ranked)", bm25),
        ("BM25 top-5, then similar to its top hit", bm25_then_similar),
    ]
}

fn find_ids(store: &dyn RepositoryStore, query: &str, rank: bool) -> Vec<String> {
    let q = DiscoveryQuery {
        content_match: Some(query.to_string()),
        ..Default::default()
    };
    let page = FindPage {
        rank,
        ..Default::default()
    };
    find(store, q, page)
        .expect("find")
        .hits
        .into_iter()
        .map(|h| h.instance_id)
        .collect()
}

fn all_words(store: &dyn RepositoryStore, query: &str) -> Vec<String> {
    find_ids(store, query, false)
}

fn bm25(store: &dyn RepositoryStore, query: &str) -> Vec<String> {
    find_ids(store, query, true)
}

/// `similar` (srs-rust#1230) as a follow-up to a search: keep BM25's top 5, then append the
/// instances similar to BM25's top hit (the "found one, what else is about this?" workflow).
/// A top-10 slice is therefore 5 direct + 5 neighbours, never more direct hits than `bm25` has.
fn bm25_then_similar(store: &dyn RepositoryStore, query: &str) -> Vec<String> {
    let direct = bm25(store, query);
    let Some(seed) = direct.first() else {
        return direct;
    };
    let mut out: Vec<String> = direct.iter().take(5).cloned().collect();
    for hit in similar_ids(store, seed) {
        if !out.contains(&hit) {
            out.push(hit);
        }
    }
    out
}

fn similar_ids(store: &dyn RepositoryStore, seed: &str) -> Vec<String> {
    similar(store, seed, DiscoveryQuery::default(), FindPage::default())
        .expect("similar")
        .hits
        .into_iter()
        .map(|h| h.instance_id)
        .collect()
}

/// The pre-#1218 Layer-1 matcher: the whole query as one contiguous phrase in a single segment.
/// Derived from the all-words hits (a superset), so it needs no second corpus walk.
fn phrase(store: &dyn RepositoryStore, query: &str) -> Vec<String> {
    let needle = normalize(query);
    let index = build_field_text_index(store).expect("field index");
    all_words(store, query)
        .into_iter()
        .filter(|id| {
            let tier = store.find_instance(id).unwrap().expect("instance").tier;
            let segments = if tier == 0 {
                project_note_text(&store.load_note_by_id(id).expect("note"))
            } else {
                project_text(&store.load_record_by_id(id).expect("record"), &index)
            };
            segments
                .iter()
                .any(|s| normalize(&s.text).contains(&needle))
        })
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
        let labels = find(store, DiscoveryQuery::default(), FindPage::default())
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
            .or_insert_with(|| all_words(store, word).into_iter().collect())
            .contains(id)
    }

    fn label_or_tag_has(&mut self, word: &str, id: &str) -> bool {
        let store = self.store;
        let tagged = self.in_tag.entry(word.to_string()).or_insert_with(|| {
            find(
                store,
                DiscoveryQuery {
                    tag: vec![word.to_string()], // words are already lowercased by the caller
                    ..Default::default()
                },
                FindPage::default(),
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
    found10_by_question: Vec<usize>,
    hit_sets: Vec<HashSet<String>>,
    by_kind: BTreeMap<String, (usize, usize)>, // kind -> (expected ids, found in top 10)
    /// (question id, expected id) found in the top 10.
    found: HashSet<(String, String)>,
    /// (question id, expected id, class) missed in the top 10.
    missed: Vec<(String, String, &'static str)>,
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
    let archive_bytes = std::fs::read(dir.join(&fx.corpus.archive)).unwrap();
    assert_eq!(
        archive_bytes.len(),
        fx.corpus.archive_bytes,
        "archive changed: a re-pack must update questions.json (archiveBytes, expected ids)"
    );
    let tmp = tempfile::tempdir().unwrap();
    let store = FileStore::new(tmp.path());
    archive_unpack(Cursor::new(archive_bytes), &store).expect("unpack pinned muSrs archive");

    // Not vacuous: the pinned corpus is whole and every expected id exists in it.
    let all: HashSet<String> = find(&store, DiscoveryQuery::default(), FindPage::default())
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
            t.found10_by_question.push(found10);
            t.hit_sets.push(ranked.iter().cloned().collect());
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
                t.missed.push((q.id.clone(), id.clone(), class));
                classes.push(class);
            }
            for id in q.expected_instance_ids.iter().filter(|i| top10.contains(i)) {
                t.found.insert((q.id.clone(), id.clone()));
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
    println!("\nmethod                                    recall@10  recall@all  MRR");
    for (name, t) in &results {
        println!(
            "{:<41} {:>9.3}  {:>10.3}  {:.3}",
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

    // Does lexical similarity cover related content the query words miss? Of the expected ids BM25
    // misses as `vocabulary-mismatch`, how many does "similar to BM25's top hit" bring into the top 10?
    let bm25_t = &results
        .iter()
        .find(|(n, _)| n.starts_with("BM25 ("))
        .unwrap()
        .1;
    let sim_t = &results
        .iter()
        .find(|(n, _)| n.starts_with("BM25 top-5"))
        .unwrap()
        .1;
    let vocab: Vec<_> = bm25_t
        .missed
        .iter()
        .filter(|(_, _, c)| *c == "vocabulary-mismatch")
        .collect();
    let recovered: Vec<_> = vocab
        .iter()
        .filter(|(q, id, _)| sim_t.found.contains(&(q.clone(), id.clone())))
        .collect();
    let all_missed_recovered = bm25_t
        .missed
        .iter()
        .filter(|(q, id, _)| sim_t.found.contains(&(q.clone(), id.clone())))
        .count();
    println!(
        "\nsimilar vs BM25 misses: vocabulary-mismatch misses {} , recovered by similar-to-top-hit {} {:?}; all BM25 misses recovered {} of {}",
        vocab.len(),
        recovered.len(),
        recovered.iter().map(|(q, id, _)| format!("{q}:{}", &id[..8])).collect::<Vec<_>>(),
        all_missed_recovered,
        bm25_t.missed.len()
    );
    // Deterministic on the pinned corpus (3 of 12 at #1230). Raise it, never lower it.
    assert!(
        recovered.len() >= 3,
        "similar recovers fewer vocabulary-mismatch misses than before"
    );
    let lost = bm25_t
        .found
        .iter()
        .filter(|k| !sim_t.found.contains(*k))
        .count();
    println!("expected ids in BM25 top 10 but pushed out by the similar row (5+5 split): {lost}");

    // Related-to: questions with several expected records. Seed with each expected record and ask
    // whether similar brings the other expected records into its top 10.
    let (mut related, mut related_found) = (0, 0);
    for q in fx
        .questions
        .iter()
        .filter(|q| q.expected_instance_ids.len() >= 2 && q.kinds.iter().all(|k| k != "tag"))
    {
        for seed in &q.expected_instance_ids {
            let top10: Vec<String> = similar_ids(&store, seed).into_iter().take(10).collect();
            for other in q.expected_instance_ids.iter().filter(|o| *o != seed) {
                related += 1;
                related_found += usize::from(top10.contains(other));
            }
        }
    }
    println!("related-to (expected records of one question, non-tag): similar recovers {related_found} of {related} ordered pairs in its top 10");
    // `similar` never returns its seed.
    for q in &fx.questions {
        for seed in &q.expected_instance_ids {
            assert!(
                !similar_ids(&store, seed).contains(seed),
                "{} similar returned its seed",
                q.id
            );
        }
    }

    // Ranking only orders: BM25 returns exactly the all-words set, and on no question
    // finds fewer expected ids in the top 10 than the substring baseline (srs-rust#1228).
    let row = |prefix: &str| {
        &results
            .iter()
            .find(|(name, _)| name.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix} row present"))
            .1
    };
    let (sub_row, words_row, bm25_row) = (row("substring"), row("all-words"), row("BM25 ("));
    assert_eq!(
        bm25_row.hit_sets, words_row.hit_sets,
        "BM25 changed the hit set"
    );
    for (i, q) in fx.questions.iter().enumerate() {
        assert!(
            bm25_row.found10_by_question[i] >= sub_row.found10_by_question[i],
            "{}: BM25 top-10 recall fell below substring",
            q.id
        );
    }

    let substring_all = results
        .iter()
        .find(|(name, _)| name.starts_with("substring"))
        .expect("substring baseline present")
        .1
        .recall_all
        / n;
    assert!(
        substring_all >= SUBSTRING_RECALL_ALL_FLOOR,
        "Layer-1 recall@all regressed: {substring_all} < {SUBSTRING_RECALL_ALL_FLOOR}"
    );
}
