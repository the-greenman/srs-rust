//! # Package-export reference-site table (RFC-003 Change C, `mechanism-64469ada`)
//!
//! Every UUID-valued property of the ten definition schemas, with the strength
//! RFC-003 assigns it. The export closure ([C1]) follows PINNED and LINEAGE
//! sites; KEYED sites hold strings, not UUIDs, so they are not rows and are
//! never visited; LOCATOR and non-reference sites are rows that produce nothing.
//!
//! Derivation rule (RFC-003 Change C): a property is a reference site when it
//! holds the UUID of a definition; its strength is the one its schema text
//! gives (`rfc-decision-c8704763`). The Protocol `FieldRef.fieldId` row follows
//! that rule (finding F1, folded into srs PR #874).
//!
//! `table_matches_every_uuid_site_in_the_definition_schemas` walks the embedded
//! schemas and fails on any UUID site without a row or any row without a site:
//! the in-repo twin of the spec-side guard srs#873, which also carries the
//! proposal to derive this table from schema annotations instead (O2).

use serde_json::Value;

use crate::package_types::DefinitionKind::{self, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Strength {
    /// `{id, version}` pair; `version_key` is the sibling key holding the version.
    Pinned {
        version_key: &'static str,
    },
    Lineage,
    Locator,
    /// UUID-valued but not a definition reference (provenance, container ids, nested own ids).
    NotReference,
}

pub(crate) struct ReferenceSite {
    /// The referring definition's kind.
    pub kind: DefinitionKind,
    /// Pointer pattern: `*` = every array item, `{*}` = every object value.
    pub path: &'static str,
    /// `None` for Locator / NotReference.
    pub target: Option<DefinitionKind>,
    pub strength: Strength,
}

const fn site(
    kind: DefinitionKind,
    path: &'static str,
    target: Option<DefinitionKind>,
    strength: Strength,
) -> ReferenceSite {
    ReferenceSite {
        kind,
        path,
        target,
        strength,
    }
}

const TYPE_VERSION: Strength = Strength::Pinned {
    version_key: "typeVersion",
};
use Strength::{Lineage, Locator, NotReference};

pub(crate) const REFERENCE_SITES: &[ReferenceSite] = &[
    site(
        Field,
        "/fieldType/rangeType/typeId",
        Some(Type),
        TYPE_VERSION,
    ),
    site(Field, "/fieldType/vocabularyRef", Some(Vocabulary), Lineage),
    site(Field, "/lineage/sourceDefinitionId", None, NotReference),
    site(Field, "/lineage/forkedFromDefinitionId", None, NotReference),
    site(
        Type,
        "/extendsTypeId",
        Some(Type),
        Strength::Pinned {
            version_key: "extendsTypeVersion",
        },
    ),
    site(Type, "/fields/*/fieldId", Some(Field), Lineage),
    site(
        Type,
        "/fieldAssignmentOverrides/*/fieldId",
        Some(Field),
        Lineage,
    ),
    site(Type, "/fieldOrder/*", Some(Field), Lineage),
    site(Type, "/validationRules/*/fieldIds/*", Some(Field), Lineage),
    site(
        Type,
        "/validationRules/*/predicateFieldId",
        Some(Field),
        Lineage,
    ),
    site(
        Type,
        "/validationRules/*/targetFieldId",
        Some(Field),
        Lineage,
    ),
    site(Type, "/identityFieldId", Some(Field), Lineage),
    site(Type, "/lifecycleRef", Some(Lifecycle), Lineage),
    site(Type, "/lifecycle/states/*/id", None, NotReference),
    site(Type, "/lifecycle/transitions/*/id", None, NotReference),
    site(Type, "/lineage/sourceDefinitionId", None, NotReference),
    site(Type, "/lineage/forkedFromDefinitionId", None, NotReference),
    site(View, "/fieldViews/*/fieldId", Some(Field), Lineage),
    site(
        View,
        "/fieldViews/*/compositeRenderer/roles/{*}",
        Some(Field),
        Lineage,
    ),
    site(
        Composition,
        "/rootTypeRefs/*/typeId",
        Some(Type),
        TYPE_VERSION,
    ),
    site(
        Composition,
        "/compositeRenderers/*/fieldId",
        Some(Field),
        Lineage,
    ),
    site(
        Composition,
        "/compositeRenderers/*/roles/{*}",
        Some(Field),
        Lineage,
    ),
    site(
        Composition,
        "/sections/*/compositeRenderers/*/fieldId",
        Some(Field),
        Lineage,
    ),
    site(
        Composition,
        "/sections/*/compositeRenderers/*/roles/{*}",
        Some(Field),
        Lineage,
    ),
    site(
        Composition,
        "/sections/*/titleFieldId",
        Some(Field),
        Lineage,
    ),
    site(
        Composition,
        "/sections/*/ordering/fieldId",
        Some(Field),
        Lineage,
    ),
    site(Composition, "/sections/*/renderViewId", Some(View), Lineage),
    site(
        Composition,
        "/sections/*/typeDispatch/{*}",
        Some(View),
        Lineage,
    ),
    site(
        Composition,
        "/sections/*/source/query/typeId",
        Some(Type),
        Lineage,
    ),
    site(Composition, "/themeRef/themeId", None, Locator),
    site(
        Composition,
        "/themeVariants/*/themeRef/themeId",
        None,
        Locator,
    ),
    site(
        Composition,
        "/sections/*/source/containerId",
        None,
        NotReference,
    ),
    site(
        Composition,
        "/sections/*/source/containerIds/*",
        None,
        NotReference,
    ),
    site(
        Composition,
        "/sections/*/source/query/containerId",
        None,
        NotReference,
    ),
    site(
        Vocabulary,
        "/extendsVocabularyId",
        Some(Vocabulary),
        Strength::Pinned {
            version_key: "extendsVocabularyVersion",
        },
    ),
    site(Vocabulary, "/terms/*/id", None, NotReference),
    site(
        Lifecycle,
        "/extendsLifecycleId",
        Some(Lifecycle),
        Strength::Pinned {
            version_key: "extendsLifecycleVersion",
        },
    ),
    site(Lifecycle, "/states/*/id", None, NotReference),
    site(Lifecycle, "/transitions/*/id", None, NotReference),
    site(Theme, "/cssClassFields/*", Some(Field), Lineage),
    site(
        Theme,
        "/elementTemplates/recordWrapperOverrides/*/typeId",
        Some(Type),
        Lineage,
    ),
    site(Blueprint, "/rootTypes/*/typeId", Some(Type), TYPE_VERSION),
    site(
        Blueprint,
        "/requiredTypes/*/typeId",
        Some(Type),
        TYPE_VERSION,
    ),
    site(
        Blueprint,
        "/structure/*/sourceType/typeId",
        Some(Type),
        TYPE_VERSION,
    ),
    site(
        Blueprint,
        "/structure/*/targetType/typeId",
        Some(Type),
        TYPE_VERSION,
    ),
    site(Protocol, "/targetType", Some(Type), Lineage),
    site(Protocol, "/stages/*/outputType", Some(Type), Lineage),
    site(
        Protocol,
        "/stages/*/contributesTo/*/typeId",
        Some(Type),
        Lineage,
    ),
    site(
        Protocol,
        "/stages/*/contributesTo/*/fieldId",
        Some(Field),
        Lineage,
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FollowedReference {
    /// Concrete pointer, for diagnostics (`/fields/2/fieldId`).
    pub path: String,
    pub target: DefinitionKind,
    pub id: String,
    /// `Some` for PINNED with its version present; `None` for LINEAGE and for a
    /// PINNED site whose optional version is absent (PD4: every version held).
    pub version: Option<u64>,
}

/// Every PINNED and LINEAGE reference `value` (a definition of `kind`) makes.
/// Empty strings are skipped.
pub(crate) fn followed_references(kind: DefinitionKind, value: &Value) -> Vec<FollowedReference> {
    let mut out = Vec::new();
    for s in REFERENCE_SITES.iter().filter(|s| s.kind == kind) {
        let version_key = match s.strength {
            Strength::Pinned { version_key } => Some(version_key),
            Strength::Lineage => None,
            Strength::Locator | Strength::NotReference => continue,
        };
        let Some(target) = s.target else { continue };
        let segs: Vec<&str> = s.path.split('/').skip(1).collect();
        visit(
            value,
            value,
            &segs,
            String::new(),
            &mut |path, leaf, parent| {
                if let Some(id) = leaf.as_str().filter(|id| !id.is_empty()) {
                    out.push(FollowedReference {
                        path,
                        target,
                        id: id.to_string(),
                        version: version_key.and_then(|k| parent.get(k)?.as_u64()),
                    });
                }
            },
        );
    }
    out
}

/// Visit every leaf `pattern` matches, with its concrete pointer and the object holding it.
fn visit<'a>(
    v: &'a Value,
    parent: &'a Value,
    segs: &[&str],
    path: String,
    f: &mut impl FnMut(String, &'a Value, &'a Value),
) {
    let Some((seg, rest)) = segs.split_first() else {
        return f(path, v, parent);
    };
    match *seg {
        "*" => {
            for (i, x) in v.as_array().into_iter().flatten().enumerate() {
                visit(x, v, rest, format!("{path}/{i}"), f);
            }
        }
        "{*}" => {
            for (k, x) in v.as_object().into_iter().flatten() {
                let k = k.replace('~', "~0").replace('/', "~1");
                visit(x, v, rest, format!("{path}/{k}"), f);
            }
        }
        key => {
            if let Some(x) = v.get(key) {
                visit(x, v, rest, format!("{path}/{key}"), f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn schema_id(kind: DefinitionKind) -> &'static str {
        match kind {
            Field => srs_schema::FIELD_SCHEMA_ID,
            Type => srs_schema::TYPE_SCHEMA_ID,
            View => srs_schema::VIEW_SCHEMA_ID,
            Composition => srs_schema::COMPOSITION_SCHEMA_ID,
            RelationType => srs_schema::RELATION_TYPE_SCHEMA_ID,
            Blueprint => srs_schema::BLUEPRINT_SCHEMA_ID,
            Protocol => srs_schema::PROTOCOL_SCHEMA_ID,
            Vocabulary => srs_schema::VOCABULARY_SCHEMA_ID,
            Lifecycle => srs_schema::LIFECYCLE_SCHEMA_ID,
            Theme => srs_schema::THEME_SCHEMA_ID,
        }
    }

    /// Collect the pointer pattern of every `format: "uuid"` leaf of a schema.
    ///
    /// The guard keys on `format: "uuid"`: a UUID site whose schema omits the format
    /// annotation is invisible to it. A scan at srs-rust#1212 found no such site.
    fn uuid_sites(
        root: &Value,
        node: &Value,
        path: &str,
        seen: &mut Vec<String>,
        out: &mut BTreeSet<String>,
    ) {
        if let Some(r) = node.get("$ref").and_then(Value::as_str) {
            let ptr = r
                .strip_prefix('#')
                .unwrap_or_else(|| panic!("non-local $ref {r}"));
            if !seen.iter().any(|s| s == r) {
                seen.push(r.to_string());
                uuid_sites(root, root.pointer(ptr).unwrap(), path, seen, out);
                seen.pop();
            }
        }
        if node.get("format").and_then(Value::as_str) == Some("uuid") && !path.is_empty() {
            out.insert(path.to_string());
        }
        for (k, v) in node
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            uuid_sites(root, v, &format!("{path}/{k}"), seen, out);
        }
        if let Some(i) = node.get("items").filter(|i| i.is_object()) {
            uuid_sites(root, i, &format!("{path}/*"), seen, out);
        }
        if let Some(a) = node.get("additionalProperties").filter(|a| a.is_object()) {
            uuid_sites(root, a, &format!("{path}/{{*}}"), seen, out);
        }
        for v in node
            .get("patternProperties")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(_, v)| v)
        {
            uuid_sites(root, v, &format!("{path}/{{*}}"), seen, out);
        }
        for key in ["oneOf", "anyOf", "allOf"] {
            for v in node
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                uuid_sites(root, v, path, seen, out);
            }
        }
        for key in ["if", "then", "else"] {
            if let Some(v) = node.get(key) {
                uuid_sites(root, v, path, seen, out);
            }
        }
    }

    #[test]
    fn table_matches_every_uuid_site_in_the_definition_schemas() {
        let kinds = [
            Field,
            Type,
            View,
            Composition,
            RelationType,
            Blueprint,
            Protocol,
            Vocabulary,
            Lifecycle,
            Theme,
        ];
        let mut schema = BTreeSet::new();
        for kind in kinds {
            let root: Value =
                serde_json::from_str(srs_schema::schema_source(schema_id(kind)).unwrap()).unwrap();
            let mut sites = BTreeSet::new();
            uuid_sites(&root, &root, "", &mut Vec::new(), &mut sites);
            sites.remove("/id");
            schema.extend(sites.into_iter().map(|p| format!("{kind:?} {p}")));
        }
        let table: BTreeSet<String> = REFERENCE_SITES
            .iter()
            .map(|s| format!("{:?} {}", s.kind, s.path))
            .collect();
        assert_eq!(table.len(), REFERENCE_SITES.len(), "duplicate rows");
        let missing: Vec<_> = schema.difference(&table).collect();
        let stale: Vec<_> = table.difference(&schema).collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "missing rows: {missing:?}; stale rows: {stale:?}"
        );
    }

    fn ids(refs: &[FollowedReference]) -> Vec<(&str, &str, Option<u64>)> {
        refs.iter()
            .map(|r| (r.path.as_str(), r.id.as_str(), r.version))
            .collect()
    }

    #[test]
    fn pinned_site_returns_its_version() {
        let b = json!({"rootTypes": [{"typeId": "t1", "typeVersion": 2}]});
        assert_eq!(
            ids(&followed_references(Blueprint, &b)),
            vec![("/rootTypes/0/typeId", "t1", Some(2))]
        );
        let f = json!({"fieldType": {"rangeType": {"typeId": "t2", "typeVersion": 3}}});
        let r = followed_references(Field, &f);
        assert_eq!(
            ids(&r),
            vec![("/fieldType/rangeType/typeId", "t2", Some(3))]
        );
        assert_eq!(r[0].target, Type);
    }

    #[test]
    fn lineage_site_returns_no_version() {
        let t = json!({"fields": [{"fieldId": "f1", "order": 0}, {"fieldId": "f2"}]});
        assert_eq!(
            ids(&followed_references(Type, &t)),
            vec![
                ("/fields/0/fieldId", "f1", None),
                ("/fields/1/fieldId", "f2", None)
            ]
        );
        let p = json!({"stages": [{"contributesTo": [{"fieldId": "f3"}]}]});
        let r = followed_references(Protocol, &p);
        assert_eq!(
            ids(&r),
            vec![("/stages/0/contributesTo/0/fieldId", "f3", None)]
        );
        assert_eq!(r[0].target, Field);
    }

    #[test]
    fn wildcards_match_arrays_and_object_values() {
        let c = json!({"sections": [{"typeDispatch": {"com.x/a": "v1", "com.x/b": "v2"}}]});
        let r = followed_references(Composition, &c);
        assert_eq!(
            ids(&r),
            vec![
                ("/sections/0/typeDispatch/com.x~1a", "v1", None),
                ("/sections/0/typeDispatch/com.x~1b", "v2", None)
            ]
        );
        assert!(r.iter().all(|x| x.target == View));
        let v = json!({"fieldViews": [{"fieldId": "f", "compositeRenderer": {"roles": {"label": "f9"}}}]});
        let got: BTreeSet<String> = followed_references(View, &v)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(got, BTreeSet::from(["f".to_string(), "f9".to_string()]));
    }

    #[test]
    fn locator_and_non_reference_sites_are_not_followed() {
        let c = json!({"themeRef": {"themeId": "th"}, "themeVariants": [{"themeRef": {"themeId": "th2"}}],
            "sections": [{"source": {"containerId": "c1", "containerIds": ["c2"], "query": {"containerId": "c3"}}}]});
        assert!(followed_references(Composition, &c).is_empty());
        let f = json!({"lineage": {"sourceDefinitionId": "x", "forkedFromDefinitionId": "y"}});
        assert!(followed_references(Field, &f).is_empty());
        let l = json!({"states": [{"id": "s"}], "transitions": [{"id": "t"}]});
        assert!(followed_references(Lifecycle, &l).is_empty());
    }

    #[test]
    fn keyed_strings_are_not_followed() {
        let b = json!({"structure": [{"relationType": "depends-on",
            "sourceType": {"typeId": "a", "typeVersion": 1}, "targetType": {"typeId": "b", "typeVersion": 1}}]});
        let got: Vec<String> = followed_references(Blueprint, &b)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(got, vec!["a".to_string(), "b".to_string()]);
        let c = json!({"sections": [{"typeDispatch": {"key-type-uuid": "view"}}]});
        let got: Vec<String> = followed_references(Composition, &c)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(got, vec!["view".to_string()]);
    }

    #[test]
    fn empty_value_is_not_a_reference() {
        assert!(followed_references(Protocol, &json!({"targetType": ""})).is_empty());
    }

    #[test]
    fn pinned_without_version_falls_back_to_lineage() {
        let t = json!({"extendsTypeId": "base"});
        assert_eq!(
            ids(&followed_references(Type, &t)),
            vec![("/extendsTypeId", "base", None)]
        );
        let t = json!({"extendsTypeId": "base", "extendsTypeVersion": 4});
        assert_eq!(
            ids(&followed_references(Type, &t)),
            vec![("/extendsTypeId", "base", Some(4))]
        );
    }
}
