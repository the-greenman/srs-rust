//! RFC-039 Change B — the one recursive value rule.
//!
//! This is the **only** implementation of the carrier's value grammar: it is
//! the instance space of RFC-032 `projectField`, restated from the value side.
//! Instance validation consumes it directly; the migration verifies its output
//! through it; a conformance test asserts agreement with the schema emitter.
//! Do not restate the `fieldType → shape` branching anywhere else (RFC-039:
//! "a second grammar would be a second source of truth").

use crate::error::CoreError;
use crate::types::field_type::{Datatype, FieldType, MapValueRange, RefMode};
use crate::types::record::json_type_name;

/// Resolved view of one entry in a Type's effective field set — the
/// assignment joined to its Field definition. Built by the caller
/// (srs-repository owns package resolution; srs-core has no I/O).
#[derive(Debug, Clone)]
pub struct EffectiveField {
    pub field_id: String,
    /// `Field.name` — the verbatim instance key ([R2b]).
    pub name: String,
    pub required: bool,
    pub order: u32,
    /// Absent only for unresolvable Fields — the caller reports that as its
    /// own diagnostic; value validation skips shape checks without one.
    pub field_type: Option<FieldType>,
}

/// Resolves an inline `ref` range to its effective field set, recursively.
/// Implemented by srs-repository over the loaded package; tests use closures.
pub trait RangeResolver {
    fn effective_fields(&self, type_id: &str, type_version: u32) -> Option<Vec<EffectiveField>>;
}

impl<F> RangeResolver for F
where
    F: Fn(&str, u32) -> Option<Vec<EffectiveField>>,
{
    fn effective_fields(&self, type_id: &str, type_version: u32) -> Option<Vec<EffectiveField>> {
        self(type_id, type_version)
    }
}

/// Validate one value against its Field's `fieldType` — Change B's single-case
/// table, with [R16]'s uniform list wrap composed on top and [R3]'s recursive
/// descent into inline composites. `key` is the dotted path used in
/// diagnostics (e.g. `rows[2].cells`).
pub fn validate_value(
    key: &str,
    field_type: &FieldType,
    value: &serde_json::Value,
    resolver: &dyn RangeResolver,
    diagnostics: &mut Vec<CoreError>,
) {
    // [R5]: null is never a value, at any depth, under any cardinality.
    if value.is_null() {
        diagnostics.push(CoreError::NullFieldValue {
            key: key.to_string(),
        });
        return;
    }

    if field_type.is_list() {
        // [R16]: cardinality "list" array-wraps uniformly, for every datatype.
        let Some(items) = value.as_array() else {
            diagnostics.push(CoreError::ValueShape {
                key: key.to_string(),
                expected: format!("array ({} list)", field_type.datatype.as_str()),
                got: json_type_name(value).to_string(),
            });
            return;
        };
        for (i, item) in items.iter().enumerate() {
            validate_single(
                &format!("{key}[{i}]"),
                field_type,
                item,
                resolver,
                diagnostics,
            );
        }
    } else {
        validate_single(key, field_type, value, resolver, diagnostics);
    }
}

