//! The repository guidance appended to the MCP `initialize` instructions (srs-rust#1668).
//!
//! The selection rule (which registry members contribute), the combined byte budget and the
//! closing "N more repositories" line are shared by every MCP adapter (ADR-037, srs-rust#1677),
//! so they live here once. This module touches no store and no path: the caller opens
//! repositories and passes each member's orientation note in.

use srs_repository::agent_index_service::ORIENTATION_NOTE_MAX_BYTES;

/// Hard combined cap (bytes) on the repository guidance appended to the instructions
/// (srs-rust#1668), the closing "more repositories" line included.
pub const REGISTRY_GUIDANCE_MAX_BYTES: usize = 4 * ORIENTATION_NOTE_MAX_BYTES;
/// Room kept inside the cap for that closing line.
pub const MORE_LINE_MAX_BYTES: usize = 160;

/// One member of a registry, as far as the selection rule needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuidanceMember<'a> {
    pub id: &'a str,
    pub read_only: bool,
}

impl GuidanceMember<'_> {
    /// Only the default and the writable members contribute: a read-only member is reference
    /// material, not the session's workspace.
    pub fn contributes(&self, default_id: &str) -> bool {
        !self.read_only || self.id.eq_ignore_ascii_case(default_id)
    }
}

/// The guidance text for a registry: the notes of the contributing members, in order, stopped
/// at [`REGISTRY_GUIDANCE_MAX_BYTES`], the members past it counted in one closing line. Empty
/// when no contributing member has a note. `note_for` is called only for contributing members
/// (it typically opens the member's store), in member order.
pub fn registry_guidance<'a>(
    members: impl IntoIterator<Item = GuidanceMember<'a>>,
    default_id: &str,
    mut note_for: impl FnMut(&str) -> Option<String>,
) -> String {
    compose(
        members
            .into_iter()
            .filter(|m| m.contributes(default_id))
            .map(|m| note_for(m.id)),
    )
}

/// The guidance text for a single served repository: its note, under the same budget.
pub fn single_repository_guidance(note: Option<String>) -> String {
    compose(std::iter::once(note))
}

fn compose(notes: impl Iterator<Item = Option<String>>) -> String {
    let mut out = String::new();
    let mut more = 0;
    for note in notes.flatten() {
        if more > 0
            || out.len() + 2 + note.len() > REGISTRY_GUIDANCE_MAX_BYTES - MORE_LINE_MAX_BYTES
        {
            more += 1;
            continue;
        }
        out.push_str("\n\n");
        out.push_str(&note);
    }
    if more > 0 {
        out.push_str(&format!(
            "\n\n{more} more repositories in this registry have guidance that does not fit \
             here: read srs://<repositoryId>/agent-index for each."
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, read_only: bool) -> GuidanceMember<'_> {
        GuidanceMember { id, read_only }
    }

    #[test]
    fn read_only_members_contribute_only_as_default() {
        let mut asked = vec![];
        let text = registry_guidance([m("a", true), m("b", false), m("c", true)], "C", |id| {
            asked.push(id.to_string());
            Some(format!("note {id}"))
        });
        assert_eq!(asked, ["b", "c"]);
        assert_eq!(text, "\n\nnote b\n\nnote c");
    }

    #[test]
    fn no_notes_is_empty() {
        assert_eq!(registry_guidance([m("a", false)], "a", |_| None), "");
        assert_eq!(single_repository_guidance(None), "");
    }

    #[test]
    fn single_repository_is_the_note_after_a_blank_line() {
        assert_eq!(single_repository_guidance(Some("n".into())), "\n\nn");
    }

    #[test]
    fn total_is_capped_and_the_rest_counted() {
        let note = "x".repeat(ORIENTATION_NOTE_MAX_BYTES);
        let ids: Vec<String> = (0..8).map(|i| format!("r{i}")).collect();
        let text = registry_guidance(ids.iter().map(|id| m(id, false)), "r0", |_| {
            Some(note.clone())
        });
        assert!(text.len() <= REGISTRY_GUIDANCE_MAX_BYTES, "{}", text.len());
        assert_eq!(
            text.matches("more repositories in this registry").count(),
            1
        );
        assert!(text.ends_with("for each."));
        assert!(text.contains("\n\n5 more repositories"), "{text}");
    }
}
