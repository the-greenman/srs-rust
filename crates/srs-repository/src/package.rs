use srs_core::types::blueprint::Blueprint;
use srs_core::types::field::Field;
use srs_core::types::lifecycle::{Lifecycle, LifecycleState, LifecycleTransition};
use srs_core::types::protocol::Protocol;
use srs_core::types::record_type::{FieldAssignment, RecordType};
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::term::Term;
use srs_core::types::theme::Theme;
use srs_core::types::view::{DocumentView, View};
use srs_core::types::vocabulary::Vocabulary;
use std::path::PathBuf;

/// A loaded package containing field definitions, record types, views, themes, blueprints, and protocols.
///
/// The `root` field contains the repository root path (not the package/ subdirectory).
#[derive(Debug, Clone)]
pub struct Package {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub fields: Vec<Field>,
    pub record_types: Vec<RecordType>,
    pub relation_type_definitions: Vec<RelationTypeDefinition>,
    pub views: Vec<View>,
    pub document_views: Vec<DocumentView>,
    pub themes: Vec<Theme>,
    pub blueprints: Vec<LoadedBlueprint>,
    pub protocols: Vec<LoadedProtocol>,
    pub root: PathBuf,
    /// ext:type-inheritance — external package dependencies declared in dependencyRefs.
    pub dependency_refs: Vec<DependencyRef>,
    pub vocabularies: Vec<Vocabulary>,
    pub lifecycles: Vec<Lifecycle>,
}

/// A protocol as loaded from a package, bundling typed struct + verbatim JSON.
///
/// `raw` preserves all fields from the on-disk JSON that are not already captured
/// by the typed `Protocol` struct (e.g. `ai_guidance`, which retains `serde_json::Value`
/// as its shape is unspecified by the spec). `source_package` is `None` for the root
/// package and `Some` for protocols merged from a dependency package.
#[derive(Debug, Clone)]
pub struct LoadedProtocol {
    pub protocol: Protocol,
    pub raw: serde_json::Value,
    pub source_package: Option<String>,
}

/// A blueprint as loaded from a package, tracking sub-package provenance.
///
/// `source_package` is `None` for the root package and `Some(rel_path)` for
/// blueprints merged from a dependency package. No `raw` field: unlike protocols,
/// no blueprint CLI command returns an opaque verbatim payload.
#[derive(Debug, Clone)]
pub struct LoadedBlueprint {
    pub blueprint: Blueprint,
    pub source_package: Option<String>,
}

/// ext:type-inheritance — a declared external package dependency reference.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyRef {
    pub namespace: String,
    pub name: String,
    pub version: String,
}

/// Unified view of a resolved lifecycle — returned by `Package::effective_lifecycle`.
/// Borrows from either an inline `TypeLifecycle` or a standalone `Lifecycle`, depending
/// on which the RecordType uses.
#[derive(Debug)]
pub struct EffectiveLifecycle<'a> {
    pub initial_state: &'a str,
    pub states: &'a [LifecycleState],
    pub transitions: &'a [LifecycleTransition],
}
impl Package {
    /// Resolve a relation type definition by its UUID id.
    pub fn resolve_relation_type_by_id(&self, id: &str) -> Option<&RelationTypeDefinition> {
        self.relation_type_definitions.iter().find(|rt| rt.id == id)
    }

    /// Resolve a relation type definition by its relationType string.
    pub fn resolve_relation_type(&self, relation_type: &str) -> Option<&RelationTypeDefinition> {
        self.relation_type_definitions
            .iter()
            .find(|rt| rt.key == relation_type)
    }

    /// Get all relation type definitions as a slice.
    pub fn relation_types(&self) -> &[RelationTypeDefinition] {
        &self.relation_type_definitions
    }

    /// Resolve a view by its UUID id.
    pub fn resolve_view(&self, id: &str) -> Option<&View> {
        self.views.iter().find(|v| v.id == id)
    }

    /// Resolve a document view by its UUID id.
    pub fn resolve_document_view(&self, id: &str) -> Option<&DocumentView> {
        self.document_views.iter().find(|v| v.id == id)
    }

    /// Resolve a theme by its UUID id.
    pub fn resolve_theme(&self, theme_id: &str) -> Option<&Theme> {
        self.themes.iter().find(|theme| theme.id == theme_id)
    }

    /// Get all themes as a slice.
    pub fn themes(&self) -> &[Theme] {
        &self.themes
    }

    /// Resolve a record type by its ID and version.
    /// Effective field set resolved to Field definitions — the input RFC-039
    /// carrier validation consumes ([R1]/[R3]/[R5]): each assignment joined to
    /// its Field's `name` (the verbatim carrier key, [R2b]) and `fieldType`.
    /// An unresolvable `fieldId` yields an entry with an empty name and no
    /// `fieldType`; definition-level validation reports the dangling id.
    pub fn resolved_effective_fields(
        &self,
        record_type: &RecordType,
    ) -> Result<Vec<srs_core::validation::value_shape::EffectiveField>, crate::error::RepositoryError>
    {
        let assignments = self.effective_fields(record_type)?;
        Ok(assignments
            .into_iter()
            .map(|fa| {
                let field = self.fields.iter().find(|f| f.id == fa.field_id);
                srs_core::validation::value_shape::EffectiveField {
                    field_id: fa.field_id,
                    name: field.map(|f| f.name.clone()).unwrap_or_default(),
                    required: fa.required,
                    order: fa.order,
                    field_type: field.map(|f| f.field_type.clone()),
                }
            })
            .collect())
    }

    pub fn resolve_type(&self, type_id: &str, version: u32) -> Option<&RecordType> {
        self.record_types
            .iter()
            .find(|rt| rt.id == type_id && rt.version == version)
    }

