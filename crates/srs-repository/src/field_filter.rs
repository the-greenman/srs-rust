//! RFC-056 — `DiscoveryQuery.fieldFilters`: tests on a Tier 2 Record's own field values.
//!
//! [`compile`] checks a query's filters against the loaded package once, in RFC-056's
//! failure order — every malformed filter first ([R7], `query-filter-malformed`), then every
//! filter that does not fit its resolved Field ([R8], `query-filter-misfit`) — and returns
//! the compiled filters plus the ids that resolve to no Field ([R6]: the query selects
//! nothing, with a warning). [`CompiledFieldFilters::matches`] is then evaluated per record
//! by the one structured-predicate evaluator shared by `find` and `discovery-query` sections.

use std::collections::HashSet;

use chrono::NaiveDate;
use serde_json::Value;
use srs_core::types::discovery::{FieldFilter, FieldFilterRange};
use srs_core::types::field_type::{AllowedValue, Cardinality, Datatype};
use srs_core::types::record::Record;
use srs_core::validation::value_shape::integral_i64;

use crate::error::RepositoryError;
use crate::package::Package;

/// The compiled `fieldFilters` of one query. Empty when the query has none.
#[derive(Debug, Default)]
pub struct CompiledFieldFilters {
    filters: Vec<Compiled>,
    /// `fieldId`s that resolve to no installed Field. Any one makes the query select
    /// nothing ([R6]); the caller reports a warning naming each.
    pub unresolved: Vec<String>,
}

#[derive(Debug)]
struct Compiled {
    /// The record key the value is read under: the resolved Field's `name` (RFC-039).
    name: String,
    /// `(typeId, typeVersion)` whose effective field set contains the Field ([R6]).
    types: HashSet<(String, u32)>,
    datatype: Datatype,
    list: bool,
    domain: Domain,
    test: Test,
}

#[derive(Debug)]
enum Domain {
    Open,
    /// A closed Field's `allowedValues`.
    Allowed(Vec<AllowedValue>),
    /// A Field bound to a Vocabulary: values resolve to term keys ([R3]).
    Vocabulary {
        terms: Vec<(String, Vec<String>)>,
        closed: bool,
    },
}

#[derive(Debug)]
enum Test {
    In(Vec<Value>),
    NotIn(Vec<Value>),
    Range(Option<Bound>, Option<Bound>),
    Missing(bool),
}

#[derive(Debug, Clone, Copy)]
enum Bound {
    Num(Num),
    /// A date bound as its first and last day ([R4]: a coarse bound covers its period).
    Date(NaiveDate, NaiveDate),
}

/// A JSON number: exact for integers, `f64` otherwise.
// ponytail: decimals compare as f64 (the workspace serde_json has no `arbitrary_precision`),
// so two decimals differing beyond ~15 significant digits compare equal; integers that fit
// i128 compare exactly. Full exactness per RFC-056 [R4] needs `arbitrary_precision` (srs-rust#1692).
#[derive(Debug, Clone, Copy)]
struct Num {
    int: Option<i128>,
    float: f64,
}

impl Num {
    fn of(v: &Value) -> Option<Num> {
        let n = v.as_number()?;
        let int = n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| integral_i64(v).map(i128::from));
        Some(Num {
            int,
            float: n.as_f64()?,
        })
    }

    fn cmp(self, other: Num) -> std::cmp::Ordering {
        match (self.int, other.int) {
            (Some(a), Some(b)) => a.cmp(&b),
            _ => self.float.total_cmp(&other.float),
        }
    }
}

fn malformed(msg: String) -> RepositoryError {
    RepositoryError::QueryFilterRefused {
        code: "query-filter-malformed",
        message: msg,
    }
}

fn misfit(msg: String) -> RepositoryError {
    RepositoryError::QueryFilterRefused {
        code: "query-filter-misfit",
        message: msg,
    }
}

