//! `type subtypes`: the transitive subtype set of a Type (ext:type-inheritance,
//! RFC-032), computed once in the core so no client walks `extendsTypeId` itself
//! (srs-rust#1147; layer rules, rfc-decision-9ee14517, rule 2 "consume, don't clone").
//!
//! A pure read over the resolved package, using the same cycle-safe closure
//! [`crate::package::Package::subtypes_of`] that the blueprint schema uses. An
//! ancestor check follows from the result: `isAncestorOrSelf(a, b)` is
//! `b` being `a` (with `include_self`) or a member of `subtypes_of(a)`.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use std::collections::BTreeMap;

/// Input contract for [`type_subtypes`].
#[derive(Debug, Clone)]
pub struct TypeSubtypesInput {
    /// The Type whose subtypes are wanted.
    pub type_id: String,
    /// Also list the Type itself (as the first-sorted entry by id, like any other).
    pub include_self: bool,
}

/// One Type in the closure. A Type with several versions appears once, at its latest.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeRef {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
}

/// Output contract for [`type_subtypes`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeSubtypesResult {
    pub type_id: String,
    /// Sorted by type id, so the output is deterministic.
    pub subtypes: Vec<TypeRef>,
}

/// Return every Type whose `extendsTypeId` chain reaches `input.type_id`.
///
/// An unknown `type_id` is `RepositoryError::DefinitionNotFound`
/// (`definition-not-found`, ADR-053).
pub fn type_subtypes(
    store: &dyn RepositoryStore,
    input: TypeSubtypesInput,
) -> Result<TypeSubtypesResult, RepositoryError> {
    let package = store.load_package()?;
    if !package
        .record_types()
        .iter()
        .any(|rt| rt.id == input.type_id)
    {
        return Err(RepositoryError::DefinitionNotFound { id: input.type_id });
    }

    // Keyed by id so versions collapse to the latest and the order is by id.
    let mut by_id: BTreeMap<&str, &srs_core::types::record_type::RecordType> = BTreeMap::new();
    for rt in package.subtypes_of(&input.type_id) {
        match by_id.get(rt.id.as_str()) {
            Some(existing) if existing.version >= rt.version => {}
            _ => {
                by_id.insert(rt.id.as_str(), rt);
            }
        }
    }
    if input.include_self {
        if let Some(rt) = package
            .record_types()
            .iter()
            .filter(|rt| rt.id == input.type_id)
            .max_by_key(|rt| rt.version)
        {
            by_id.insert(rt.id.as_str(), rt);
        }
    }

    let subtypes = by_id
        .values()
        .map(|rt| TypeRef {
            id: rt.id.clone(),
            namespace: rt.namespace.clone(),
            name: rt.name.clone(),
            version: rt.version,
        })
        .collect();
    Ok(TypeSubtypesResult {
        type_id: input.type_id,
        subtypes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use srs_core::types::record_type::RecordType;

    fn rt(id: &str, parent: Option<&str>, version: u32) -> RecordType {
        RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: id.to_string(),
            version,
            description: "test".to_string(),
            fields: vec![],
            extends_type_id: parent.map(str::to_string),
            extends_type_version: parent.map(|_| 1),
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        }
    }

    fn store_with(types: Vec<RecordType>) -> MemoryStore {
        let empty = MemoryStore::empty();
        let manifest = empty.load_manifest().unwrap();
        let mut package = empty.load_package().unwrap();
        package.record_types = types;
        MemoryStore::new(manifest, package)
    }

    fn ids(r: &TypeSubtypesResult) -> Vec<&str> {
        r.subtypes.iter().map(|t| t.id.as_str()).collect()
    }

    fn input(id: &str, include_self: bool) -> TypeSubtypesInput {
        TypeSubtypesInput {
            type_id: id.to_string(),
            include_self,
        }
    }

    #[test]
    fn chain_of_three_is_transitive_and_sorted() {
        let store = store_with(vec![
            rt("c", Some("b"), 1),
            rt("b", Some("a"), 1),
            rt("a", None, 1),
        ]);
        assert_eq!(
            ids(&type_subtypes(&store, input("a", false)).unwrap()),
            ["b", "c"]
        );
        assert_eq!(
            ids(&type_subtypes(&store, input("b", false)).unwrap()),
            ["c"]
        );
        assert!(type_subtypes(&store, input("c", false))
            .unwrap()
            .subtypes
            .is_empty());
    }

    #[test]
    fn include_self_lists_the_type_in_id_order() {
        let store = store_with(vec![rt("b", Some("a"), 1), rt("a", None, 1)]);
        assert_eq!(
            ids(&type_subtypes(&store, input("a", true)).unwrap()),
            ["a", "b"]
        );
    }

    #[test]
    fn cycle_terminates() {
        let store = store_with(vec![rt("x", Some("y"), 1), rt("y", Some("x"), 1)]);
        let r = type_subtypes(&store, input("x", false)).unwrap();
        assert_eq!(ids(&r), ["y"]);
    }

    #[test]
    fn versions_collapse_to_the_latest() {
        let store = store_with(vec![
            rt("a", None, 1),
            rt("b", Some("a"), 1),
            rt("b", Some("a"), 2),
        ]);
        let r = type_subtypes(&store, input("a", false)).unwrap();
        assert_eq!(r.subtypes.len(), 1);
        assert_eq!(r.subtypes[0].version, 2);
    }

    #[test]
    fn unknown_type_is_a_coded_error() {
        let store = store_with(vec![rt("a", None, 1)]);
        let err = type_subtypes(&store, input("nope", false)).unwrap_err();
        assert_eq!(err.code(), "definition-not-found");
    }
}