/// The `single` case — exactly one row of Change B's table matches.
fn validate_single(
    key: &str,
    field_type: &FieldType,
    value: &serde_json::Value,
    resolver: &dyn RangeResolver,
    diagnostics: &mut Vec<CoreError>,
) {
    if value.is_null() {
        diagnostics.push(CoreError::NullFieldValue {
            key: key.to_string(),
        });
        return;
    }
    match field_type.datatype {
        Datatype::String => expect(key, value.is_string(), "string", value, diagnostics),
        Datatype::Number => expect(key, value.is_number(), "number", value, diagnostics),
        Datatype::Integer => expect(
            key,
            value.as_i64().is_some() || value.as_u64().is_some(),
            "integer",
            value,
            diagnostics,
        ),
        Datatype::Boolean => expect(key, value.is_boolean(), "boolean", value, diagnostics),
        // Portable scalar table: dates are ISO-8601 strings on the wire.
        Datatype::Date | Datatype::DateTime => {
            expect(
                key,
                value.is_string(),
                "ISO-8601 string",
                value,
                diagnostics,
            );
        }
        Datatype::Ref => match field_type.effective_mode() {
            RefMode::Reference => {
                // Wire shape only — target existence/type is [R14], checked by
                // the caller with repository context.
                expect(
                    key,
                    value.is_string(),
                    "instance-id string",
                    value,
                    diagnostics,
                );
            }
            RefMode::Inline => {
                // [R3]: an inline composite value IS a fieldValues map for the
                // rangeType, validated by the same rule at every depth.
                let Some(obj) = value.as_object() else {
                    diagnostics.push(CoreError::ValueShape {
                        key: key.to_string(),
                        expected: "object (inline composite fieldValues map)".to_string(),
                        got: json_type_name(value).to_string(),
                    });
                    return;
                };
                let Some(range) = &field_type.range_type else {
                    diagnostics.push(CoreError::InvalidFieldValue {
                        key: key.to_string(),
                        reason: "inline ref field declares no rangeType".to_string(),
                    });
                    return;
                };
                let Some(range_fields) =
                    resolver.effective_fields(&range.type_id, range.type_version)
                else {
                    diagnostics.push(CoreError::InvalidFieldValue {
                        key: key.to_string(),
                        reason: format!(
                            "rangeType {}@{} does not resolve",
                            range.type_id, range.type_version
                        ),
                    });
                    return;
                };
                validate_field_values_map(key, obj, &range_fields, resolver, diagnostics);
            }
        },
        Datatype::Map => {
            let Some(obj) = value.as_object() else {
                diagnostics.push(CoreError::ValueShape {
                    key: key.to_string(),
                    expected: "object (map)".to_string(),
                    got: json_type_name(value).to_string(),
                });
                return;
            };
            if let Some(MapValueRange::String) = field_type.value_range {
                for (k, v) in obj {
                    if !v.is_string() {
                        diagnostics.push(CoreError::ValueShape {
                            key: format!("{key}.{k}"),
                            expected: "string (map valueRange)".to_string(),
                            got: json_type_name(v).to_string(),
                        });
                    }
                }
            }
        }
        // [R3] obligation for `dependent` is the descriptor named by
        // `dependsOn` — a sibling-value-directed check that needs the whole
        // record; the record-level validator owns it. Wire shape here is
        // unconstrained.
        Datatype::Dependent => {}
    }
}

/// Validate a name-keyed fieldValues map against an effective field set —
/// [R1] (keys resolve; unknown keys rejected), [R5] (required ⇒ present),
/// and [R3] per value. Shared by the record validator (depth 0) and the
/// inline-composite recursion (depth ≥ 1).
pub fn validate_field_values_map(
    path: &str,
    map: &serde_json::Map<String, serde_json::Value>,
    effective_fields: &[EffectiveField],
    resolver: &dyn RangeResolver,
    diagnostics: &mut Vec<CoreError>,
) {
    let at = |k: &str| {
        if path.is_empty() {
            k.to_string()
        } else {
            format!("{path}.{k}")
        }
    };

    // [R1]: every key names a Field in the effective set.
    for key in map.keys() {
        if !effective_fields.iter().any(|f| f.name == *key) {
            diagnostics.push(CoreError::UnknownFieldKey { key: at(key) });
        }
    }

    // [R5]: required ⇒ key present.
    for field in effective_fields {
        if field.required && !map.contains_key(&field.name) {
            diagnostics.push(CoreError::MissingRequiredField {
                key: at(&field.name),
            });
        }
    }

    // [R3]: each present value conforms to its Field's fieldType.
    for field in effective_fields {
        let Some(value) = map.get(&field.name) else {
            continue;
        };
        let Some(field_type) = &field.field_type else {
            continue; // unresolvable Field — reported by the caller
        };
        validate_value(&at(&field.name), field_type, value, resolver, diagnostics);
    }
}