/// A date bound: `YYYY`, `YYYY-MM` or `YYYY-MM-DD`, naming a real year, month or day.
fn parse_date_bound(s: &str) -> Option<(NaiveDate, NaiveDate)> {
    let parts: Vec<&str> = s.split('-').collect();
    let num = |p: &str, len: usize| {
        (p.len() == len && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<u32>().ok())
            .flatten()
    };
    match parts.as_slice() {
        [y] => {
            let y = num(y, 4)? as i32;
            Some((
                NaiveDate::from_ymd_opt(y, 1, 1)?,
                NaiveDate::from_ymd_opt(y, 12, 31)?,
            ))
        }
        [y, m] => {
            let (y, m) = (num(y, 4)? as i32, num(m, 2)?);
            let start = NaiveDate::from_ymd_opt(y, m, 1)?;
            let next = if m == 12 {
                NaiveDate::from_ymd_opt(y + 1, 1, 1)?
            } else {
                NaiveDate::from_ymd_opt(y, m + 1, 1)?
            };
            Some((start, next.pred_opt()?))
        }
        [y, m, d] => {
            let day = NaiveDate::from_ymd_opt(num(y, 4)? as i32, num(m, 2)?, num(d, 2)?)?;
            Some((day, day))
        }
        _ => None,
    }
}

/// A stored `date` value: a full calendar date.
fn parse_date(v: &Value) -> Option<NaiveDate> {
    let s = v.as_str()?;
    (s.len() == 10)
        .then(|| parse_date_bound(s))
        .flatten()
        .map(|(d, _)| d)
}

fn parse_bound(v: &Value) -> Option<Bound> {
    if let Some(n) = Num::of(v) {
        return Some(Bound::Num(n));
    }
    v.as_str()
        .and_then(parse_date_bound)
        .map(|(a, b)| Bound::Date(a, b))
}

/// [R7]: checks that need no Field.
fn check_shape(i: usize, f: &FieldFilter) -> Result<(), RepositoryError> {
    let at = |m: &str| malformed(format!("fieldFilters[{i}]: {m}"));
    if f.field_id.as_deref().is_none_or(str::is_empty) {
        return Err(at("has no fieldId"));
    }
    let tests = [
        f.in_values.is_some(),
        f.not_in.is_some(),
        f.range.is_some(),
        f.missing.is_some(),
    ];
    if tests.iter().filter(|t| **t).count() != 1 {
        return Err(at("must carry exactly one of in, notIn, range or missing"));
    }
    for (key, list) in [("in", &f.in_values), ("notIn", &f.not_in)] {
        let Some(list) = list else { continue };
        if list.is_empty() {
            return Err(at(&format!("{key} is empty")));
        }
        for v in list {
            let ok = match v {
                Value::String(s) => !s.is_empty(),
                Value::Bool(_) => true,
                Value::Number(_) => integral_i64(v).is_some() || v.as_u64().is_some(),
                _ => false,
            };
            if !ok {
                return Err(at(&format!(
                    "{key} lists {v}, which is not an integer, a boolean or a non-empty string"
                )));
            }
        }
    }
    if let Some(FieldFilterRange { from, to }) = &f.range {
        if from.is_none() && to.is_none() {
            return Err(at("range has neither from nor to"));
        }
        let mut kinds = Vec::new();
        for v in [from, to].into_iter().flatten() {
            match parse_bound(v) {
                Some(b) => kinds.push(matches!(b, Bound::Num(_))),
                None => return Err(at(&format!("range bound {v} is neither a number nor a real date, month or year (YYYY, YYYY-MM or YYYY-MM-DD)"))),
            }
        }
        if kinds.len() == 2 && kinds[0] != kinds[1] {
            return Err(at("range has one numeric bound and one date bound"));
        }
    }
    Ok(())
}

