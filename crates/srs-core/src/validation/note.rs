use crate::error::CoreError;
use crate::types::note::Note;
use std::collections::HashSet;

pub fn validate_note(note: &Note) -> Result<(), CoreError> {
    // Check for duplicate section names
    let mut seen_names = HashSet::new();
    for section in &note.sections {
        if !seen_names.insert(&section.name) {
            return Err(CoreError::DuplicateSectionName {
                name: section.name.clone(),
            });
        }
    }

    // Check for empty tags on note
    if let Some(ref tags) = note.tags {
        for tag in tags {
            if tag.is_empty() {
                return Err(CoreError::EmptyTag);
            }
        }
    }

    // Check for empty tags on sections
    for section in &note.sections {
        if let Some(ref tags) = section.tags {
            for tag in tags {
                if tag.is_empty() {
                    return Err(CoreError::EmptyTag);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::note::{Note, NoteSection};

    fn minimal_note() -> Note {
        Note {
            instance_id: "test-id".to_string(),
            title: None,
            tags: None,
            sections: vec![NoteSection {
                name: "section1".to_string(),
                label: None,
                content: "content".to_string(),
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        }
    }

    #[test]
    fn valid_note_passes() {
        let note = minimal_note();
        assert!(validate_note(&note).is_ok());
    }

    #[test]
    fn duplicate_section_name_fails() {
        let note = Note {
            sections: vec![
                NoteSection {
                    name: "problem".to_string(),
                    label: None,
                    content: "content1".to_string(),
                    content_hint: None,
                    tags: None,
                },
                NoteSection {
                    name: "problem".to_string(),
                    label: None,
                    content: "content2".to_string(),
                    content_hint: None,
                    tags: None,
                },
            ],
            ..minimal_note()
        };
        let result = validate_note(&note);
        assert_eq!(
            result,
            Err(CoreError::DuplicateSectionName {
                name: "problem".to_string()
            })
        );
    }

    #[test]
    fn empty_tag_on_note_fails() {
        let note = Note {
            tags: Some(vec!["valid".to_string(), "".to_string()]),
            ..minimal_note()
        };
        let result = validate_note(&note);
        assert_eq!(result, Err(CoreError::EmptyTag));
    }

    #[test]
    fn empty_tag_on_section_fails() {
        let note = Note {
            sections: vec![NoteSection {
                name: "section1".to_string(),
                label: None,
                content: "content".to_string(),
                content_hint: None,
                tags: Some(vec!["".to_string()]),
            }],
            ..minimal_note()
        };
        let result = validate_note(&note);
        assert_eq!(result, Err(CoreError::EmptyTag));
    }
}
