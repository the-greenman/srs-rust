use crate::error::CoreError;
use crate::types::container::Container;

pub fn validate_container(container: &Container) -> Result<(), CoreError> {
    if container.container_id.is_empty() {
        return Err(CoreError::MissingRequiredField {
            field_id: "containerId".to_string(),
        });
    }
    uuid::Uuid::parse_str(&container.container_id).map_err(|_| CoreError::InvalidFieldValue {
        field_id: "containerId".to_string(),
        reason: "must be a valid UUID".to_string(),
    })?;
    if container.title.is_empty() {
        return Err(CoreError::MissingRequiredField {
            field_id: "title".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn minimal() -> Container {
        Container {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn validate_container_passes_minimal() {
        assert!(validate_container(&minimal()).is_ok());
    }

    #[test]
    fn validate_container_empty_container_id_fails() {
        let mut c = minimal();
        c.container_id.clear();
        assert_eq!(
            validate_container(&c),
            Err(CoreError::MissingRequiredField {
                field_id: "containerId".to_string()
            })
        );
    }

    #[test]
    fn validate_container_non_uuid_container_id_fails() {
        let mut c = minimal();
        c.container_id = "not-a-uuid".to_string();
        assert_eq!(
            validate_container(&c),
            Err(CoreError::InvalidFieldValue {
                field_id: "containerId".to_string(),
                reason: "must be a valid UUID".to_string()
            })
        );
    }

    #[test]
    fn validate_container_empty_title_fails() {
        let mut c = minimal();
        c.title.clear();
        assert_eq!(
            validate_container(&c),
            Err(CoreError::MissingRequiredField {
                field_id: "title".to_string()
            })
        );
    }
}