fn expect(
    key: &str,
    ok: bool,
    expected: &str,
    value: &serde_json::Value,
    diagnostics: &mut Vec<CoreError>,
) {
    if !ok {
        diagnostics.push(CoreError::ValueShape {
            key: key.to_string(),
            expected: expected.to_string(),
            got: json_type_name(value).to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::field_type::ExactTypeRef;
    use serde_json::json;

    fn no_resolver() -> impl RangeResolver {
        |_: &str, _: u32| -> Option<Vec<EffectiveField>> { None }
    }

    fn ef(name: &str, required: bool, ft: FieldType) -> EffectiveField {
        EffectiveField {
            field_id: format!("id-{name}"),
            name: name.to_string(),
            required,
            order: 0,
            field_type: Some(ft),
        }
    }

    #[test]
    fn scalar_types_validated() {
        let mut d = Vec::new();
        validate_value(
            "s",
            &FieldType::string(),
            &json!("ok"),
            &no_resolver(),
            &mut d,
        );
        validate_value(
            "n",
            &FieldType::number(),
            &json!(1.5),
            &no_resolver(),
            &mut d,
        );
        validate_value(
            "b",
            &FieldType::boolean(),
            &json!(true),
            &no_resolver(),
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");

        validate_value(
            "s",
            &FieldType::string(),
            &json!(42),
            &no_resolver(),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert!(matches!(&d[0], CoreError::ValueShape { key, .. } if key == "s"));
    }

    #[test]
    fn null_rejected_at_any_depth() {
        let mut d = Vec::new();
        validate_value(
            "x",
            &FieldType::string(),
            &json!(null),
            &no_resolver(),
            &mut d,
        );
        assert!(matches!(&d[0], CoreError::NullFieldValue { key } if key == "x"));

        let mut d = Vec::new();
        validate_value(
            "xs",
            &FieldType::string().into_list(),
            &json!(["a", null]),
            &no_resolver(),
            &mut d,
        );
        assert!(matches!(&d[0], CoreError::NullFieldValue { key } if key == "xs[1]"));
    }

    #[test]
    fn list_wrap_uniform_for_map_and_dependent() {
        // [R16]: the wrap applies to every datatype, map included.
        let map_list = FieldType::new(Datatype::Map).into_list();
        let mut d = Vec::new();
        validate_value(
            "m",
            &map_list,
            &json!([{"a": "x"}, {"b": "y"}]),
            &no_resolver(),
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");

        let mut d = Vec::new();
        validate_value("m", &map_list, &json!({"a": "x"}), &no_resolver(), &mut d);
        assert!(matches!(&d[0], CoreError::ValueShape { key, .. } if key == "m"));
    }

    #[test]
    fn reference_mode_is_id_string() {
        let ft = FieldType::instance_ref(ExactTypeRef {
            type_id: "t".into(),
            type_version: 1,
        });
        let mut d = Vec::new();
        validate_value("r", &ft, &json!("some-uuid"), &no_resolver(), &mut d);
        assert!(d.is_empty());
        validate_value("r", &ft, &json!({"nested": true}), &no_resolver(), &mut d);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn inline_composite_validates_depth_three() {
        // outer.rows[] -> row { cells: string[] , sub: inline composite { leaf: string } }
        let leaf_type = "leaf-type";
        let row_type = "row-type";
        let resolver = move |type_id: &str, _v: u32| -> Option<Vec<EffectiveField>> {
            match type_id {
                "row-type" => Some(vec![
                    ef("cells", true, FieldType::string().into_list()),
                    ef(
                        "sub",
                        false,
                        FieldType::inline_ref(ExactTypeRef {
                            type_id: "leaf-type".into(),
                            type_version: 1,
                        }),
                    ),
                ]),
                "leaf-type" => Some(vec![ef("leaf", true, FieldType::string())]),
                _ => None,
            }
        };
        let _ = (leaf_type, row_type);

        let rows_ft = FieldType::inline_ref(ExactTypeRef {
            type_id: "row-type".into(),
            type_version: 1,
        })
        .into_list();

        let good = json!([
            {"cells": ["a", "b"], "sub": {"leaf": "x"}},
            {"cells": ["c"]}
        ]);
        let mut d = Vec::new();
        validate_value("rows", &rows_ft, &good, &resolver, &mut d);
        assert!(d.is_empty(), "{d:?}");

        // depth-3 violation: leaf must be a string
        let bad = json!([{"cells": ["a"], "sub": {"leaf": 42}}]);
        let mut d = Vec::new();
        validate_value("rows", &rows_ft, &bad, &resolver, &mut d);
        assert_eq!(d.len(), 1);
        assert!(
            matches!(&d[0], CoreError::ValueShape { key, .. } if key == "rows[0].sub.leaf"),
            "{d:?}"
        );

        // unknown key inside a composite is rejected ([R1] at depth)
        let unknown = json!([{"cells": ["a"], "mystery": 1}]);
        let mut d = Vec::new();
        validate_value("rows", &rows_ft, &unknown, &resolver, &mut d);
        assert!(
            matches!(&d[0], CoreError::UnknownFieldKey { key } if key == "rows[0].mystery"),
            "{d:?}"
        );

        // required key missing inside a composite ([R5] at depth)
        let missing = json!([{}]);
        let mut d = Vec::new();
        validate_value("rows", &rows_ft, &missing, &resolver, &mut d);
        assert!(
            matches!(&d[0], CoreError::MissingRequiredField { key } if key == "rows[0].cells"),
            "{d:?}"
        );
    }
}