/// Compile a query's field filters against the effective package (RFC-056 Change D).
pub fn compile(
    package: &Package,
    filters: &[FieldFilter],
) -> Result<CompiledFieldFilters, RepositoryError> {
    if filters.is_empty() {
        return Ok(CompiledFieldFilters::default());
    }
    // [R7] over the whole query before anything else.
    for (i, f) in filters.iter().enumerate() {
        check_shape(i, f)?;
    }
    let mut out = CompiledFieldFilters::default();
    for (i, f) in filters.iter().enumerate() {
        let id = f.field_id.as_deref().unwrap_or_default();
        let Some(field) = package.resolve_field(id) else {
            out.unresolved.push(id.to_string());
            continue;
        };
        let at = |m: String| {
            misfit(format!(
                "fieldFilters[{i}] on Field '{}' ({id}): {m}",
                field.name
            ))
        };
        let ft = &field.field_type;
        let dt = ft.datatype;
        let test = if let Some(v) = &f.in_values {
            Test::In(v.clone())
        } else if let Some(v) = &f.not_in {
            Test::NotIn(v.clone())
        } else if let Some(FieldFilterRange { from, to }) = &f.range {
            Test::Range(
                from.as_ref().and_then(parse_bound),
                to.as_ref().and_then(parse_bound),
            )
        } else {
            Test::Missing(f.missing.unwrap_or(false))
        };
        // [R8]: the test must suit the datatype (RFC-056 Change B).
        let allowed = matches!(
            (&test, &dt),
            (Test::Missing(_), _)
                | (
                    Test::In(_) | Test::NotIn(_),
                    Datatype::String | Datatype::Integer | Datatype::Boolean
                )
                | (
                    Test::Range(..),
                    Datatype::Date | Datatype::Integer | Datatype::Number
                )
        );
        if !allowed {
            return Err(at(format!(
                "the test does not apply to a {} Field",
                dt.as_str()
            )));
        }
        if let Test::Range(from, to) = &test {
            let want_num = dt != Datatype::Date;
            if [from, to]
                .into_iter()
                .flatten()
                .any(|b| matches!(b, Bound::Num(_)) != want_num)
            {
                return Err(at(format!(
                    "a range bound of the wrong kind for a {} Field",
                    dt.as_str()
                )));
            }
        }
        let domain = match (&ft.vocabulary_ref, ft.allowed_values()) {
            (_, Some(av)) if ft.is_closed() => Domain::Allowed(av.to_vec()),
            (Some(vid), _) => match package.resolve_vocabulary(vid) {
                Some(v) => Domain::Vocabulary {
                    terms: v
                        .effective_terms()
                        .into_iter()
                        .map(|t| (t.key.clone(), t.aliases.clone().unwrap_or_default()))
                        .collect(),
                    closed: ft.is_closed(),
                },
                None if ft.is_closed() => {
                    return Err(at(format!(
                        "its vocabularyRef '{vid}' resolves to no Vocabulary"
                    )))
                }
                // Open Field, unresolvable Vocabulary: values compare as written ([R3]).
                None => Domain::Open,
            },
            _ => Domain::Open,
        };
        if let Test::In(list) | Test::NotIn(list) = &test {
            for v in list {
                let fits = match dt {
                    Datatype::String => v.is_string(),
                    Datatype::Integer => integral_i64(v).is_some() || v.as_u64().is_some(),
                    _ => v.is_boolean(),
                };
                if !fits {
                    return Err(at(format!(
                        "lists {v}, which is not a {} value",
                        dt.as_str()
                    )));
                }
                if !domain.admits(v) {
                    return Err(at(format!(
                        "lists {v}, which is outside the Field's closed domain"
                    )));
                }
            }
        }
        let mut types = HashSet::new();
        for rt in &package.record_types {
            if let Ok(fas) = package.effective_fields(rt) {
                if fas.iter().any(|fa| fa.field_id == field.id) {
                    types.insert((rt.id.clone(), rt.version));
                }
            }
        }
        out.filters.push(Compiled {
            name: field.name.clone(),
            types,
            list: ft.cardinality == Some(Cardinality::List),
            datatype: dt,
            domain,
            test,
        });
    }
    Ok(out)
}

impl Domain {
    /// The term key a string resolves to: key match first, then alias (RFC-006).
    fn resolve<'a>(&'a self, s: &'a str) -> Option<&'a str> {
        match self {
            Domain::Vocabulary { terms, .. } => terms
                .iter()
                .find(|(k, _)| k == s)
                .or_else(|| terms.iter().find(|(_, a)| a.iter().any(|x| x == s)))
                .map(|(k, _)| k.as_str()),
            _ => Some(s),
        }
    }

    /// A closed domain admits only its values; an open one admits anything.
    fn admits(&self, v: &Value) -> bool {
        match self {
            Domain::Open => true,
            Domain::Allowed(av) => match (v.as_str(), integral_i64(v)) {
                (Some(s), _) => av.contains(&AllowedValue::String(s.to_string())),
                (None, Some(i)) => av.contains(&AllowedValue::Integer(i)),
                _ => false,
            },
            Domain::Vocabulary { closed, .. } => {
                !closed || v.as_str().is_some_and(|s| self.resolve(s).is_some())
            }
        }
    }

