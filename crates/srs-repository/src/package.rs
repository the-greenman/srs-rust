use srs_core::types::blueprint::Blueprint;
use srs_core::types::field::Field;
use srs_core::types::lifecycle::{Lifecycle, LifecycleState, LifecycleTransition};
use srs_core::types::protocol::Protocol;
use srs_core::types::record_type::{FieldAssignment, FieldGroup, RecordType};
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
    pub blueprints: Vec<Blueprint>,
    pub protocols: Vec<LoadedProtocol>,
    pub root: PathBuf,
    /// ext:type-inheritance — external package dependencies declared in dependencyRefs.
    pub dependency_refs: Vec<DependencyRef>,
    pub vocabularies: Vec<Vocabulary>,
    pub lifecycles: Vec<Lifecycle>,
}

/// A protocol as loaded from a package, bundling typed struct + verbatim JSON.
///
/// `raw` preserves all fields from the on-disk JSON, including any value-centric
/// stage fields (e.g. `output_type`) that are not fully captured by the typed
/// `Protocol` struct. `source_package` is `None` for the root package and `Some`
/// for protocols merged from a dependency package.
#[derive(Debug, Clone)]
pub struct LoadedProtocol {
    pub protocol: Protocol,
    pub raw: serde_json::Value,
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

/// Returned by [`Package::effective_fields_and_groups`].
#[derive(Debug)]
pub struct EffectiveFieldsAndGroups {
    /// Fields in their final sorted/fieldOrder-reordered order (same as `effective_fields`).
    pub fields: Vec<FieldAssignment>,
    /// 1-based position of each field in the merged field+group sequence.
    /// Parallel to `fields`: `field_positions[i]` is the merged position of `fields[i]`.
    pub field_positions: Vec<usize>,
    /// Groups with their 1-based position in the merged field+group sequence.
    pub groups: Vec<OrderedGroup>,
}

/// A field group with its computed 1-based position in the merged (fields + groups) sequence.
#[derive(Debug)]
pub struct OrderedGroup {
    /// The full FieldGroup struct (including group_id, order, fields, label, etc.).
    pub group: FieldGroup,
    /// 1-based position of this group in the merged sequence.
    pub merged_position: usize,
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

