//! Governance container type registry.
//!
//! Maps the friendly CLI key (used in `srs-gov <key> list|get|create`) to the
//! containerType stored in the repo, plus the child record type(s) that can be
//! created inside each container.  This is the single piece of governance config
//! in srs-gov — the stand-in for the nav taxonomy tracked in the-greenman/srs#92.

pub struct ContainerTypeDef {
    /// CLI key used to address this container (e.g. "decision_log")
    pub key: &'static str,
    /// `containerType` value in the SRS data
    pub container_type: &'static str,
    /// Human display name
    pub label: &'static str,
    /// Icon glyph for list output
    pub icon: &'static str,
    /// Child types creatable inside this container: (cli-name, namespace/name)
    pub creatable: &'static [(&'static str, &'static str)],
}

pub static GOVERNANCE_CONTAINERS: &[ContainerTypeDef] = &[
    ContainerTypeDef {
        key: "articles",
        container_type: "document",
        label: "Articles",
        icon: "§",
        creatable: &[("article", "governance/article")],
    },
    ContainerTypeDef {
        key: "decision_log",
        container_type: "decision_log",
        label: "Decision Log",
        icon: "⊕",
        creatable: &[("decision", "governance/decision")],
    },
    ContainerTypeDef {
        key: "roles",
        container_type: "document",
        label: "Roles",
        icon: "◈",
        creatable: &[("role", "governance/role")],
    },
];

/// Look up a container type def by CLI key.
pub fn by_key(key: &str) -> Option<&'static ContainerTypeDef> {
    GOVERNANCE_CONTAINERS.iter().find(|d| d.key == key)
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
    fn document_container_matching_disambiguates_by_title() {
        let mut used = HashSet::new();

        let roles = match_container(Some("document"), "Roles", &mut used).unwrap();
        let articles = match_container(Some("document"), "Articles", &mut used).unwrap();

        assert_eq!(roles.key, "roles");
        assert_eq!(articles.key, "articles");
    }
}
