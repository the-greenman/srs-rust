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

    /// The `limit` terms most characteristic of instance `id`, best first (srs-rust#1230,
    /// "more like this"); `None` if the index does not know `id`. Same determinism contract.
    fn top_terms(&self, id: &str, limit: usize) -> Option<Vec<String>>;

    /// Share of indexed instances (0.0–1.0) containing `term` as a whole token
    /// ([`tokens`]); 0.0 for an unknown term or an empty index (srs-rust#1284).
    fn token_document_fraction(&self, term: &str) -> f64;
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
    /// Documents containing each whole token (not substring): idf for [`DiscoveryIndex::top_terms`].
    token_df: HashMap<String, usize>,
}

/// Shortest token that can be a similarity term (drops "of", "a", "the"-sized noise).
pub(crate) const MIN_TERM_CHARS: usize = 3;

/// The index's tokens of already-normalized `text`: alphanumeric runs of at least
/// [`MIN_TERM_CHARS`] chars. Shared with any-term matching (srs-rust#1284).
pub(crate) fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= MIN_TERM_CHARS)
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
        let mut token_df: HashMap<String, usize> = HashMap::new();
        for doc in &docs {
            let distinct: std::collections::HashSet<&str> =
                doc.parts.iter().flat_map(|(_, t)| tokens(t)).collect();
            for t in distinct {
                *token_df.entry(t.to_string()).or_default() += 1;
            }
        }
        let avg_len = (docs.iter().map(|d| d.len).sum::<f32>() / docs.len().max(1) as f32).max(1.0);
        Ok(Self {
            docs,
            by_id,
            avg_len,
            token_df,
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

    fn token_document_fraction(&self, term: &str) -> f64 {
        if self.docs.is_empty() {
            return 0.0;
        }
        self.token_df.get(term).copied().unwrap_or(0) as f64 / self.docs.len() as f64
    }

    fn top_terms(&self, id: &str, limit: usize) -> Option<Vec<String>> {
        let doc = &self.docs[*self.by_id.get(id)?];
        let n = self.docs.len() as f64;
        let mut tf: HashMap<&str, f64> = HashMap::new();
        for (wt, text) in &doc.parts {
            for t in tokens(text) {
                *tf.entry(t).or_default() += f64::from(*wt);
            }
        }
        let mut ranked: Vec<(i64, &str)> = tf
            .into_iter()
            .map(|(t, w)| {
                let df = self.token_df.get(t).copied().unwrap_or(1) as f64;
                let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
                // Quantised so wasm32 and native pick the same terms on near-ties.
                (-((w * (K1 + 1.0) / (w + K1) * idf * 1e4).round() as i64), t)
            })
            .collect();
        ranked.sort(); // best score first, ties by term
        Some(
            ranked
                .into_iter()
                .take(limit)
                .map(|(_, t)| t.to_string())
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_terms_prefers_rare_title_terms() {
        let doc = |t: &str, body: &str| Doc {
            len: 4.0,
            parts: vec![(W_TITLE, t.into()), (W_BODY, body.into())],
        };
        let docs = vec![
            doc("zebra", "common common common"),
            doc("other", "common words"),
            doc("third", "common"),
        ];
        let mut token_df = HashMap::new();
        for d in &docs {
            for t in d.parts.iter().flat_map(|(_, t)| tokens(t)) {
                token_df.entry(t.to_string()).or_insert(0);
            }
        }
        token_df.insert("zebra".into(), 1);
        token_df.insert("common".into(), 3);
        let by_id = [("a", 0), ("b", 1), ("c", 2)]
            .map(|(k, v)| (k.to_string(), v))
            .into();
        let index = Bm25Index {
            docs,
            by_id,
            avg_len: 4.0,
            token_df,
        };
        assert_eq!(index.top_terms("a", 1), Some(vec!["zebra".to_string()]));
        assert_eq!(index.top_terms("missing", 1), None);
    }
}