    /// Get all fields as a slice.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Get all record types as a slice.
    pub fn record_types(&self) -> &[RecordType] {
        &self.record_types
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

        let extends_type_id = match &record_type.extends_type_id {
            None => {
                let mut fields = record_type.fields.clone();
                fields.sort_by_key(|fa| fa.order);
                return Ok(fields);
            }
            Some(id) => id.clone(),
        };
        let extends_version = record_type.extends_type_version.unwrap_or(1);

        // Walk the inheritance chain iteratively, collecting type IDs to detect cycles.
        let mut chain: Vec<Vec<FieldAssignment>> = vec![record_type.fields.clone()];
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(record_type.id.clone());

        let mut current_id = extends_type_id;
        let mut current_version = extends_version;

        loop {
            if visited.contains(&current_id) {
                return Err(RepositoryError::TypeInheritanceCycle {
                    type_id: current_id,
                });
            }
            let base = self
                .resolve_type(&current_id, current_version)
                .ok_or_else(|| RepositoryError::TypeNotFound {
                    type_id: current_id.clone(),
                    version: current_version,
                })?;
            visited.insert(current_id.clone());
            chain.push(base.fields.clone());
            match &base.extends_type_id {
                None => break,
                Some(next_id) => {
                    current_id = next_id.clone();
                    current_version = base.extends_type_version.unwrap_or(1);
                }
            }
        }

        // Build the merged list: base fields first, then own fields (chain is reversed).
        chain.reverse();
        let mut merged: Vec<FieldAssignment> = Vec::new();
        let own_field_ids: HashSet<String> = record_type
            .fields
            .iter()
            .map(|fa| fa.field_id.clone())
            .collect();

        let mut seen_ids: HashSet<String> = HashSet::new();
        for level_fields in &chain[..chain.len() - 1] {
            for fa in level_fields {
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
            // Group IDs are allowed through here — they are not field IDs but are valid
            // fieldOrder entries handled by effective_fields_and_groups.
            let group_ids_in_type: HashSet<&str> = record_type
                .field_groups
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|g| g.group_id.as_str())
                .collect();
            for fid in field_order {
                if !effective_ids.contains(fid.as_str())
                    && !group_ids_in_type.contains(fid.as_str())
                {
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

    /// Resolve the effective field list AND the merged position of each field group.
    ///
    /// Extends [`effective_fields`] to support group IDs in `fieldOrder`. When `fieldOrder`
    /// is declared on a type that has `fieldGroups`, this function validates that all group
    /// IDs are listed in `fieldOrder` (in addition to the field-completeness check already
    /// performed by `effective_fields`). When `fieldOrder` is absent, groups are merged into
    /// the position sequence using a stable merge-sort by `order` (fields before groups on
    /// tie; groups by `group_id` on group-group tie).
    ///
    /// Callers that do not need group ordering should continue to use [`effective_fields`].
    pub fn effective_fields_and_groups(
        &self,
        record_type: &RecordType,
    ) -> Result<EffectiveFieldsAndGroups, crate::error::RepositoryError> {
        use crate::error::RepositoryError;

        let fields = self.effective_fields(record_type)?;

        let groups = match &record_type.field_groups {
            None => vec![],
            Some(g) if g.is_empty() => vec![],
            Some(groups) => groups.clone(),
        };

        if groups.is_empty() {
            // No groups: field positions are their 1-based index in effective_fields.
            let field_positions = (1..=fields.len()).collect();
            return Ok(EffectiveFieldsAndGroups {
                fields,
                field_positions,
                groups: vec![],
            });
        }

        if let Some(field_order) = &record_type.field_order {
            // `effective_fields` already validated that all field IDs are in `field_order`
            // and that no unknown field IDs appear. Here we add the group layer.
            let field_ids: std::collections::HashSet<&str> =
                fields.iter().map(|fa| fa.field_id.as_str()).collect();
            let group_ids: std::collections::HashSet<&str> =
                groups.iter().map(|g| g.group_id.as_str()).collect();

            // Validate: no unknown IDs (IDs that are neither a field ID nor a group ID).
            for id in field_order {
                if !field_ids.contains(id.as_str()) && !group_ids.contains(id.as_str()) {
                    return Err(RepositoryError::FieldOrderMismatch {
                        type_id: record_type.id.clone(),
                        field_id: id.clone(),
                    });
                }
            }

            // Validate: all group IDs must appear in field_order.
            let listed_ids: std::collections::HashSet<&str> =
                field_order.iter().map(|s| s.as_str()).collect();
            for group in &groups {
                if !listed_ids.contains(group.group_id.as_str()) {
                    return Err(RepositoryError::FieldOrderMismatch {
                        type_id: record_type.id.clone(),
                        field_id: group.group_id.clone(),
                    });
                }
            }

            // Walk field_order sequentially (1-based), recording positions for fields and groups.
            let mut ordered_groups: Vec<OrderedGroup> = Vec::new();
            // field_positions maps field_id → merged position; filled as we walk.
            let mut field_pos_map: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for (pos, id) in field_order.iter().enumerate() {
                let merged_position = pos + 1;
                if let Some(group) = groups.iter().find(|g| g.group_id == *id) {
                    ordered_groups.push(OrderedGroup {
                        group: group.clone(),
                        merged_position,
                    });
                } else {
                    field_pos_map.insert(id.as_str(), merged_position);
                }
            }
            // Build field_positions in the same order as `fields`.
            // unwrap_or(&0) is safe: every field_id in `fields` was in field_order
            // (validated above via effective_fields), so it was inserted into field_pos_map.
            let field_positions: Vec<usize> = fields
                .iter()
                .map(|fa| *field_pos_map.get(fa.field_id.as_str()).unwrap_or(&0))
                .collect();

            Ok(EffectiveFieldsAndGroups {
                fields,
                field_positions,
                groups: ordered_groups,
            })
        } else {
            // No fieldOrder: stable merge-sort of fields (by assignment.order) and groups
            // (by group.order) into a single position sequence.
            // Tie-breaking: fields before groups at equal order; groups by group_id
            // lexicographically at equal group order.
            let mut groups_sorted = groups.clone();
            groups_sorted.sort_by(|a, b| a.order.cmp(&b.order).then(a.group_id.cmp(&b.group_id)));

            // Two-pointer merge: fields are already sorted by effective_fields.
            let mut field_idx = 0usize;
            let mut group_idx = 0usize;
            let mut position = 0usize;
            let mut ordered_groups: Vec<OrderedGroup> = Vec::new();
            let mut field_positions: Vec<usize> = vec![0; fields.len()];

            while field_idx < fields.len() || group_idx < groups_sorted.len() {
                position += 1;
                let take_field = if field_idx >= fields.len() {
                    false
                } else if group_idx >= groups_sorted.len() {
                    true
                } else {
                    // Fields before groups at equal order.
                    fields[field_idx].order <= groups_sorted[group_idx].order
                };

                if take_field {
                    field_positions[field_idx] = position;
                    field_idx += 1;
                } else {
                    ordered_groups.push(OrderedGroup {
                        group: groups_sorted[group_idx].clone(),
                        merged_position: position,
                    });
                    group_idx += 1;
                }
            }

            Ok(EffectiveFieldsAndGroups {
                fields,
                field_positions,
                groups: ordered_groups,
            })
        }
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
            "extends_type_id must survive load_package; extra = {:?}",
            child.extra
        );
        assert!(
            child.extra.is_empty(),
            "extends_type_id must not fall into extra after load_package"
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
            "instanceIndex": []
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
            repeatable: false,
            min_items: None,
            max_items: None,
        }
    }

    fn make_type(id: &str, fields: Vec<FieldAssignment>) -> RecordType {
        RecordType {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: id.to_string(),
            version: 1,
            description: "test".to_string(),
            fields,
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
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
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: id.to_string(),
            version: 1,
            description: "test".to_string(),
            fields,
            field_groups: None,
            extends_type_id: Some(parent_id.to_string()),
            extends_type_version: Some(1),
            field_order,
            field_assignment_overrides: overrides,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
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
    fn effective_fields_field_order_group_id_is_allowed() {
        // A group ID in fieldOrder is valid — effective_fields must pass it through silently
        // so that effective_fields_and_groups can handle it. A group ID is not a field ID
        // and must not be treated as "unknown".
        let rt = srs_core::types::record_type::RecordType {
            id: "t".to_string(),
            namespace: "com.test".to_string(),
            name: "t".to_string(),
            version: 1,
            description: String::new(),
            fields: vec![fa("f1", 0, true), fa("f2", 1, false)],
            field_groups: Some(vec![srs_core::types::record_type::FieldGroup {
                group_id: "my-group".to_string(),
                order: 0,
                fields: vec![],
                label: None,
                description: None,
                required: false,
                repeatable: false,
                min_items: None,
                max_items: None,
                composite_renderer: None,
            }]),
            extends_type_id: None,
            extends_type_version: None,
            field_order: Some(vec![
                "f1".to_string(),
                "my-group".to_string(),
                "f2".to_string(),
            ]),
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        };
        let pkg = make_package_with_types(vec![rt.clone()]);
        let fields = pkg
            .effective_fields(&rt)
            .expect("group ID in fieldOrder must not cause an error");
        // Only field IDs returned; group ID is skipped during reordering.
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].field_id, "f1");
        assert_eq!(fields[1].field_id, "f2");
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
        use srs_core::types::record::{FieldValue, Record};
        use srs_core::validation::record::validate_record;

        let base = make_type("base", vec![fa("f1", 0, true)]);
        let child = make_child_type("child", vec![fa("f2", 0, false)], "base", None, None);
        let pkg = make_package_with_types(vec![base, child.clone()]);
        let effective = pkg.effective_fields(&child).unwrap();

        let record = Record {
            instance_id: "r1".to_string(),
            type_id: "child".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "child".to_string(),
            field_values: vec![FieldValue {
                field_id: "f1".to_string(),
                value: serde_json::json!("hello"),
                entries: None,
                source: None,
                edited_at: None,
            }],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::HashMap::new(),
        };

        // f1 is inherited (required) and present → should pass
        assert!(
            validate_record(&record, &child, &effective).is_ok(),
            "record with inherited required field present should pass"
        );

        // without f1 → should fail (inherited required field missing)
        let record_no_f1 = Record {
            field_values: vec![],
            ..record
        };
        assert!(
            validate_record(&record_no_f1, &child, &effective).is_err(),
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
            id: "rt-test".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "test".to_string(),
            fields: vec![],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle,
            lifecycle_ref,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
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
            extra: std::collections::HashMap::new(),
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
            extra: std::collections::HashMap::new(),
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

    // ── effective_fields_and_groups tests ─────────────────────────────────────

    fn make_group(group_id: &str, order: u32) -> srs_core::types::record_type::FieldGroup {
        srs_core::types::record_type::FieldGroup {
            group_id: group_id.to_string(),
            order,
            fields: vec![],
            label: None,
            description: None,
            required: false,
            repeatable: false,
            min_items: None,
            max_items: None,
            composite_renderer: None,
        }
    }

    fn make_type_with_groups(
        id: &str,
        fields: Vec<FieldAssignment>,
        groups: Option<Vec<srs_core::types::record_type::FieldGroup>>,
        field_order: Option<Vec<String>>,
    ) -> RecordType {
        RecordType {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: id.to_string(),
            version: 1,
            description: "test".to_string(),
            fields,
            field_groups: groups,
            extends_type_id: None,
            extends_type_version: None,
            field_order,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn effective_fields_and_groups_no_groups_returns_empty() {
        let rt = make_type("base", vec![fa("f1", 0, false), fa("f2", 1, false)]);
        let pkg = make_package_with_types(vec![rt.clone()]);
        let result = pkg.effective_fields_and_groups(&rt).unwrap();
        assert_eq!(result.fields.len(), 2);
        assert!(result.groups.is_empty());
    }

    #[test]
    fn effective_fields_and_groups_no_field_order_interleaves_by_order() {
        // field(order=0), group(order=1), field(order=2) → merged positions 1, 2, 3.
        // group gets merged_position=2 (field at 0 takes slot 1, group at 1 takes slot 2,
        // field at 2 takes slot 3).
        let rt = make_type_with_groups(
            "t",
            vec![fa("f1", 0, false), fa("f2", 2, false)],
            Some(vec![make_group("g1", 1)]),
            None,
        );
        let pkg = make_package_with_types(vec![rt.clone()]);
        let result = pkg.effective_fields_and_groups(&rt).unwrap();
        assert_eq!(result.fields.len(), 2);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].group.group_id, "g1");
        assert_eq!(
            result.groups[0].merged_position, 2,
            "group at order=1 merges between field(order=0) and field(order=2) → position 2"
        );
    }

    #[test]
    fn effective_fields_and_groups_field_order_assigns_group_positions() {
        // fieldOrder: [field_a, group_id, field_b] → group gets merged_position: 2.
        let rt = make_type_with_groups(
            "t",
            vec![fa("fa", 0, false), fa("fb", 1, false)],
            Some(vec![make_group("g1", 99)]),
            Some(vec!["fa".to_string(), "g1".to_string(), "fb".to_string()]),
        );
        let pkg = make_package_with_types(vec![rt.clone()]);
        let result = pkg.effective_fields_and_groups(&rt).unwrap();
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].group.group_id, "g1");
        assert_eq!(
            result.groups[0].merged_position, 2,
            "g1 is at position 2 in fieldOrder [fa, g1, fb]"
        );
    }

    #[test]
    fn effective_fields_and_groups_field_order_missing_group_errors() {
        // fieldOrder lists only field IDs when a group is present → FieldOrderMismatch.
        let rt = make_type_with_groups(
            "t",
            vec![fa("fa", 0, false)],
            Some(vec![make_group("g1", 0)]),
            Some(vec!["fa".to_string()]),
        );
        let pkg = make_package_with_types(vec![rt.clone()]);
        let err = pkg.effective_fields_and_groups(&rt).unwrap_err();
        assert!(
            matches!(err, crate::error::RepositoryError::FieldOrderMismatch { ref field_id, .. } if field_id == "g1"),
            "expected FieldOrderMismatch with field_id=g1, got: {err:?}"
        );
    }

    #[test]
    fn effective_fields_and_groups_field_order_unknown_id_errors() {
        // fieldOrder contains a string that is neither a field ID nor a group ID.
        let rt = make_type_with_groups(
            "t",
            vec![fa("fa", 0, false)],
            Some(vec![make_group("g1", 0)]),
            Some(vec![
                "fa".to_string(),
                "g1".to_string(),
                "unknown-id".to_string(),
            ]),
        );
        let pkg = make_package_with_types(vec![rt.clone()]);
        let err = pkg.effective_fields_and_groups(&rt).unwrap_err();
        assert!(
            matches!(err, crate::error::RepositoryError::FieldOrderMismatch { ref field_id, .. } if field_id == "unknown-id"),
            "expected FieldOrderMismatch with field_id=unknown-id, got: {err:?}"
        );
    }

    #[test]
    fn effective_fields_and_groups_group_before_all_fields() {
        // group(order=0) with two fields(order=1, order=2) and no fieldOrder.
        // Fields before groups on tie, but group.order=0 < field.order=1 → group first.
        let rt = make_type_with_groups(
            "t",
            vec![fa("f1", 1, false), fa("f2", 2, false)],
            Some(vec![make_group("g1", 0)]),
            None,
        );
        let pkg = make_package_with_types(vec![rt.clone()]);
        let result = pkg.effective_fields_and_groups(&rt).unwrap();
        assert_eq!(result.groups.len(), 1);
        assert_eq!(
            result.groups[0].merged_position, 1,
            "group at order=0 is before fields at order=1 and order=2 → position 1"
        );
    }

    #[test]
    fn effective_fields_and_groups_tie_fields_before_groups() {
        // field(order=0) and group(order=0): fields come before groups on equal order.
        let rt = make_type_with_groups(
            "t",
            vec![fa("f1", 0, false)],
            Some(vec![make_group("g1", 0)]),
            None,
        );
        let pkg = make_package_with_types(vec![rt.clone()]);
        let result = pkg.effective_fields_and_groups(&rt).unwrap();
        assert_eq!(result.groups.len(), 1);
        assert_eq!(
            result.groups[0].merged_position, 2,
            "field and group both at order=0; field goes first → group gets position 2"
        );
    }
}