    /// The comparison key of a string value: its term key when it resolves, else as written.
    fn canonical<'a>(&'a self, s: &'a str) -> &'a str {
        self.resolve(s).unwrap_or(s)
    }
}

impl Compiled {
    /// Whether one element conforms to the Field's datatype and closed domain ([R5]).
    fn conforms(&self, v: &Value) -> bool {
        let shape = match self.datatype {
            Datatype::String => v.is_string(),
            Datatype::Integer => integral_i64(v).is_some() || v.as_u64().is_some(),
            Datatype::Number => v.is_number(),
            Datatype::Boolean => v.is_boolean(),
            Datatype::Date => parse_date(v).is_some(),
            _ => true,
        };
        shape && self.domain.admits(v)
    }

    fn equals(&self, stored: &Value, listed: &Value) -> bool {
        match self.datatype {
            Datatype::String => match (stored.as_str(), listed.as_str()) {
                (Some(a), Some(b)) => self.domain.canonical(a) == self.domain.canonical(b),
                _ => false,
            },
            Datatype::Integer => match (Num::of(stored), Num::of(listed)) {
                (Some(a), Some(b)) => a.cmp(b).is_eq(),
                _ => false,
            },
            _ => stored == listed,
        }
    }

    fn in_range(&self, v: &Value, from: Option<Bound>, to: Option<Bound>) -> bool {
        match self.datatype {
            Datatype::Date => {
                let Some(d) = parse_date(v) else { return false };
                from.is_none_or(|b| matches!(b, Bound::Date(start, _) if d >= start))
                    && to.is_none_or(|b| matches!(b, Bound::Date(_, end) if d <= end))
            }
            _ => {
                let Some(n) = Num::of(v) else { return false };
                from.is_none_or(|b| matches!(b, Bound::Num(lo) if n.cmp(lo).is_ge()))
                    && to.is_none_or(|b| matches!(b, Bound::Num(hi) if n.cmp(hi).is_le()))
            }
        }
    }

    fn holds(&self, record: &Record) -> bool {
        // [R6]: only a record whose bound Type version's effective field set has the Field.
        if !self
            .types
            .contains(&(record.type_id.clone(), record.type_version))
        {
            return false;
        }
        let value = record.field_values.get(&self.name);
        // [R5]: absent, "", [] and {} are no value.
        let has_value = match value {
            None | Some(Value::Null) => false,
            Some(Value::String(s)) => !s.is_empty(),
            Some(Value::Array(a)) => !a.is_empty(),
            Some(Value::Object(o)) => !o.is_empty(),
            Some(_) => true,
        };
        if let Test::Missing(m) = self.test {
            return has_value != m;
        }
        let Some(value) = value.filter(|_| has_value) else {
            return false;
        };
        // Each element judged on its own; a list Field must hold a list, a single Field must not.
        let elements: Vec<&Value> = match (self.list, value) {
            (true, Value::Array(a)) => a.iter().filter(|e| self.conforms(e)).collect(),
            (false, v) if !v.is_array() && self.conforms(v) => vec![v],
            _ => Vec::new(),
        };
        match &self.test {
            Test::In(list) => elements
                .iter()
                .any(|e| list.iter().any(|l| self.equals(e, l))),
            Test::NotIn(list) => {
                !elements.is_empty()
                    && !elements
                        .iter()
                        .any(|e| list.iter().any(|l| self.equals(e, l)))
            }
            Test::Range(from, to) => elements.iter().any(|e| self.in_range(e, *from, *to)),
            Test::Missing(_) => unreachable!(),
        }
    }
}

impl CompiledFieldFilters {
    /// True when the query has field filters (so Notes can never match).
    pub fn is_active(&self) -> bool {
        !self.filters.is_empty() || !self.unresolved.is_empty()
    }

