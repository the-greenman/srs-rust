//! Parity gate for the `find` / `relation neighbours` payload mirrors (ADR-048 rule 3):
//! serializing a service value through its mirror must equal serializing it directly,
//! so serde-attribute drift on the service types (skip_serializing_if, flatten, rename)
//! fails here. Field add/remove is a compile error in the mirrors' `From` impls.
use serde_json::to_value;
use srs::payload::{DiscoveryResultPayload, NeighboursResultPayload};
use srs_repository::context_query_service::{
    EdgeDirection, NeighbourEdge, NeighbourSummary, NeighboursResult,
};
use srs_repository::discovery_service::{
    DiscoveryFacets, DiscoveryHit, DiscoveryResult, FacetCount, FacetCounts, FieldFacet,
};

fn counts(n: usize, other: usize) -> FacetCounts {
    FacetCounts {
        values: (0..n)
            .map(|i| FacetCount {
                value: format!("v{i}"),
                count: i + 1,
            })
            .collect(),
        other,
    }
}

fn hit(full: bool) -> DiscoveryHit {
    let s = |x: &str| full.then(|| x.to_string());
    DiscoveryHit {
        instance_id: "i".into(),
        uri: "srs://r/record/i".into(),
        label: "L".into(),
        type_id: s("t"),
        container_ids: if full { vec!["c".into()] } else { vec![] },
        type_namespace: s("ns"),
        type_name: s("n"),
        lifecycle_state: s("draft"),
        score: full.then_some(1.5),
        snippet: s("…snip…"),
        matched_fields: if full { vec!["f".into()] } else { vec![] },
    }
}

fn find_roundtrip(r: DiscoveryResult) {
    let direct = to_value(&r).unwrap();
    let mirrored = to_value(DiscoveryResultPayload::from(r)).unwrap();
    assert_eq!(direct, mirrored);
}

#[test]
fn find_full_and_empty_facets_match() {
    find_roundtrip(DiscoveryResult {
        hits: vec![hit(true), hit(false)],
        total: 2,
        facets: DiscoveryFacets {
            by_type: counts(2, 3),
            notes: 4,
            tags: counts(1, 0),
            fields: vec![FieldFacet {
                field: "status".into(),
                counts: counts(2, 1),
            }],
        },
        diagnostics: vec!["d".into()],
    });
    find_roundtrip(DiscoveryResult {
        hits: vec![],
        total: 0,
        facets: DiscoveryFacets::default(),
        diagnostics: vec![],
    });
}

#[test]
fn neighbours_both_directions_match() {
    let n = |full: bool| NeighbourSummary {
        instance_id: "n".into(),
        uri: "srs://r/record/n".into(),
        label: full.then(|| "L".into()),
        type_namespace: full.then(|| "ns".into()),
        type_name: full.then(|| "t".into()),
    };
    let r = NeighboursResult {
        instance_id: "i".into(),
        total: 2,
        neighbours: vec![
            NeighbourEdge {
                direction: EdgeDirection::Out,
                relation_id: "r1".into(),
                relation_type: "depends-on".into(),
                neighbour: n(true),
            },
            NeighbourEdge {
                direction: EdgeDirection::In,
                relation_id: "r2".into(),
                relation_type: "refines".into(),
                neighbour: n(false),
            },
        ],
    };
    let direct = to_value(&r).unwrap();
    assert_eq!(direct, to_value(NeighboursResultPayload::from(r)).unwrap());
}
