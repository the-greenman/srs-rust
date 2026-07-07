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
//! UUID type chain returned by `srs repo navigation`.  The RFC-009 migration to
//! `typeNamespace`/`typeName` matching is now complete across all srs-gov paths.

pub struct ContainerTypeDef {
    /// CLI key used to address this container (e.g. "decision_log")
    pub key: &'static str,
    /// Namespace of the SRS type that roots this section (navigation join key)
    pub root_type_namespace: &'static str,
    /// Name of the SRS type that roots this section (navigation join key)
    pub root_type_name: &'static str,
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
/// Used by `cmd_top`, `resolve_container_id`, and `sections_from_navigation` to match
/// navigation nodes by their `typeNamespace`/`typeName`.
pub fn by_root_type(namespace: &str, name: &str) -> Option<&'static ContainerTypeDef> {
    GOVERNANCE_CONTAINERS
        .iter()
        .find(|d| d.root_type_namespace == namespace && d.root_type_name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
