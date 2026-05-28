use crate::error::CoreError;
use crate::types::tag_definition::TagDefinition;

/// Validates a TagDefinition.
///
/// Checks:
/// - `tag_key` is non-empty → `Err(CoreError::EmptyTagKey)`
/// - All strings in `roles` and `aliases` are non-empty → `Err(CoreError::EmptyTag)`
pub fn validate_tag_definition(td: &TagDefinition) -> Result<(), CoreError> {
    // Check tag_key is non-empty
    if td.tag_key.is_empty() {
        return Err(CoreError::EmptyTagKey);
    }

    // Check all roles are non-empty
    if let Some(roles) = &td.roles {
        for role in roles {
            if role.is_empty() {
                return Err(CoreError::EmptyTag);
            }
        }
    }

    // Check all aliases are non-empty
    if let Some(aliases) = &td.aliases {
        for alias in aliases {
            if alias.is_empty() {
                return Err(CoreError::EmptyTag);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_td(tag_key: &str) -> TagDefinition {
        TagDefinition {
            instance_id: "test-id".to_string(),
            tag_key: tag_key.to_string(),
            label: None,
            description: None,
            roles: None,
            aliases: None,
            status: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn validate_tag_definition_passes_minimal() {
        let td = create_test_td("foundation");
        assert!(validate_tag_definition(&td).is_ok());
    }

    #[test]
    fn validate_tag_definition_empty_key_fails() {
        let td = create_test_td("");
        let result = validate_tag_definition(&td);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::EmptyTagKey));
    }

    #[test]
    fn validate_tag_definition_empty_role_fails() {
        let td = TagDefinition {
            roles: Some(vec!["foundation".to_string(), "".to_string()]),
            ..create_test_td("test")
        };
        let result = validate_tag_definition(&td);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::EmptyTag));
    }

    #[test]
    fn validate_tag_definition_empty_alias_fails() {
        let td = TagDefinition {
            aliases: Some(vec!["alias1".to_string(), "".to_string()]),
            ..create_test_td("test")
        };
        let result = validate_tag_definition(&td);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::EmptyTag));
    }

    #[test]
    fn validate_tag_definition_valid_roles_ok() {
        let td = TagDefinition {
            roles: Some(vec!["foundation".to_string(), "navigation".to_string()]),
            ..create_test_td("purpose")
        };
        assert!(validate_tag_definition(&td).is_ok());
    }

    #[test]
    fn validate_tag_definition_valid_aliases_ok() {
        let td = TagDefinition {
            aliases: Some(vec!["core".to_string(), "primary".to_string()]),
            ..create_test_td("foundation")
        };
        assert!(validate_tag_definition(&td).is_ok());
    }
}
