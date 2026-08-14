use crate::error::CoreError;
use crate::types::container::Container;

pub fn validate_container(container: &Container) -> Result<(), CoreError> {
    if container.container_id.is_empty() {
        return Err(CoreError::MissingRequiredField {
            key: "containerId".to_string(),
        });
    }
    uuid::Uuid::parse_str(&container.container_id).map_err(|_| CoreError::InvalidFieldValue {
        key: "containerId".to_string(),
        reason: "must be a valid UUID".to_string(),
    })?;
    if container.title.is_empty() {
        return Err(CoreError::MissingRequiredField {
            key: "title".to_string(),
        });
    }
    // A blank membership/identity id can never resolve to an instance, so it is a
    // guaranteed fatal [R13] dangling reference at the next catalog build — i.e. a
    // write that would make the repository unloadable. Reject it here, the one place
    // `create_container` and `update_container` both route through (srs-rust#841).
    reject_blank_ids(
        "identityInstanceId",
        container.identity_instance_id.iter().map(String::as_str),
    )?;
    reject_blank_ids(
        "rootInstanceIds",
        container
            .root_instance_ids
            .iter()
            .flatten()
            .map(String::as_str),
    )?;
    reject_blank_ids(
        "memberInstanceIds",
        container
            .member_instance_ids
            .iter()
            .flatten()
            .map(String::as_str),
    )?;
    Ok(())
}

fn reject_blank_ids<'a>(
    key: &str,
    mut ids: impl Iterator<Item = &'a str>,
) -> Result<(), CoreError> {
    if ids.any(|id| id.trim().is_empty()) {
        return Err(CoreError::InvalidFieldValue {
            key: key.to_string(),
            reason: "instance id must not be blank".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn minimal() -> Container {
        Container {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: BTreeMap::new(),
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
                key: "containerId".to_string()
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
                key: "containerId".to_string(),
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
                key: "title".to_string()
            })
        );
    }

    fn blank(key: &str) -> Result<(), CoreError> {
        Err(CoreError::InvalidFieldValue {
            key: key.to_string(),
            reason: "instance id must not be blank".to_string(),
        })
    }

    #[test]
    fn validate_container_blank_root_instance_id_fails() {
        let mut c = minimal();
        c.root_instance_ids = Some(vec![String::new()]);
        assert_eq!(validate_container(&c), blank("rootInstanceIds"));
    }

    #[test]
    fn validate_container_whitespace_member_instance_id_fails() {
        let mut c = minimal();
        c.member_instance_ids = Some(vec!["   ".to_string()]);
        assert_eq!(validate_container(&c), blank("memberInstanceIds"));
    }

    #[test]
    fn validate_container_blank_identity_instance_id_fails() {
        let mut c = minimal();
        c.identity_instance_id = Some(String::new());
        assert_eq!(validate_container(&c), blank("identityInstanceId"));
    }

    #[test]
    fn validate_container_passes_with_populated_membership() {
        let mut c = minimal();
        let id = "6ba7b810-9dad-11d1-80b4-00c04fd430c8".to_string();
        c.identity_instance_id = Some(id.clone());
        c.root_instance_ids = Some(vec![id.clone()]);
        c.member_instance_ids = Some(vec![id]);
        assert!(validate_container(&c).is_ok());
    }
}
