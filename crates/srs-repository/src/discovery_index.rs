//! `DiscoveryIndex` — the single Layer-2 extension point reserved by ADR-019
//! (srs-rust#1228), and its first implementation, [`Bm25Index`].
//!
//! An index only **orders** candidates the Layer-1 matcher already found; it never adds
//! or drops one, so the RFC-012 recall floor is untouched (`[R4]`). It is built from the
//! same [`text_projection`] segments the matcher reads (no second projection), is
//! deterministic, and costs no dependency.
//!
//! Scoring is BM25 with per-segment field weights (BM25F-style): label/title above tags
//! above short fields (statement, description) above long bodies. A word's term frequency
//! is its normalized substring count — the same notion of "contains" Layer 1 uses — so
//! a Layer-1 candidate normally has a non-zero score (a word spanning two segments scores 0 but is still returned). Length normalisation keeps the 34K-char
//! run-reports from dominating.
//!
//! Build cost is one pass over every record and note, paid once per store write epoch where the
//! store has a cache slot (`FileStore`); other stores rebuild per ranked query. The index depends
//! only on the corpus, never on the query's filters.
//!
//! ponytail: document frequency is a substring scan over the cached normalized text
//! (O(corpus) per query word). Add a token-keyed inverted index if a corpus outgrows that.

use crate::error::RepositoryError;
use crate::record_store::{self, RecordListFilter};
use crate::store::RepositoryStore;
use crate::text_projection::{self, FieldTextIndex, TextSegment};
use std::collections::HashMap;
use std::fmt::Debug;

/// Orders discovery candidates. Implementations MUST be deterministic and MUST NOT
/// change the candidate set (ADR-019 decision 5).
pub trait DiscoveryIndex: Debug {
    /// One relevance score per candidate instance id, parallel to `candidates`; higher is
    /// better. `words` are already normalized ([`text_projection::normalize`]).
    fn score(&self, words: &[String], candidates: &[&str]) -> Vec<f32>;
}

const K1: f64 = 1.2;
const B: f64 = 0.75;
/// Field weights: label / title.
const W_TITLE: f32 = 4.0;
/// Tags.
const W_TAG: f32 = 2.0;
/// Short fields (statement, description, ...).
const W_SHORT: f32 = 2.0;
/// Long body fields (run-report text, note sections).
const W_BODY: f32 = 1.0;
/// A segment longer than this many chars is a body, not a statement/description.
const SHORT_MAX_CHARS: usize = 400;

#[derive(Debug)]
struct Doc {
    /// Whitespace word count over all segments: the BM25 document length.
    len: f32,
    /// `(weight, normalized text)` per segment.
    parts: Vec<(f32, String)>,
}

/// BM25 over every Tier 0 and Tier 2 instance, built once and cached per store epoch.
#[derive(Debug)]
pub struct Bm25Index {
    docs: Vec<Doc>,
    by_id: HashMap<String, usize>,
    avg_len: f32,
}

fn weight(seg: &TextSegment) -> f32 {
    match seg.field_name.as_str() {
        text_projection::LABEL_SENTINEL | text_projection::NOTE_TITLE_SENTINEL => W_TITLE,
        text_projection::TAG_SENTINEL => W_TAG,
        _ if seg.text.chars().count() <= SHORT_MAX_CHARS => W_SHORT,
        _ => W_BODY,
    }
}

impl Bm25Index {
    pub fn build(
        store: &dyn RepositoryStore,
        field_index: &FieldTextIndex,
    ) -> Result<Self, RepositoryError> {
        let mut docs = Vec::new();
        let mut by_id = HashMap::new();
        let mut add = |id: String, segments: Vec<TextSegment>| {
            let parts: Vec<(f32, String)> = segments
                .iter()
                .map(|s| (weight(s), text_projection::normalize(&s.text)))
                .collect();
            let len = parts
                .iter()
                .map(|(_, t)| t.split_whitespace().count())
                .sum::<usize>() as f32;
            by_id.insert(id, docs.len());
            docs.push(Doc { len, parts });
        };
        for record in record_store::list_records_filtered(store, RecordListFilter::default())? {
            add(
                record.instance_id.clone(),
                text_projection::project_text(&record, field_index),
            );
        }
        for entry in &store.catalog()?.instances {
            if entry.tier != Some(0) {
                continue;
            }
            let locator = entry.locator.as_deref().unwrap_or_default();
            let note = crate::store::note_from_value(store.load_instance_json(locator)?, locator)?;
            add(
                note.instance_id.clone(),
                text_projection::project_note_text(&note),
            );
        }
        let avg_len = (docs.iter().map(|d| d.len).sum::<f32>() / docs.len().max(1) as f32).max(1.0);
        Ok(Self {
            docs,
            by_id,
            avg_len,
        })
    }
}

impl DiscoveryIndex for Bm25Index {
    fn score(&self, words: &[String], candidates: &[&str]) -> Vec<f32> {
        // f64 internally and quantised on the way out, so wasm32 and native order near-ties alike.
        let n = self.docs.len() as f64;
        let idf: Vec<f64> = words
            .iter()
            .map(|w| {
                let df = self
                    .docs
                    .iter()
                    .filter(|d| d.parts.iter().any(|(_, t)| t.contains(w.as_str())))
                    .count() as f64;
                (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
            })
            .collect();
        candidates
            .iter()
            .map(|id| {
                let Some(doc) = self.by_id.get(*id).map(|&i| &self.docs[i]) else {
                    return 0.0;
                };
                let norm = K1 * (1.0 - B + B * f64::from(doc.len) / f64::from(self.avg_len));
                let raw: f64 = words
                    .iter()
                    .zip(&idf)
                    .map(|(w, idf)| {
                        let tf: f64 = doc
                            .parts
                            .iter()
                            .map(|(wt, t)| f64::from(*wt) * t.matches(w.as_str()).count() as f64)
                            .sum();
                        idf * tf * (K1 + 1.0) / (tf + norm)
                    })
                    .sum();
                ((raw * 1e4).round() / 1e4) as f32
            })
            .collect()
    }
}
