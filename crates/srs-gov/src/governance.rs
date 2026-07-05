//! Governance container type registry.
//!
//! Maps the friendly CLI key (used in `srs-gov <key> list|get|create`) to the
//! SRS type that roots each governance section, plus the child record type(s)
//! that can be created inside each container.  This is the single piece of
//! governance config in srs-gov — the stand-in for the nav taxonomy tracked in
//! the-greenman/srs#92.
//!
//! ## Navigation join key (RFC-009)
//!
//! `root_type_namespace` + `root_type_name` identify a governance section via the
//! UUID type chain returned by `srs repo navigation`.  This replaces the former
//! `containerType` string hint (soft-deprecated by RFC-009).  The `container_type`
//! field is retained only for the TUI path (`tui_data.rs`) which has not yet
//! migrated; it will be removed when the TUI migrates (epic #262).

pub struct ContainerTypeDef {
    /// CLI key used to address this container (e.g. "decision_log")
    pub key: &'static str,
    /// Namespace of the SRS type that roots this section (navigation join key)
    pub root_type_namespace: &'static str,
    /// Name of the SRS type that roots this section (navigation join key)
    pub root_type_name: &'static str,
    /// `containerType` hint value — retained for TUI compat only (see module doc)
    pub container_type: &'static str,
    /// Human display name
    pub label: &'static str,
    /// Icon glyph for list output
    pub icon: &'static str,
    /// Child types creatable inside this container: (cli-name, namespace/name)
    pub creatable: &'static [(&'static str, &'static str)],
}

// Release 1 is a decision-log-only template. The `article` and `role` types remain
// defined (dormant) in the com.mudemocracy.governance package, but a freshly-created
// governance document scaffolds only the Decision Log. Re-adding Articles/Roles is a
// future package-upgrade concern (muDemocracy.org#37), not a release-1 container.
pub static GOVERNANCE_CONTAINERS: &[ContainerTypeDef] = &[ContainerTypeDef {
    key: "decision_log",
    root_type_namespace: "governance",
    root_type_name: "decision_log",
    container_type: "decision_log",
    label: "Decision Log",
    icon: "⊕",
    creatable: &[("decision", "governance/decision")],
}];

/// Look up a container type def by CLI key.
pub fn by_key(key: &str) -> Option<&'static ContainerTypeDef> {
    GOVERNANCE_CONTAINERS.iter().find(|d| d.key == key)
}

/// Look up a container type def by the SRS type that roots the section.
///
/// Used by `cmd_top` and `resolve_container_id` to match navigation nodes
/// by their `typeNamespace`/`typeName` instead of the deprecated `containerType` hint.
pub fn by_root_type(namespace: &str, name: &str) -> Option<&'static ContainerTypeDef> {
    GOVERNANCE_CONTAINERS
        .iter()
        .find(|d| d.root_type_namespace == namespace && d.root_type_name == name)
}

/// Attempt to match a container list entry (from srs JSON) to a known governance def.
/// Returns `None` for containers whose containerType is not in the allowlist, or for
/// containers that share a type with an already-matched entry (e.g. two "document" containers
/// need disambiguation by which key hasn't been matched yet).
pub fn match_container(
    container_type: Option<&str>,
    title: &str,
    used_keys: &mut std::collections::HashSet<&'static str>,
) -> Option<&'static ContainerTypeDef> {
    let ct = container_type?;
    let exact_title_match = GOVERNANCE_CONTAINERS.iter().find(|d| {
        d.container_type == ct && !used_keys.contains(d.key) && title.eq_ignore_ascii_case(d.label)
    });

    exact_title_match
        .or_else(|| {
            GOVERNANCE_CONTAINERS
                .iter()
                .find(|d| d.container_type == ct && !used_keys.contains(d.key))
        })
        .inspect(|d| {
            used_keys.insert(d.key);
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn by_root_type_finds_decision_log() {
        let def = by_root_type("governance", "decision_log").unwrap();
        assert_eq!(def.key, "decision_log");
    }

    #[test]
    fn by_root_type_returns_none_for_unknown() {
        assert!(by_root_type("governance", "unknown_type").is_none());
        assert!(by_root_type("other_ns", "decision_log").is_none());
    }

    #[test]
    fn decision_log_container_matches_by_type() {
        let mut used = HashSet::new();

        let dl = match_container(Some("decision_log"), "Decision Log", &mut used).unwrap();
        assert_eq!(dl.key, "decision_log");

        // The def is single-use: a second decision_log container has nothing left to match.
        assert!(match_container(Some("decision_log"), "Other Log", &mut used).is_none());

        // Article/role container types are no longer in the registry (dormant in the package).
        assert!(match_container(Some("document"), "Articles", &mut HashSet::new()).is_none());
    }
}