    /// Resolve a record type by its namespace and name.
    ///
    /// This is the preferred lookup method as it avoids hardcoding UUIDs in tests.
    pub fn resolve_type_by_name(&self, namespace: &str, name: &str) -> Option<&RecordType> {
        self.record_types
            .iter()
            .find(|rt| rt.namespace == namespace && rt.name == name)
    }

    /// Resolve a field by its ID.
    pub fn resolve_field(&self, field_id: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == field_id)
    }

    /// Find a field by its name.
    pub fn find_field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Namespace-qualified field lookup. Prefer this over `find_field_by_name`
    /// whenever the package may carry same-named fields in multiple namespaces
    /// (e.g. `governance/title` vs the implicit-core `com.semanticops.core/title`).
    pub fn find_field(&self, namespace: &str, name: &str) -> Option<&Field> {
        self.fields
            .iter()
            .find(|f| f.namespace == namespace && f.name == name)
    }

    /// Get all fields as a slice.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Get all record types as a slice.
    pub fn record_types(&self) -> &[RecordType] {
        &self.record_types
    }

    /// ext:type-inheritance — resolve the ancestor chain for a RecordType, self first,
    /// walking `extendsTypeId` up through each ancestor with cycle detection (Inv 39).
    ///
    /// Returns `[record_type, parent, grandparent, ..., root]`. For a non-inheriting
    /// type, returns `[record_type]`. Shared by [`effective_fields`] and
    /// [`effective_identity_field_id`] so both walk the chain identically.
    fn ancestor_chain<'a>(
        &'a self,
        record_type: &'a RecordType,
    ) -> Result<Vec<&'a RecordType>, crate::error::RepositoryError> {
        use crate::error::RepositoryError;
        use std::collections::HashSet;

        let mut chain: Vec<&RecordType> = vec![record_type];
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(record_type.id.clone());

        let mut current = record_type;
        while let Some(extends_type_id) = &current.extends_type_id {
            let extends_version = current.extends_type_version.unwrap_or(1);

            if visited.contains(extends_type_id) {
                return Err(RepositoryError::TypeInheritanceCycle {
                    type_id: extends_type_id.clone(),
                });
            }
            let base = self
                .resolve_type(extends_type_id, extends_version)
                .ok_or_else(|| RepositoryError::TypeNotFound {
                    type_id: extends_type_id.clone(),
                    version: extends_version,
                })?;
            visited.insert(extends_type_id.clone());
            chain.push(base);
            current = base;
        }

        Ok(chain)
    }

    /// ext:type-inheritance — resolve the effective field list for a RecordType.
    ///
    /// For non-inheriting types, returns a clone of `record_type.fields` sorted by `order`.
    /// For inheriting types, walks the chain, merges base + own fields (Inv 39-42),
    /// and applies `fieldOrder` and `fieldAssignmentOverrides` if present.
    pub fn effective_fields(
        &self,
        record_type: &RecordType,
    ) -> Result<Vec<FieldAssignment>, crate::error::RepositoryError> {
        use crate::error::RepositoryError;
        use std::collections::HashSet;

        if record_type.extends_type_id.is_none() {
            let mut fields = record_type.fields.clone();
            fields.sort_by_key(|fa| fa.order);
            return Ok(fields);
        }

        // ancestor_chain returns [self, parent, ..., root]; ancestors in root-to-parent
        // order (oldest first) is chain[1..].rev() — matches the original chain.reverse()
        // + chain[..chain.len()-1] derivation exactly, just expressed over &RecordType
        // instead of pre-cloned Vec<FieldAssignment> per level.
        let chain = self.ancestor_chain(record_type)?;

        let mut merged: Vec<FieldAssignment> = Vec::new();
        let own_field_ids: HashSet<String> = record_type
            .fields
            .iter()
            .map(|fa| fa.field_id.clone())
            .collect();

        let mut seen_ids: HashSet<String> = HashSet::new();
        for ancestor in chain[1..].iter().rev() {
            for fa in &ancestor.fields {
                // Inv 40: own fields must not duplicate inherited fields
                if own_field_ids.contains(&fa.field_id) {
                    return Err(RepositoryError::InheritedFieldDuplicate {
                        type_id: record_type.id.clone(),
                        base_type_id: "ancestor".to_string(),
                        field_id: fa.field_id.clone(),
                    });
                }
                if seen_ids.insert(fa.field_id.clone()) {
                    merged.push(fa.clone());
                }
            }
        }
        // Add own fields
        let mut own_fields = record_type.fields.clone();
        own_fields.sort_by_key(|fa| fa.order);
        for fa in own_fields {
            seen_ids.insert(fa.field_id.clone());
            merged.push(fa);
        }

        // Inv 42: apply fieldAssignmentOverrides
        if let Some(overrides) = &record_type.field_assignment_overrides {
            for ovr in overrides {
                if own_field_ids.contains(&ovr.field_id) {
                    // Override targets an own field, not an inherited one
                    return Err(RepositoryError::OverrideTargetsOwnField {
                        type_id: record_type.id.clone(),
                        field_id: ovr.field_id.clone(),
                    });
                }
                let fa = merged.iter_mut().find(|fa| fa.field_id == ovr.field_id);
                match fa {
                    None => {
                        // Override targets a field that is neither inherited nor owned — Inv 42
                        return Err(RepositoryError::OverrideTargetsOwnField {
                            type_id: record_type.id.clone(),
                            field_id: ovr.field_id.clone(),
                        });
                    }
                    Some(fa) => {
                        if ovr.required == Some(false) && fa.required {
                            return Err(RepositoryError::OverrideRelaxesRequired {
                                type_id: record_type.id.clone(),
                                field_id: ovr.field_id.clone(),
                            });
                        }
                        if let Some(req) = ovr.required {
                            fa.required = req;
                        }
                        if let Some(label) = &ovr.display_label {
                            fa.display_label = Some(label.clone());
                        }
                    }
                }
            }
        }

        // Inv 41: apply fieldOrder if present
        if let Some(field_order) = &record_type.field_order {
            let effective_ids: HashSet<&str> =
                merged.iter().map(|fa| fa.field_id.as_str()).collect();

            // Detect duplicates in fieldOrder
            let mut seen_in_order: HashSet<&str> = HashSet::new();
            for fid in field_order {
                if !seen_in_order.insert(fid.as_str()) {
                    return Err(RepositoryError::FieldOrderMismatch {
                        type_id: record_type.id.clone(),
                        field_id: fid.clone(),
                    });
                }
            }

            // Every effective field must appear in fieldOrder (no missing fields)
            for fa in &merged {
                if !seen_in_order.contains(fa.field_id.as_str()) {
                    return Err(RepositoryError::FieldOrderMismatch {
                        type_id: record_type.id.clone(),
                        field_id: fa.field_id.clone(),
                    });
                }
            }

            // fieldOrder must not reference unknown fields (not in effective set).
            for fid in field_order {
                if !effective_ids.contains(fid.as_str()) {
                    return Err(RepositoryError::FieldOrderMismatch {
                        type_id: record_type.id.clone(),
                        field_id: fid.clone(),
                    });
                }
            }

            // Reorder merged according to fieldOrder
            let mut reordered: Vec<FieldAssignment> = Vec::with_capacity(merged.len());
            for fid in field_order {
                if let Some(pos) = merged.iter().position(|fa| &fa.field_id == fid) {
                    reordered.push(merged.remove(pos));
                }
            }
            return Ok(reordered);
        }

        Ok(merged)
    }

    /// RFC-020 — resolve a RecordType's effective `identityFieldId`, cascading the
    /// ext:type-inheritance ancestor chain (Rule [N+34]).
    ///
    /// A Type's own `identityFieldId`, if declared, wins immediately (no chain walk).
    /// Otherwise, returns the nearest ancestor's own `identityFieldId`, resolved
    /// transitively up the chain. Returns `Ok(None)` if no Type in the chain declares
    /// one. This inheritance rule cascades, unlike `fieldOrder`/`effective_fields`,
    /// which only look at the resolving Type itself.
    pub fn effective_identity_field_id(
        &self,
        record_type: &RecordType,
    ) -> Result<Option<String>, crate::error::RepositoryError> {
        if let Some(field_id) = &record_type.identity_field_id {
            return Ok(Some(field_id.clone()));
        }

        let chain = self.ancestor_chain(record_type)?;
        Ok(chain.iter().find_map(|rt| rt.identity_field_id.clone()))
    }

    /// Resolve a Vocabulary by its UUID id.
    pub fn resolve_vocabulary(&self, id: &str) -> Option<&Vocabulary> {
        self.vocabularies.iter().find(|v| v.id == id)
    }

    /// Resolve a Lifecycle by its UUID id.
    pub fn resolve_lifecycle(&self, id: &str) -> Option<&Lifecycle> {
        self.lifecycles.iter().find(|lc| lc.id == id)
    }

    /// Resolve a Lifecycle by namespace and name.
    pub fn resolve_lifecycle_by_name(&self, namespace: &str, name: &str) -> Option<&Lifecycle> {
        self.lifecycles
            .iter()
            .find(|lc| lc.namespace == namespace && lc.name == name)
    }

    /// Resolve a Term by vocabulary id and key (or alias).
    pub fn resolve_term_by_key(&self, vocabulary_id: &str, key: &str) -> Option<&Term> {
        self.resolve_vocabulary(vocabulary_id)
            .and_then(|v| v.resolve_term_by_key(key))
    }

    /// Resolve the effective lifecycle for a RecordType.
    ///
    /// Priority: `lifecycle_ref` (resolved via the package's standalone lifecycles) >
    /// inline `lifecycle`. Returns `None` in two cases:
    /// - The type has neither `lifecycle` nor `lifecycle_ref`.
    /// - `lifecycle_ref` is set but the UUID does not resolve in this package (dangling ref —
    ///   this should have been caught at package load time; treat as no lifecycle).
    pub fn effective_lifecycle<'a>(
        &'a self,
        record_type: &'a RecordType,
    ) -> Option<EffectiveLifecycle<'a>> {
        if let Some(ref_id) = &record_type.lifecycle_ref {
            self.resolve_lifecycle(ref_id).map(|lc| EffectiveLifecycle {
                initial_state: &lc.initial_state,
                states: &lc.states,
                transitions: &lc.transitions,
            })
        } else {
            record_type.lifecycle.as_ref().map(|lc| EffectiveLifecycle {
                initial_state: &lc.initial_state,
                states: &lc.states,
                transitions: &lc.transitions,
            })
        }
    }
}

