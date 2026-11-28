//! RFC-056 — `DiscoveryQuery.fieldFilters`: tests on a Tier 2 Record's own field values.
//!
//! [`compile`] checks a query's filters against the loaded package once, in RFC-056's
//! failure order — every malformed filter first ([R7], `query-filter-malformed`), then every
//! filter that does not fit its resolved Field ([R8], `query-filter-misfit`) — and returns
//! the compiled filters plus the ids that resolve to no Field ([R6]: the query selects
//! nothing, with a warning). [`CompiledFieldFilters::matches`] is then evaluated per record
//! by the one structured-predicate evaluator shared by `find` and `discovery-query` sections.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde_json::Value;
use srs_core::types::discovery::{FieldFilter, FieldFilterRange};
use srs_core::types::field_type::{AllowedValue, Datatype, RefMode};
use srs_core::types::record::Record;
use srs_core::types::term::Term;
use srs_core::types::vocabulary::resolve_term_among;
use srs_core::validation::value_shape::integral_i64;

use crate::diagnostic::Diagnostics;
use crate::error::RepositoryError;
use crate::package::Package;

/// The compiled `fieldFilters` of one query. Empty when the query has none.
#[derive(Debug, Default)]
pub struct CompiledFieldFilters {
    filters: Vec<Compiled>,
    /// `fieldId`s that resolve to no installed Field. Any one makes the query select
    /// nothing ([R6]); the caller reports a warning naming each.
    pub unresolved: Vec<String>,
    /// Open Fields whose `vocabularyRef` resolves to nothing: their values compare as
    /// written, with a warning (RFC-056 Change B).
    pub warnings: Vec<String>,
}

#[derive(Debug)]
struct Compiled {
    /// The record key the value is read under: the resolved Field's `name` (RFC-039).
    name: String,
    /// `typeId → typeVersion`s whose effective field set contains the Field ([R6]).
    types: HashMap<String, HashSet<u32>>,
    datatype: Datatype,
    list: bool,
    /// `{}` is no value only on a `map` or inline `ref` Field ([R5]).
    empty_object_is_none: bool,
    domain: Domain,
    test: Test,
}

#[derive(Debug)]
enum Domain {
    Open,
    /// A closed Field's `allowedValues`.
    Allowed(Vec<AllowedValue>),
    /// A Field bound to a Vocabulary: values resolve to term keys over its effective
    /// entry set, `extends` followed and retired entries excluded ([R3]).
    Vocabulary {
        terms: Vec<Term>,
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

#[derive(Debug, Clone)]
enum Bound {
    Num(Num),
    /// A date bound as its first and last day ([R4]: a coarse bound covers its period).
    Date(NaiveDate, NaiveDate),
}

/// A JSON number held as the decimal it was written as ([R4]: no rounding).
///
/// The workspace `serde_json` keeps a number's written digits (`arbitrary_precision`), so
/// the value is `0.<digits> x 10^point` with no leading or trailing zero in `digits`
/// (empty means zero). Two numbers compare by sign, then point, then digits.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Num {
    negative: bool,
    digits: String,
    point: i128,
}

impl Num {
    fn of(v: &Value) -> Option<Num> {
        let n = v.as_number()?;
        // An integral float such as `12.0` is the integer 12 (see `integral_i64`).
        Num::parse(&n.to_string())
    }

    fn parse(text: &str) -> Option<Num> {
        let (negative, rest) = match text.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (mantissa, exp) = match rest.split_once(['e', 'E']) {
            Some((m, e)) => (m, e.parse::<i128>().ok()?),
            None => (rest, 0),
        };
        let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if int.is_empty() && frac.is_empty()
            || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let all = format!("{int}{frac}");
        let lead = all.len() - all.trim_start_matches('0').len();
        let digits = all.trim_matches('0').to_string();
        if digits.is_empty() {
            return Some(Num {
                negative: false,
                digits,
                point: 0,
            });
        }
        Some(Num {
            negative,
            digits,
            point: int.len() as i128 - lead as i128 + exp,
        })
    }