    /// Every filter holds for `record` ([R1]). An unresolved filter never holds ([R6]).
    pub fn matches(&self, record: &Record) -> bool {
        self.unresolved.is_empty() && self.filters.iter().all(|f| f.holds(record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use srs_core::types::field::Field;
    use srs_core::types::record_type::RecordType;
    use std::path::PathBuf;

    const T: &str = "aa000000-0000-4000-a000-000000000001";
    const STATUS: &str = "aa000000-0000-4000-a000-0000000000f1";
    const WHEN: &str = "aa000000-0000-4000-a000-0000000000f2";
    const MINUTES: &str = "aa000000-0000-4000-a000-0000000000f3";
    const WORDS: &str = "aa000000-0000-4000-a000-0000000000f4";
    const KIND: &str = "aa000000-0000-4000-a000-0000000000f5";
    const VOCAB: &str = "aa000000-0000-4000-a000-0000000000c1";

    fn field(id: &str, name: &str, ft: serde_json::Value) -> Field {
        serde_json::from_value(json!({
            "id": id, "namespace": "t", "name": name, "version": 1, "description": name,
            "aiGuidance": {"purpose": "t"}, "fieldType": ft, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    fn package() -> Package {
        let assignments: Vec<serde_json::Value> = [STATUS, WHEN, MINUTES, WORDS, KIND]
            .iter()
            .enumerate()
            .map(|(i, f)| json!({"fieldId": f, "order": i + 1, "required": false}))
            .collect();
        let rt: RecordType = serde_json::from_value(json!({
            "id": T, "namespace": "t", "name": "doc", "version": 1, "description": "doc",
            "fields": assignments,
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let vocab = serde_json::from_value(json!({
            "id": VOCAB, "version": 1, "namespace": "t", "name": "kinds", "mode": "closed", "createdAt": "2026-01-01T00:00:00Z",
            "terms": [
                {"id": "aa000000-0000-4000-a000-0000000000d1", "version": 1, "namespace": "t", "key": "email", "aliases": ["e-mail"], "createdAt": "2026-01-01T00:00:00Z"},
                {"id": "aa000000-0000-4000-a000-0000000000d2", "version": 1, "namespace": "t", "key": "letter", "status": "retired", "createdAt": "2026-01-01T00:00:00Z"}
            ]
        }))
        .unwrap();
        Package {
            id: "aa000000-0000-4000-a000-0000000000ff".into(),
            namespace: "t".into(),
            name: "t".into(),
            version: "1.0.0".into(),
            fields: vec![
                field(
                    STATUS,
                    "status",
                    json!({"datatype": "string", "valueDomain": "closed", "allowedValues": ["draft", "published"]}),
                ),
                field(WHEN, "when", json!({"datatype": "date"})),
                field(MINUTES, "minutes", json!({"datatype": "integer"})),
                field(WORDS, "words", json!({"datatype": "number"})),
                field(
                    KIND,
                    "kind",
                    json!({"datatype": "string", "valueDomain": "closed", "vocabularyRef": VOCAB}),
                ),
            ],
            record_types: vec![rt],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![vocab],
            lifecycles: vec![],
        }
    }

    fn filters(v: serde_json::Value) -> Vec<FieldFilter> {
        serde_json::from_value(v).unwrap()
    }

    fn code(v: serde_json::Value) -> &'static str {
        compile(&package(), &filters(v)).unwrap_err().code()
    }

    fn record(values: serde_json::Value) -> Record {
        serde_json::from_value(json!({
            "instanceId": "aa000000-0000-4000-a000-00000000aaaa", "typeId": T, "typeVersion": 1,
            "typeNamespace": "t", "typeName": "doc", "fieldValues": values, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    fn holds(f: serde_json::Value, values: serde_json::Value) -> bool {
        compile(&package(), &filters(json!([f])))
            .unwrap()
            .matches(&record(values))
    }

    #[test]
    fn r7_malformed_filters_are_refused() {
        for f in [
            json!([{"in": ["x"]}]),
            json!([{"fieldId": STATUS}]),
            json!([{"fieldId": STATUS, "in": ["draft"], "missing": true}]),
            json!([{"fieldId": STATUS, "in": []}]),
            json!([{"fieldId": STATUS, "in": [""]}]),
            json!([{"fieldId": STATUS, "in": [2.5]}]),
            json!([{"fieldId": WHEN, "range": {}}]),
            json!([{"fieldId": WHEN, "range": {"from": "2026-02-30"}}]),
            json!([{"fieldId": WHEN, "range": {"from": "2026-13"}}]),
            json!([{"fieldId": WHEN, "range": {"from": "2026-08", "to": 5}}]),
        ] {
            assert_eq!(code(f.clone()), "query-filter-malformed", "{f}");
        }
    }

    #[test]
    fn r8_misfits_are_refused() {
        for f in [
            json!([{"fieldId": STATUS, "range": {"from": "2026"}}]),
            json!([{"fieldId": WORDS, "in": [4]}]),
            json!([{"fieldId": MINUTES, "in": ["4"]}]),
            json!([{"fieldId": WHEN, "range": {"from": 2026}}]),
            json!([{"fieldId": WORDS, "range": {"from": "2026"}}]),
            json!([{"fieldId": STATUS, "in": ["publshed"]}]),
            json!([{"fieldId": KIND, "in": ["letter"]}]),
        ] {
            assert_eq!(code(f.clone()), "query-filter-misfit", "{f}");
        }
    }

    #[test]
    fn failure_order_malformed_before_misfit_before_unresolved() {
        let misfit_then_malformed =
            json!([{"fieldId": STATUS, "in": ["publshed"]}, {"fieldId": WHEN, "range": {}}]);
        assert_eq!(code(misfit_then_malformed), "query-filter-malformed");
        let unknown_then_misfit = json!([{"fieldId": "aa000000-0000-4000-a000-00000000dead", "in": ["x"]}, {"fieldId": STATUS, "in": ["nope"]}]);
        assert_eq!(code(unknown_then_misfit), "query-filter-misfit");
    }

    #[test]
    fn r6_unresolved_field_selects_nothing() {
        let c = compile(
            &package(),
            &filters(json!([{"fieldId": "aa000000-0000-4000-a000-00000000dead", "missing": true}])),
        )
        .unwrap();
        assert_eq!(c.unresolved, vec!["aa000000-0000-4000-a000-00000000dead"]);
        assert!(!c.matches(&record(json!({}))));
    }

    #[test]
    fn semantics() {
        // notIn skips no value; empty string is no value.
        assert!(!holds(
            json!({"fieldId": STATUS, "notIn": ["published"]}),
            json!({})
        ));
        assert!(holds(
            json!({"fieldId": STATUS, "missing": true}),
            json!({"status": ""})
        ));
        assert!(holds(
            json!({"fieldId": STATUS, "notIn": ["published"]}),
            json!({"status": "draft"})
        ));
        // Aliases resolve on both sides; a stored retired value never matches.
        assert!(holds(
            json!({"fieldId": KIND, "in": ["e-mail"]}),
            json!({"kind": "email"})
        ));
        assert!(!holds(
            json!({"fieldId": KIND, "notIn": ["email"]}),
            json!({"kind": "letter"})
        ));
        // Partial dates cover their period.
        assert!(holds(
            json!({"fieldId": WHEN, "range": {"from": "2026-08", "to": "2026-08"}}),
            json!({"when": "2026-08-31"})
        ));
        assert!(!holds(
            json!({"fieldId": WHEN, "range": {"to": "2026-07"}}),
            json!({"when": "2026-08-01"})
        ));
        assert!(!holds(
            json!({"fieldId": WHEN, "range": {"from": "2026-08", "to": "2026-06"}}),
            json!({"when": "2026-07-01"})
        ));
        // Numbers: exact integers, inclusive bounds, fractional bound on an integer, 12.0 is 12.
        assert!(holds(
            json!({"fieldId": MINUTES, "in": [12]}),
            json!({"minutes": 12.0})
        ));
        assert!(holds(
            json!({"fieldId": MINUTES, "range": {"from": 2.5, "to": 3}}),
            json!({"minutes": 3})
        ));
        assert!(!holds(
            json!({"fieldId": MINUTES, "range": {"from": 2.5}}),
            json!({"minutes": 2})
        ));
        assert!(holds(
            json!({"fieldId": MINUTES, "range": {"from": 9007199254740993_u64}}),
            json!({"minutes": 9007199254740993_u64})
        ));
        assert!(!holds(
            json!({"fieldId": MINUTES, "range": {"from": 9007199254740993_u64}}),
            json!({"minutes": 9007199254740992_u64})
        ));
        assert!(holds(
            json!({"fieldId": WORDS, "range": {"from": 40, "to": 80}}),
            json!({"words": 80})
        ));
        assert!(!holds(
            json!({"fieldId": WORDS, "range": {"from": 40}}),
            json!({"words": "42"})
        ));
        // A non-integral number on an integer Field does not conform.
        assert!(!holds(
            json!({"fieldId": MINUTES, "range": {"from": 0}}),
            json!({"minutes": 2.5})
        ));
    }
}