/// RFC-039 [R3]: inline-composite recursion resolves range Types against the
/// loaded package. One resolver for validation and the migration verifier.
impl srs_core::validation::value_shape::RangeResolver for Package {
    fn effective_fields(
        &self,
        type_id: &str,
        type_version: u32,
    ) -> Option<Vec<srs_core::validation::value_shape::EffectiveField>> {
        let rt = self.resolve_type(type_id, type_version)?;
        self.resolved_effective_fields(rt).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RepositoryError;
    use crate::store::{FileStore, RepositoryStore};
    use srs_core::types::record_type::FieldAssignmentOverride;
    use std::path::Path;

    fn srs_spec_repo() -> PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let vendored = manifest.join("../../tests/fixtures/spec-repo");
        if let Ok(c) = vendored.canonicalize() {
            if c.join(".srs").exists() {
                return c;
            }
        }
        let mut dir = manifest.to_path_buf();
        loop {
            let candidate = dir.join("../srs/srs");
            if let Ok(c) = candidate.canonicalize() {
                if c.join(".srs").exists() {
                    return c;
                }
            }
            match dir.parent() {
                Some(p) if p != dir => dir = p.to_path_buf(),
                _ => break,
            }
        }
        manifest.join("../../../srs/srs")
    }

    #[test]
    fn load_package_preserves_extends_type_id() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        let types_dir = root.join("package/types");
        std::fs::create_dir_all(&types_dir).unwrap();
        std::fs::write(
            types_dir.join("base.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "00000000-0000-4000-8000-000000000030",
                "namespace": "com.test",
                "name": "base",
                "version": 1,
                "description": "Base type",
                "fields": [],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            types_dir.join("child.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "00000000-0000-4000-8000-000000000031",
                "namespace": "com.test",
                "name": "child",
                "version": 1,
                "description": "Child type",
                "fields": [],
                "extendsTypeId": "00000000-0000-4000-8000-000000000030",
                "extendsTypeVersion": 1,
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();

        write_package_json(
            &root.join("package"),
            "primary-pkg-id",
            "com.test",
            "primary",
            &[],
            &["types/base.json", "types/child.json"],
        );

        let package = FileStore::new(root)
            .load_package()
            .expect("should load package with inheritance");
        let child = package
            .record_types
            .iter()
            .find(|t| t.name == "child")
            .expect("child type must be loaded");
        assert_eq!(
            child.extends_type_id.as_deref(),
            Some("00000000-0000-4000-8000-000000000030"),
            "extends_type_id must survive load_package"
        );
    }

    #[test]
    fn load_package_from_live_repo() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        assert_eq!(package.namespace, "com.semanticops.srs");
        assert!(
            package.fields.len() > 20,
            "expected >20 fields, got {}",
            package.fields.len()
        );
        assert!(
            package.record_types.len() > 5,
            "expected >5 types, got {}",
            package.record_types.len()
        );
    }

    #[test]
    fn resolve_type_by_name_finds_known_type() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        // Use name-based lookup to avoid hardcoding UUIDs
        let ext_type = package
            .resolve_type_by_name("com.semanticops.srs", "meta.extension")
            .expect("should find meta.extension type");

        assert_eq!(ext_type.name, "meta.extension");
        assert_eq!(ext_type.namespace, "com.semanticops.srs");
        assert_eq!(ext_type.version, 1);
        assert!(!ext_type.fields.is_empty());
    }