    /// Whether the value has no fractional part (`12.0` is integral, like `integral_i64`).
    fn is_integral(&self) -> bool {
        self.digits.is_empty() || self.digits.len() as i128 <= self.point
    }

    fn cmp(&self, other: &Num) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let zero = |n: &Num| n.digits.is_empty();
        match (zero(self), zero(other)) {
            (true, true) => return Equal,
            (true, false) => return if other.negative { Greater } else { Less },
            (false, true) => return if self.negative { Less } else { Greater },
            _ => {}
        }
        if self.negative != other.negative {
            return if self.negative { Less } else { Greater };
        }
        let magnitude = self
            .point
            .cmp(&other.point)
            .then_with(|| self.digits.cmp(&other.digits));
        if self.negative {
            magnitude.reverse()
        } else {
            magnitude
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
        if list.iter().any(|v| v.as_str() == Some("")) {
            return Err(at(&format!("{key} lists an empty string")));
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
            (Some(vid), _) => match package.effective_vocabulary_terms(vid) {
                Some(terms) => Domain::Vocabulary {
                    terms: terms.into_iter().cloned().collect(),
                    closed: ft.is_closed(),
                },
                None if ft.is_closed() => {
                    return Err(at(format!(
                        "its vocabularyRef '{vid}' resolves to no Vocabulary"
                    )))
                }
                // Open Field, unresolvable Vocabulary: values compare as written ([R3]).
                None => {
                    out.warnings.push(format!(
                        "fieldFilters[{i}] on Field '{}' ({id}): its vocabularyRef '{vid}' resolves to no Vocabulary; values compare as written",
                        field.name
                    ));
                    Domain::Open
                }
            },
            _ => Domain::Open,
        };
        if let Test::In(list) | Test::NotIn(list) = &test {
            for v in list {
                let fits = match dt {
                    Datatype::String => v.is_string(),
                    Datatype::Integer => Num::of(v).is_some_and(|n| n.is_integral()),
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
        let mut types: HashMap<String, HashSet<u32>> = HashMap::new();
        for rt in &package.record_types {
            if let Ok(fas) = package.effective_fields(rt) {
                if fas.iter().any(|fa| fa.field_id == field.id) {
                    types.entry(rt.id.clone()).or_default().insert(rt.version);
                }
            }
        }
        out.filters.push(Compiled {
            name: field.name.clone(),
            types,
            list: ft.is_list(),
            empty_object_is_none: dt == Datatype::Map
                || (dt == Datatype::Ref && ft.effective_mode() == RefMode::Inline),
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
            Domain::Vocabulary { terms, .. } => {
                resolve_term_among(&terms.iter().collect::<Vec<_>>(), s).map(|t| t.key.as_str())
            }
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
            Datatype::Integer => Num::of(v).is_some_and(|n| n.is_integral()),
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
                (Some(a), Some(b)) => a.cmp(&b).is_eq(),
                _ => false,
            },
            _ => stored == listed,
        }
    }

    fn in_range(&self, v: &Value, from: Option<&Bound>, to: Option<&Bound>) -> bool {
        match self.datatype {
            Datatype::Date => {
                let Some(d) = parse_date(v) else { return false };
                from.is_none_or(|b| matches!(b, Bound::Date(start, _) if d >= *start))
                    && to.is_none_or(|b| matches!(b, Bound::Date(_, end) if d <= *end))
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
            .get(record.type_id.as_str())
            .is_some_and(|v| v.contains(&record.type_version))
        {
            return false;
        }
        let value = record.field_values.get(&self.name);
        // [R5]: absent, "", [] and (on a map or inline ref Field) {} are no value.
        let has_value = match value {
            None | Some(Value::Null) => false,
            Some(Value::String(s)) => !s.is_empty(),
            Some(Value::Array(a)) => !a.is_empty(),
            Some(Value::Object(o)) => !(o.is_empty() && self.empty_object_is_none),
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
            Test::Range(from, to) => elements
                .iter()
                .any(|e| self.in_range(e, from.as_ref(), to.as_ref())),
            Test::Missing(_) => unreachable!(),
        }
    }
}

/// [`compile`], then report each unresolved `fieldId` ([R6]) and unresolvable open
/// `vocabularyRef` (Change B) as a warning. The one entry point for `find`, `similar` and
/// `discovery-query` sections.
pub(crate) fn compile_reporting(
    package: &Package,
    filters: &[FieldFilter],
    diagnostics: &mut Diagnostics,
) -> Result<CompiledFieldFilters, RepositoryError> {
    let compiled = compile(package, filters)?;
    for id in &compiled.unresolved {
        diagnostics.warning(
            "field-filter-unknown",
            format!("warning: fieldFilters fieldId '{id}' names no Field; no instances can match"),
        );
    }
    for w in &compiled.warnings {
        diagnostics.warning(
            "field-filter-vocabulary-unresolved",
            format!("warning: {w}"),
        );
    }
    Ok(compiled)
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
    fn r4_decimals_compare_exactly_beyond_f64() {
        let range = |from: &str, to: &str, w: &str| {
            let f: FieldFilter = serde_json::from_str(&format!(
                r#"{{"fieldId":"{WORDS}","range":{{"from":{from},"to":{to}}}}}"#
            ))
            .unwrap();
            let r: Record = serde_json::from_value(json!({
                "instanceId": "aa000000-0000-4000-a000-00000000aaaa", "typeId": T, "typeVersion": 1,
                "typeNamespace": "t", "typeName": "doc", "createdAt": "2026-01-01T00:00:00Z",
                "fieldValues": {"words": serde_json::from_str::<serde_json::Value>(w).unwrap()}
            }))
            .unwrap();
            compile(&package(), &[f]).unwrap().matches(&r)
        };
        let long = "0.1000000000000000055511151231257827";
        // Equal to 0.1 as f64, but not as written: 0.1 is below the long decimal.
        assert!(!range(long, long, "0.1"));
        assert!(range(long, long, long));
        assert!(range("0.1", "0.1", "1e-1"));
        assert!(range("-0.5", "0.0", "-0.25"));
        assert!(range("-0.5", "-0.25", "-0.2500000000000000000001"));
        assert!(!range("-0.5", "-0.25", "-0.2499999999999999999999"));
        assert!(range("0", "0", "-0.0"));
    }

    #[test]
    fn r4_integers_beyond_u64_compare_exactly() {
        let big = "18446744073709551617"; // 2^64 + 1
        let next = "18446744073709551618";
        let f = |min: &str| {
            let f: FieldFilter =
                serde_json::from_str(&format!(r#"{{"fieldId":"{MINUTES}","in":[{min}]}}"#))
                    .unwrap();
            compile(&package(), &[f]).unwrap()
        };
        let r: Record = serde_json::from_value(json!({
            "instanceId": "aa000000-0000-4000-a000-00000000aaaa", "typeId": T, "typeVersion": 1,
            "typeNamespace": "t", "typeName": "doc", "createdAt": "2026-01-01T00:00:00Z",
            "fieldValues": {"minutes": serde_json::from_str::<serde_json::Value>(big).unwrap()}
        }))
        .unwrap();
        assert!(f(big).matches(&r));
        assert!(!f(next).matches(&r));
    }

    #[test]
    fn r7_malformed_filters_are_refused() {
        for f in [
            json!([{"in": ["x"]}]),
            json!([{"fieldId": STATUS}]),
            json!([{"fieldId": STATUS, "in": ["draft"], "missing": true}]),
            json!([{"fieldId": STATUS, "in": []}]),
            json!([{"fieldId": STATUS, "in": [""]}]),
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
            json!([{"fieldId": MINUTES, "in": [2.5]}]),
            json!([{"fieldId": STATUS, "in": [null]}]),
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

    fn vocab(
        id: &str,
        extends: Option<&str>,
        terms: serde_json::Value,
    ) -> srs_core::types::vocabulary::Vocabulary {
        let mut v = json!({"id": id, "version": 1, "namespace": "t", "name": id, "mode": "closed",
            "createdAt": "2026-01-01T00:00:00Z", "terms": terms});
        if let Some(p) = extends {
            v["extendsVocabularyId"] = json!(p);
            v["extendsVocabularyVersion"] = json!(1);
        }
        serde_json::from_value(v).unwrap()
    }

    fn term(id: &str, key: &str, aliases: &[&str]) -> serde_json::Value {
        json!({"id": id, "version": 1, "namespace": "t", "key": key, "aliases": aliases, "createdAt": "2026-01-01T00:00:00Z"})
    }

    #[test]
    fn empty_object_is_a_value_except_on_map_or_inline_ref() {
        // On a string Field, {} is a value that does not fit: missing:false holds, in never does.
        assert!(holds(
            json!({"fieldId": STATUS, "missing": false}),
            json!({"status": {}})
        ));
        assert!(!holds(
            json!({"fieldId": STATUS, "missing": true}),
            json!({"status": {}})
        ));
        assert!(!holds(
            json!({"fieldId": STATUS, "notIn": ["draft"]}),
            json!({"status": {}})
        ));
    }

    #[test]
    fn open_field_with_unresolvable_vocabulary_warns_and_compares_as_written() {
        let mut p = package();
        p.fields.push(field(
            "aa000000-0000-4000-a000-0000000000f6",
            "label",
            json!({"datatype": "string", "vocabularyRef": "aa000000-0000-4000-a000-0000000000c9"}),
        ));
        p.record_types[0].fields.push(serde_json::from_value(json!({"fieldId": "aa000000-0000-4000-a000-0000000000f6", "order": 6, "required": false})).unwrap());
        let c = compile(
            &p,
            &filters(json!([{"fieldId": "aa000000-0000-4000-a000-0000000000f6", "in": ["x"]}])),
        )
        .unwrap();
        assert_eq!(c.warnings.len(), 1);
        assert!(c.matches(&record(json!({"label": "x"}))));
    }

    #[test]
    fn vocabulary_extends_and_alias_tie_break() {
        let mut p = package();
        let parent = "aa000000-0000-4000-a000-0000000000c2";
        p.vocabularies = vec![
            vocab(
                parent,
                None,
                json!([term("aa000000-0000-4000-a000-0000000000e1", "memo", &[])]),
            ),
            vocab(
                VOCAB,
                Some(parent),
                json!([
                    // Two terms share the alias "note": the lower id ("email") wins (RFC-006 V2).
                    term("aa000000-0000-4000-a000-0000000000d2", "letter", &["note"]),
                    term("aa000000-0000-4000-a000-0000000000d1", "email", &["note"]),
                ]),
            ),
        ];
        // A parent term is in the effective entry set, so it is not a misfit and matches.
        let c = compile(&p, &filters(json!([{"fieldId": KIND, "in": ["memo"]}]))).unwrap();
        assert!(c.matches(&record(json!({"kind": "memo"}))));
        let c = compile(&p, &filters(json!([{"fieldId": KIND, "in": ["note"]}]))).unwrap();
        assert!(c.matches(&record(json!({"kind": "email"}))));
        assert!(!c.matches(&record(json!({"kind": "letter"}))));
        // Validation reads the same set: keys and aliases, parent included.
        use srs_core::validation::value_shape::RangeResolver;
        let keys = p.vocabulary_term_keys(VOCAB).unwrap();
        for k in ["letter", "email", "note", "memo"] {
            assert!(keys.iter().any(|x| x == k), "{k} missing from {keys:?}");
        }
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
