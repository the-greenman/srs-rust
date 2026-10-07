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
                type_id: (i % 2 == 0).then(|| format!("t{i}")),
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

// ── render projection mirrors (#1261) ────────────────────────────────────────

use srs::payload::{
    CompositionProjection, ExportBundlePayload, OkfBundlePayload, RenderCompositionPayload,
};
use srs_repository::render_service as svc;

fn svc_record(full: bool, children: Vec<svc::ProjectedRecord>) -> svc::ProjectedRecord {
    svc::ProjectedRecord {
        instance_id: "i".into(),
        type_id: "t".into(),
        type_version: 1,
        type_namespace: "ns".into(),
        type_name: "n".into(),
        record_heading: full.then(|| "H".into()),
        preamble: full.then(|| "P".into()),
        fields: serde_json::json!({"a": 1}),
        ordered_field_keys: vec!["a".into()],
        relations: full.then(|| {
            vec![svc::ProjectedRelationRow {
                relation_type: "depends-on".into(),
                direction: svc::ProjectedRelationDirection::Inverse,
                label: "L".into(),
                targets: vec![svc::ProjectedRelationTarget {
                    instance_id: "x".into(),
                    display_label: "X".into(),
                }],
            }]
        }),
        properties: full.then(|| {
            vec![svc::ProjectedPropertyRow {
                property: srs_core::types::view::RecordProperty::Tags,
                label: "Tags".into(),
                value: svc::ProjectedPropertyValue::List(vec!["a".into()]),
            }]
        }),
        children,
        depth: full.then_some(2),
    }
}

fn svc_projection(full: bool) -> svc::CompositionProjection {
    let section = |sections| svc::ProjectedSection {
        section_id: "s".into(),
        title: full.then(|| "T".into()),
        order: 1,
        records: vec![svc_record(
            full,
            if full {
                vec![svc_record(false, vec![])]
            } else {
                vec![]
            },
        )],
        sections,
    };
    svc::CompositionProjection {
        schema: "s".into(),
        composition_id: "c".into(),
        container_id: None,
        generated_at: "now".into(),
        container_title: "CT".into(),
        preamble: full.then(|| "pre".into()),
        sections: vec![section(if full { vec![section(vec![])] } else { vec![] })],
    }
}

fn golden(name: &str) -> jsonschema::Validator {
    let p = format!("{}/schemas/payload/{name}.json", env!("CARGO_MANIFEST_DIR"));
    jsonschema::validator_for(&serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
        .unwrap()
}

#[test]
fn render_projection_mirror_matches_service_serialization() {
    for full in [true, false] {
        let p = svc_projection(full);
        let direct = to_value(&p).unwrap();
        assert_eq!(direct, to_value(CompositionProjection::from(p)).unwrap());
    }
}

/// Keys serde skips (empty diagnostics/children/sections, absent optionals) must not be
/// `required` in the goldens: real output with them absent must validate.
#[test]
fn render_goldens_accept_output_with_skipped_keys_absent() {
    for full in [true, false] {
        let v = to_value(RenderCompositionPayload {
            rendered: "r".into(),
            diagnostics: vec![],
            projection: Some(svc_projection(full).into()),
        })
        .unwrap();
        if !full {
            let s = v.to_string();
            assert!(!s.contains("\"children\"") && !s.contains("\"sections\":[]"));
        }
        let g = golden("render-composition");
        assert!(g.is_valid(&v), "{:?}", g.iter_errors(&v).next());
    }
    let v = to_value(RenderCompositionPayload {
        rendered: "r".into(),
        diagnostics: vec![],
        projection: None,
    })
    .unwrap();
    assert!(golden("render-composition").is_valid(&v));
    let v = to_value(ExportBundlePayload {
        rendered_filename: "f".into(),
        attachment_count: 0,
        output_path: "o".into(),
        diagnostics: vec![],
    })
    .unwrap();
    assert!(v.get("diagnostics").is_none());
    assert!(golden("render-export-bundle").is_valid(&v));
    let v = to_value(OkfBundlePayload {
        file_count: 1,
        output_dir: "d".into(),
        diagnostics: vec![],
    })
    .unwrap();
    assert!(v.get("diagnostics").is_none());
    assert!(golden("render-okf-bundle").is_valid(&v));
}