    #[test]
    fn find_field_by_name_finds_status() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        let status_field = package
            .find_field_by_name("status")
            .expect("should find status field");

        assert_eq!(status_field.name, "status");
        assert_eq!(status_field.namespace, "com.semanticops.srs");
    }

    #[test]
    fn resolve_type_by_name_returns_none_for_unknown() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        assert!(package
            .resolve_type_by_name("unknown.namespace", "unknown-type")
            .is_none());
    }

    #[test]
    fn resolve_field_returns_none_for_unknown() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        assert!(package
            .resolve_field("00000000-0000-0000-0000-000000000000")
            .is_none());
    }

    #[test]
    fn load_package_loads_relation_types() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        assert!(
            package.relation_type_definitions.len() >= 7,
            "expected at least 7 relation types (canonical), got {}",
            package.relation_type_definitions.len()
        );
    }

    #[test]
    fn load_package_relation_types_are_deterministically_sorted() {
        // Regression: relation_type_definitions were collected from a HashMap,
        // so their order was randomized per process. That order leaks into the
        // regenerated package.json `relationTypes` index in `repo copy`, making
        // .srsj bundles non-deterministic. They must come out sorted by (key, id).
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        let keys: Vec<(&str, &str)> = package
            .relation_type_definitions
            .iter()
            .map(|rt| (rt.key.as_str(), rt.id.as_str()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(
            keys, sorted,
            "relation_type_definitions must be sorted by (key, id) for deterministic output"
        );
    }

    #[test]
    fn load_package_loads_document_views() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");
        assert!(
            !package.document_views.is_empty(),
            "expected at least one document view"
        );
    }

    #[test]
    fn resolve_document_view_finds_srs_spec_view() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");
        let view = package
            .resolve_document_view("ec34f54b-8636-5c8b-af5b-c9eb3df24fe6")
            .expect("should find srs spec document view");
        assert_eq!(view.name, "srs-spec-document-view");
    }

    #[test]
    fn resolve_document_view_returns_none_for_unknown() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");
        assert!(package
            .resolve_document_view("00000000-0000-0000-0000-000000000000")
            .is_none());
    }

    #[test]
    fn load_package_loads_themes() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        let themes_dir = root.join("package/themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(
            themes_dir.join("basic-theme.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": "00000000-0000-4000-8000-000000000950",
                "namespace": "fixture.theme",
                "name": "basic-theme",
                "version": 1,
                "description": "Basic theme",
                "targets": ["markdown"],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();

        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "primary-pkg-id",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": ["themes/basic-theme.json"]
            }))
            .unwrap(),
        )
        .unwrap();

        let package = FileStore::new(root)
            .load_package()
            .expect("should load themed package");
        assert_eq!(package.themes.len(), 1);
        assert_eq!(package.themes[0].name, "basic-theme");
    }

    #[test]
    fn resolve_theme_finds_known_theme() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        std::fs::create_dir_all(root.join("package/themes")).unwrap();
        std::fs::write(
            root.join("package/themes/basic-theme.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": "00000000-0000-4000-8000-000000000951",
                "namespace": "fixture.theme",
                "name": "basic-theme",
                "version": 1,
                "description": "Basic theme",
                "targets": ["markdown"],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "primary-pkg-id",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": ["themes/basic-theme.json"]
            }))
            .unwrap(),
        )
        .unwrap();

        let package = FileStore::new(root)
            .load_package()
            .expect("should load themed package");
        let theme = package
            .resolve_theme("00000000-0000-4000-8000-000000000951")
            .expect("should resolve theme by id");
        assert_eq!(theme.name, "basic-theme");
    }

    #[test]
    fn resolve_theme_returns_none_for_unknown() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        std::fs::create_dir_all(root.join("package/themes")).unwrap();
        std::fs::write(
            root.join("package/themes/basic-theme.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": "00000000-0000-4000-8000-000000000952",
                "namespace": "fixture.theme",
                "name": "basic-theme",
                "version": 1,
                "description": "Basic theme",
                "targets": ["markdown"],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "primary-pkg-id",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": ["themes/basic-theme.json"]
            }))
            .unwrap(),
        )
        .unwrap();

        let package = FileStore::new(root)
            .load_package()
            .expect("should load themed package");
        assert!(package
            .resolve_theme("00000000-0000-4000-8000-000000000000")
            .is_none());
    }

    #[test]
    fn load_package_without_themes_key_loads_without_error() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        let package = FileStore::new(root)
            .load_package()
            .expect("should load package without themes key");
        assert!(package.themes.is_empty());
    }

    #[test]
    fn load_package_theme_validation_fails_on_empty_targets() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        std::fs::create_dir_all(root.join("package/themes")).unwrap();
        std::fs::write(
            root.join("package/themes/invalid-theme.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": "00000000-0000-4000-8000-000000000953",
                "namespace": "fixture.theme",
                "name": "invalid-theme",
                "version": 1,
                "description": "Invalid theme",
                "targets": [],
                "createdAt": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "primary-pkg-id",
                "namespace": "com.test",
                "name": "primary",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": ["themes/invalid-theme.json"]
            }))
            .unwrap(),
        )
        .unwrap();

        let result = FileStore::new(root).load_package();
        assert!(
            matches!(result, Err(RepositoryError::ThemeValidation { .. })),
            "expected theme validation error, got {result:?}"
        );
    }

    #[test]
    fn resolve_canonical_relation_type_precedes() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        let rt = package
            .resolve_relation_type("precedes")
            .expect("should find canonical 'precedes' relation type");

        assert_eq!(rt.namespace, "com.semanticops.srs");
        assert!(rt.is_active());
        assert!(rt.is_irreflexive());
    }

    /// Write a minimal SRS repo at `root` with a primary package at `root/package/`.
    fn create_minimal_repo(root: &Path) {
        // .srs marker
        std::fs::create_dir_all(root.join(".srs")).unwrap();
        // manifest.json
        let manifest = serde_json::json!({
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo-id",
            "namespace": "com.test",
            "dataModelRevision": 2
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        // primary package
        let pkg_dir = root.join("package");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        write_package_json(&pkg_dir, "primary-pkg-id", "com.test", "primary", &[], &[]);
    }

    /// Write a package.json for the given dir, listing optional field/type files.
    fn write_package_json(
        dir: &Path,
        id: &str,
        namespace: &str,
        name: &str,
        fields: &[&str],
        types: &[&str],
    ) {
        let pkg = serde_json::json!({
            "id": id,
            "namespace": namespace,
            "name": name,
            "version": "1.0.0",
            "fields": fields,
            "types": types,
            "relationTypes": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            dir.join("package.json"),
            serde_json::to_string_pretty(&pkg).unwrap(),
        )
        .unwrap();
    }

    fn write_field_json(dir: &Path, file: &str, id: &str, name: &str) {
        let field = serde_json::json!({
            "id": id,
            "namespace": "com.test",
            "name": name,
            "version": 1,
            "valueType": "string"
        });
        std::fs::write(
            dir.join(file),
            serde_json::to_string_pretty(&field).unwrap(),
        )
        .unwrap();
    }

    fn add_package_ref_to_manifest(root: &Path, rel_path: &str) {
        let manifest_path = root.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let refs = manifest
            .get("packageRefs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut refs = refs;
        refs.push(serde_json::json!({"mode": "local", "path": rel_path}));
        manifest["packageRefs"] = serde_json::json!(refs);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn load_package_errors_on_missing_package_ref() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);
        add_package_ref_to_manifest(root, "package/nonexistent");

        let result = FileStore::new(root).load_package();
        assert!(
            matches!(result, Err(RepositoryError::PackageRefMissing { .. })),
            "expected PackageRefMissing, got {result:?}"
        );
    }

    #[test]
    fn load_package_detects_conflicting_field_definitions() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        // Sub-package with a field using the same id as primary but different name.
        let sub_dir = root.join("package").join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        write_field_json(
            &root.join("package"),
            "field-a.json",
            "field-uuid-1",
            "original_name",
        );
        write_package_json(
            &root.join("package"),
            "primary-pkg-id",
            "com.test",
            "primary",
            &["field-a.json"],
            &[],
        );

        write_field_json(
            &sub_dir,
            "field-a-conflict.json",
            "field-uuid-1",
            "different_name",
        );
        write_package_json(
            &sub_dir,
            "sub-pkg-id",
            "com.test",
            "sub",
            &["field-a-conflict.json"],
            &[],
        );
        add_package_ref_to_manifest(root, "package/sub");

        let result = FileStore::new(root).load_package();
        assert!(
            matches!(
                result,
                Err(RepositoryError::PackageRefConflict { ref kind, .. }) if kind == "field"
            ),
            "expected PackageRefConflict(field), got {result:?}"
        );
    }

    #[test]
    fn load_package_coalesces_identical_field_definitions() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        create_minimal_repo(root);

        let sub_dir = root.join("package").join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();

        // Same field in both primary and sub-package.
        write_field_json(
            &root.join("package"),
            "field-a.json",
            "field-uuid-1",
            "shared_field",
        );
        write_package_json(
            &root.join("package"),
            "primary-pkg-id",
            "com.test",
            "primary",
            &["field-a.json"],
            &[],
        );
        write_field_json(&sub_dir, "field-a.json", "field-uuid-1", "shared_field");
        write_package_json(
            &sub_dir,
            "sub-pkg-id",
            "com.test",
            "sub",
            &["field-a.json"],
            &[],
        );
        add_package_ref_to_manifest(root, "package/sub");

        let package = FileStore::new(root)
            .load_package()
            .expect("identical fields should coalesce without error");
        // Field should appear exactly once.
        let count = package
            .fields
            .iter()
            .filter(|f| f.id == "field-uuid-1")
            .count();
        assert_eq!(count, 1, "expected exactly one copy of field-uuid-1");
    }

    #[test]
    fn deprecated_relation_types_loaded_with_correct_status() {
        let srs_repo = srs_spec_repo();
        let package = FileStore::new(&srs_repo)
            .load_package()
            .expect("should load live srs package");

        let deprecated: Vec<_> = package
            .relation_type_definitions
            .iter()
            .filter(|rt| !rt.is_active())
            .collect();

        assert!(
            !deprecated.is_empty(),
            "expected at least one deprecated relation type"
        );
        for rt in deprecated {
            assert!(
                rt.resolves(),
                "deprecated/tombstone types should still resolve"
            );
        }
    }

    // ── effective_fields tests ────────────────────────────────────────────────

    fn make_package_with_types(types: Vec<RecordType>) -> Package {
        Package {
            id: "pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: types,
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/test"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    fn fa(field_id: &str, order: u32, required: bool) -> FieldAssignment {
        FieldAssignment {
            field_id: field_id.to_string(),
            order,
            required,
            display_label: None,
            description: None,
        }
    }

    fn make_type(id: &str, fields: Vec<FieldAssignment>) -> RecordType {
        RecordType {
            extra: Default::default(),
            schema: None,
            ai_guidance: None,
            tags: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: id.to_string(),
            version: 1,
            description: "test".to_string(),
            fields,
            extends_type_id: None,
            extends_type_version: None,
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

    fn make_child_type(
        id: &str,
        fields: Vec<FieldAssignment>,
        parent_id: &str,
        field_order: Option<Vec<String>>,
        overrides: Option<Vec<FieldAssignmentOverride>>,
    ) -> RecordType {
        RecordType {
            extra: Default::default(),
            schema: None,
            ai_guidance: None,
            tags: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: id.to_string(),
            version: 1,
            description: "test".to_string(),
            fields,
            extends_type_id: Some(parent_id.to_string()),
            extends_type_version: Some(1),
            field_order,
            field_assignment_overrides: overrides,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        }
    }

    #[test]
    fn effective_fields_non_inheriting_returns_sorted_own_fields() {
        let rt = make_type("base", vec![fa("f2", 1, false), fa("f1", 0, true)]);
        let pkg = make_package_with_types(vec![rt.clone()]);
        let result = pkg.effective_fields(&rt).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].field_id, "f1");
        assert_eq!(result[1].field_id, "f2");
    }

    #[test]
    fn effective_fields_single_level_inheritance() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        let child = make_child_type("child", vec![fa("f2", 0, false)], "base", None, None);
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].field_id, "f1", "base field first");
        assert_eq!(result[1].field_id, "f2", "own field second");
    }

    #[test]
    fn effective_fields_two_level_chain() {
        let grandparent = make_type("gp", vec![fa("f1", 0, true)]);
        let mut parent = make_child_type("parent", vec![fa("f2", 0, false)], "gp", None, None);
        parent.extends_type_id = Some("gp".to_string());
        let mut child = make_child_type("child", vec![fa("f3", 0, false)], "parent", None, None);
        child.extends_type_id = Some("parent".to_string());
        let pkg = make_package_with_types(vec![grandparent, parent, child.clone()]);
        let result = pkg.effective_fields(&child).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].field_id, "f1", "grandparent field first");
        assert_eq!(result[1].field_id, "f2", "parent field second");
        assert_eq!(result[2].field_id, "f3", "own field third");
    }

    #[test]
    fn effective_fields_detects_cycle() {
        let mut a = make_child_type("a", vec![], "b", None, None);
        let mut b = make_child_type("b", vec![], "a", None, None);
        a.extends_type_id = Some("b".to_string());
        b.extends_type_id = Some("a".to_string());
        let pkg = make_package_with_types(vec![a.clone(), b]);
        let result = pkg.effective_fields(&a);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::TypeInheritanceCycle { .. })
            ),
            "expected TypeInheritanceCycle, got {:?}",
            result
        );
    }

    #[test]
    fn effective_identity_field_id_own_value_wins_without_walking_chain() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        let mut child = make_child_type("child", vec![fa("f2", 0, false)], "base", None, None);
        child.identity_field_id = Some("f2".to_string());
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_identity_field_id(&child).unwrap();
        assert_eq!(result, Some("f2".to_string()));
    }

    #[test]
    fn effective_identity_field_id_inherits_from_base() {
        let mut base = make_type("base", vec![fa("f1", 0, true)]);
        base.identity_field_id = Some("f1".to_string());
        let child = make_child_type("child", vec![fa("f2", 0, false)], "base", None, None);
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_identity_field_id(&child).unwrap();
        assert_eq!(result, Some("f1".to_string()));
    }

    #[test]
    fn effective_identity_field_id_inherits_transitively_through_two_levels() {
        let mut grandparent = make_type("gp", vec![fa("f1", 0, true)]);
        grandparent.identity_field_id = Some("f1".to_string());
        let mut parent = make_child_type("parent", vec![fa("f2", 0, false)], "gp", None, None);
        parent.extends_type_id = Some("gp".to_string());
        let mut child = make_child_type("child", vec![fa("f3", 0, false)], "parent", None, None);
        child.extends_type_id = Some("parent".to_string());
        let pkg = make_package_with_types(vec![grandparent, parent, child.clone()]);
        let result = pkg.effective_identity_field_id(&child).unwrap();
        assert_eq!(
            result,
            Some("f1".to_string()),
            "grandparent's identityFieldId resolves transitively"
        );
    }

    #[test]
    fn effective_identity_field_id_override_wins_over_base() {
        let mut base = make_type("base", vec![fa("f1", 0, true)]);
        base.identity_field_id = Some("f1".to_string());
        let mut child = make_child_type("child", vec![fa("f2", 0, false)], "base", None, None);
        child.identity_field_id = Some("f2".to_string());
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_identity_field_id(&child).unwrap();
        assert_eq!(
            result,
            Some("f2".to_string()),
            "child's own identityFieldId overrides the inherited one"
        );
    }

    #[test]
    fn effective_identity_field_id_none_when_no_type_in_chain_declares_one() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        let child = make_child_type("child", vec![fa("f2", 0, false)], "base", None, None);
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_identity_field_id(&child).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn effective_identity_field_id_detects_cycle() {
        let mut a = make_child_type("a", vec![], "b", None, None);
        let mut b = make_child_type("b", vec![], "a", None, None);
        a.extends_type_id = Some("b".to_string());
        b.extends_type_id = Some("a".to_string());
        let pkg = make_package_with_types(vec![a.clone(), b]);
        let result = pkg.effective_identity_field_id(&a);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::TypeInheritanceCycle { .. })
            ),
            "expected TypeInheritanceCycle, got {:?}",
            result
        );
    }

    #[test]
    fn effective_fields_field_order_reorders() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        let child = make_child_type(
            "child",
            vec![fa("f2", 0, false)],
            "base",
            Some(vec!["f2".to_string(), "f1".to_string()]),
            None,
        );
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child).unwrap();
        assert_eq!(result[0].field_id, "f2", "fieldOrder: f2 first");
        assert_eq!(result[1].field_id, "f1", "fieldOrder: f1 second");
    }

    #[test]
    fn effective_fields_field_order_incomplete_errors() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        // fieldOrder only lists f2, missing f1
        let child = make_child_type(
            "child",
            vec![fa("f2", 0, false)],
            "base",
            Some(vec!["f2".to_string()]),
            None,
        );
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::FieldOrderMismatch { .. })
            ),
            "expected FieldOrderMismatch, got {:?}",
            result
        );
    }

    #[test]
    fn effective_fields_field_order_duplicate_entry_errors() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        // fieldOrder contains f2 twice — Inv 41 violation
        let child = make_child_type(
            "child",
            vec![fa("f2", 0, false)],
            "base",
            Some(vec!["f1".to_string(), "f2".to_string(), "f2".to_string()]),
            None,
        );
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::FieldOrderMismatch { .. })
            ),
            "expected FieldOrderMismatch for duplicate fieldOrder entry, got {:?}",
            result
        );
    }

    #[test]
    fn effective_fields_field_order_unknown_id_errors() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        // fieldOrder contains "bogus" which is not in the effective set — Inv 41 violation
        let child = make_child_type(
            "child",
            vec![fa("f2", 0, false)],
            "base",
            Some(vec![
                "f1".to_string(),
                "f2".to_string(),
                "bogus".to_string(),
            ]),
            None,
        );
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::FieldOrderMismatch { .. })
            ),
            "expected FieldOrderMismatch for unknown fieldOrder entry, got {:?}",
            result
        );
    }

    #[test]
    fn effective_fields_override_targets_unknown_field_errors() {
        let base = make_type("base", vec![fa("f1", 0, false)]);
        // override targets "bogus" which is neither inherited nor owned
        let child = make_child_type(
            "child",
            vec![fa("f2", 0, false)],
            "base",
            None,
            Some(vec![FieldAssignmentOverride {
                field_id: "bogus".to_string(),
                display_label: None,
                display_hint: None,
                required: Some(true),
            }]),
        );
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::OverrideTargetsOwnField { .. })
            ),
            "expected OverrideTargetsOwnField for unknown override target, got {:?}",
            result
        );
    }

    #[test]
    fn effective_fields_detects_duplicate_field() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        // own fields contains f1 which is also in base — Inv 40 violation
        let child = make_child_type("child", vec![fa("f1", 0, false)], "base", None, None);
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::InheritedFieldDuplicate { .. })
            ),
            "expected InheritedFieldDuplicate, got {:?}",
            result
        );
    }

    #[test]
    fn effective_fields_override_relaxes_required_errors() {
        let base = make_type("base", vec![fa("f1", 0, true)]);
        let child = make_child_type(
            "child",
            vec![fa("f2", 0, false)],
            "base",
            None,
            Some(vec![FieldAssignmentOverride {
                field_id: "f1".to_string(),
                display_label: None,
                display_hint: None,
                required: Some(false),
            }]),
        );
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::OverrideRelaxesRequired { .. })
            ),
            "expected OverrideRelaxesRequired, got {:?}",
            result
        );
    }

    #[test]
    fn effective_fields_override_tightens_required_ok() {
        let base = make_type("base", vec![fa("f1", 0, false)]);
        let child = make_child_type(
            "child",
            vec![fa("f2", 0, false)],
            "base",
            None,
            Some(vec![FieldAssignmentOverride {
                field_id: "f1".to_string(),
                display_label: None,
                display_hint: None,
                required: Some(true),
            }]),
        );
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child).unwrap();
        let f1 = result.iter().find(|fa| fa.field_id == "f1").unwrap();
        assert!(f1.required, "override tightened required: false → true");
    }

    #[test]
    fn effective_fields_override_targets_own_field_errors() {
        let base = make_type("base", vec![fa("f1", 0, false)]);
        let child = make_child_type(
            "child",
            vec![fa("f2", 0, false)],
            "base",
            None,
            Some(vec![FieldAssignmentOverride {
                field_id: "f2".to_string(),
                display_label: None,
                display_hint: None,
                required: Some(true),
            }]),
        );
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let result = pkg.effective_fields(&child);
        assert!(
            matches!(
                result,
                Err(crate::error::RepositoryError::OverrideTargetsOwnField { .. })
            ),
            "expected OverrideTargetsOwnField, got {:?}",
            result
        );
    }

    #[test]
    fn validate_record_uses_effective_fields() {
        use srs_core::types::field::Field;
        use srs_core::types::field_type::FieldType;
        use srs_core::types::record::{FieldValues, Record};
        use srs_core::validation::record::validate_record;
        use srs_core::validation::value_shape::EffectiveField;

        let base = make_type("base", vec![fa("f1", 0, true)]);
        let child = make_child_type("child", vec![fa("f2", 0, false)], "base", None, None);
        let mut pkg = make_package_with_types(vec![base, child.clone()]);
        pkg.fields = vec![
            Field::new("f1", "com.test", "field_one", FieldType::string()),
            Field::new("f2", "com.test", "field_two", FieldType::string()),
        ];
        let effective = pkg.resolved_effective_fields(&child).unwrap();
        let resolver = |_: &str, _: u32| -> Option<Vec<EffectiveField>> { None };

        let record = Record {
            field_meta: None,
            instance_id: "r1".to_string(),
            type_id: "child".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "child".to_string(),
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert("field_one", serde_json::json!("hello"));
                fv
            },
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        };

        // f1 is inherited (required) and present → should pass
        assert!(
            validate_record(&record, &child, &effective, &resolver).is_ok(),
            "record with inherited required field present should pass"
        );

        // without f1 → should fail (inherited required field missing)
        let record_no_f1 = Record {
            field_values: FieldValues::new(),
            ..record
        };
        assert!(
            validate_record(&record_no_f1, &child, &effective, &resolver).is_err(),
            "record missing inherited required field should fail"
        );
    }

    // ── effective_lifecycle tests ──────────────────────────────────────────────

    fn make_lc_states() -> Vec<srs_core::types::lifecycle::LifecycleState> {
        vec![
            srs_core::types::lifecycle::LifecycleState {
                id: None,
                version: None,
                namespace: None,
                key: "draft".to_string(),
                label: None,
                description: None,
                aliases: None,
                is_initial: Some(true),
                is_final: None,
                status: None,
                requires_relation: None,
                properties: None,
            },
            srs_core::types::lifecycle::LifecycleState {
                id: None,
                version: None,
                namespace: None,
                key: "active".to_string(),
                label: None,
                description: None,
                aliases: None,
                is_initial: None,
                is_final: Some(true),
                status: None,
                requires_relation: None,
                properties: None,
            },
        ]
    }

    fn make_lc_transitions() -> Vec<srs_core::types::lifecycle::LifecycleTransition> {
        vec![srs_core::types::lifecycle::LifecycleTransition {
            id: None,
            name: "publish".to_string(),
            from: "draft".to_string(),
            to: "active".to_string(),
            description: None,
            properties: None,
        }]
    }

    fn make_minimal_record_type(
        lifecycle: Option<srs_core::types::record_type::TypeLifecycle>,
        lifecycle_ref: Option<String>,
    ) -> srs_core::types::record_type::RecordType {
        srs_core::types::record_type::RecordType {
            extra: Default::default(),
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "rt-test".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "test".to_string(),
            fields: vec![],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle,
            lifecycle_ref,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        }
    }

    fn make_minimal_package(lifecycles: Vec<srs_core::types::lifecycle::Lifecycle>) -> Package {
        Package {
            id: "pkg-test".to_string(),
            namespace: "com.test".to_string(),
            name: "test-pkg".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles,
        }
    }

    #[test]
    fn effective_lifecycle_inline_resolves() {
        let inline_lc = srs_core::types::record_type::TypeLifecycle {
            states: make_lc_states(),
            transitions: make_lc_transitions(),
            initial_state: "draft".to_string(),
        };
        let rt = make_minimal_record_type(Some(inline_lc), None);
        let pkg = make_minimal_package(vec![]);
        let eff = pkg.effective_lifecycle(&rt).expect("should resolve");
        assert_eq!(eff.initial_state, "draft");
        assert_eq!(eff.states.len(), 2);
        assert_eq!(eff.transitions.len(), 1);
    }

    #[test]
    fn effective_lifecycle_ref_resolves() {
        let standalone = srs_core::types::lifecycle::Lifecycle {
            schema: None,
            tags: None,
            id: "lc-ref-standalone-001".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "test-lc".to_string(),
            states: make_lc_states(),
            transitions: make_lc_transitions(),
            initial_state: "draft".to_string(),
            extends_lifecycle_id: None,
            extends_lifecycle_version: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let rt = make_minimal_record_type(None, Some("lc-ref-standalone-001".to_string()));
        let pkg = make_minimal_package(vec![standalone]);
        let eff = pkg.effective_lifecycle(&rt).expect("should resolve");
        assert_eq!(eff.initial_state, "draft");
        assert_eq!(eff.states.len(), 2);
        assert_eq!(eff.transitions.len(), 1);
    }

    #[test]
    fn effective_lifecycle_none_when_absent() {
        let rt = make_minimal_record_type(None, None);
        let pkg = make_minimal_package(vec![]);
        assert!(pkg.effective_lifecycle(&rt).is_none());
    }

    #[test]
    fn effective_lifecycle_ref_wins_over_inline() {
        let inline_lc = srs_core::types::record_type::TypeLifecycle {
            states: make_lc_states(),
            transitions: make_lc_transitions(),
            initial_state: "inline-initial".to_string(),
        };
        let standalone = srs_core::types::lifecycle::Lifecycle {
            schema: None,
            tags: None,
            id: "lc-ref-standalone-001".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "test-lc".to_string(),
            states: make_lc_states(),
            transitions: make_lc_transitions(),
            initial_state: "ref-initial".to_string(),
            extends_lifecycle_id: None,
            extends_lifecycle_version: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let rt =
            make_minimal_record_type(Some(inline_lc), Some("lc-ref-standalone-001".to_string()));
        let pkg = make_minimal_package(vec![standalone]);
        let eff = pkg.effective_lifecycle(&rt).expect("should resolve");
        assert_eq!(
            eff.initial_state, "ref-initial",
            "lifecycle_ref must take priority over inline"
        );
    }
}
