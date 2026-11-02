//! The boundary an implementation resolves within (RFC-049 [R8]) and the ordered
//! refusals for reads ([R5]) and routed writes ([R16], [R17]).
//!
//! Without a registry the boundary is the one repository served. With a registry
//! it is the `repository` entries that loaded; each request names its repository by
//! id (absent = the default, [R13]) and resolves to that repository's root path.
//! Callers then open the repository through the ordinary per-repository services.

use std::path::{Path, PathBuf};

use srs_core::extensions::registry::{Locator, RegistryEntry};

use crate::error::RepositoryError;
use crate::field_type_migration_service::CURRENT_DATA_MODEL_REVISION;

/// Whether a request reads or writes ([R17]'s `read-only` applies to writes only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// One boundary member: a `repository` entry that loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub id: String,
    pub label: Option<String>,
    pub locator: Locator,
    pub read_only: bool,
}

/// A request resolved to one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub id: String,
    pub path: PathBuf,
}

/// The repositories an implementation may resolve into ([R8]).
#[derive(Debug, Clone)]
pub struct Boundary {
    members: Vec<Member>,
    /// Relative `local` locators resolve against this directory (the registry file's).
    base: PathBuf,
    /// The default repository id ([R13]); for a single repository, that repository.
    default_id: String,
    /// Whether the default is a member that loaded ([R13]: report it, refuse default requests).
    default_loaded: bool,
    /// Whether members came from a registry. A single served repository is opened as
    /// today: its identity and revision are the store's to check, not the boundary's.
    from_registry: bool,
}

impl Boundary {
    /// The boundary of an implementation using no registry: exactly the one repository
    /// it serves ([R8]), read from its manifest.
    pub fn single(repo_path: &Path) -> Result<Self, RepositoryError> {
        let (id, _) = read_identity(repo_path)?;
        Ok(Self {
            members: vec![Member {
                id: id.clone(),
                label: None,
                locator: Locator::Local {
                    path: repo_path.to_string_lossy().into_owned(),
                },
                read_only: false,
            }],
            base: PathBuf::new(),
            default_id: id,
            default_loaded: true,
            from_registry: false,
        })
    }

    /// Load a registry file as a boundary. `default_id` names the default repository;
    /// absent, the first `repository` entry in the document is the default. Entries
    /// that fail to load are left out ([R15]); the caller reports `outcomes`.
    pub fn from_registry(
        registry_path: &Path,
        default_id: Option<&str>,
    ) -> Result<(Self, Vec<srs_core::extensions::registry::EntryOutcome>), RepositoryError> {
        let loaded = crate::registry_service::read_registry_file(registry_path)?;
        let members: Vec<Member> = loaded
            .entries
            .iter()
            .filter_map(|e| match e {
                RegistryEntry::Repository(r) => Some(Member {
                    id: r.id.clone(),
                    label: r.label.clone(),
                    locator: r.locator.clone(),
                    read_only: r.read_only,
                }),
                RegistryEntry::Package(_) => None,
            })
            .collect();
        // The default is named by the caller, else the first repository entry in the
        // document, whether or not it loaded ([R13]).
        let default_id = match default_id {
            Some(id) => id.to_string(),
            None => loaded
                .registry
                .entries
                .iter()
                .find(|e| e.get("kind").and_then(|k| k.as_str()) == Some("repository"))
                .and_then(|e| e.get("id").and_then(|v| v.as_str()))
                .unwrap_or_default()
                .to_string(),
        };
        let default_loaded = members
            .iter()
            .any(|m| m.id.eq_ignore_ascii_case(&default_id));
        let base = registry_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        Ok((
            Self {
                members,
                base,
                default_id,
                default_loaded,
                from_registry: true,
            },
            loaded.outcomes,
        ))
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    pub fn default_id(&self) -> &str {
        &self.default_id
    }

    /// Whether the default repository's entry loaded ([R13]).
    pub fn default_loaded(&self) -> bool {
        self.default_loaded
    }

    /// Resolve a request naming `repository_id` (absent = the default) for `access`,
    /// refusing with the first outcome that holds, in [R17]'s order:
    /// `not-in-boundary`, `read-only` (writes; decided from the entry, so the target
    /// is never reached), `locator-unusable`, `id-mismatch`, `revision-unsupported`.
    pub fn resolve(
        &self,
        repository_id: Option<&str>,
        access: Access,
    ) -> Result<Resolved, RepositoryError> {
        let id = repository_id.unwrap_or(&self.default_id);
        let member = self
            .members
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(id))
            .ok_or_else(|| RepositoryError::NotInBoundary {
                repository_id: id.to_string(),
            })?;
        if access == Access::Write && member.read_only {
            return Err(refused(
                "read-only",
                format!(
                    "repository '{}' is read-only in this server's registry",
                    member.id
                ),
            ));
        }
        if !self.from_registry {
            let Locator::Local { path } = &member.locator else {
                unreachable!("a single served repository has a local path")
            };
            return Ok(Resolved {
                id: member.id.clone(),
                path: PathBuf::from(path),
            });
        }
        let path = match &member.locator {
            Locator::Local { path } => self.base.join(path),
            // ponytail: no remote transport yet; add one when a remote entry is served.
            Locator::Remote { url } => {
                return Err(refused(
                    "locator-unusable",
                    format!(
                        "repository '{}': remote locator '{url}' is not supported",
                        member.id
                    ),
                ))
            }
        };
        let (found, revision) = read_identity(&path).map_err(|_| {
            refused(
                "locator-unusable",
                format!(
                    "repository '{}': locator '{}' does not lead to a repository",
                    member.id,
                    path.display()
                ),
            )
        })?;
        if !found.eq_ignore_ascii_case(&member.id) {
            return Err(refused(
                "id-mismatch",
                format!(
                    "registry entry '{}' locates a repository that reports id '{found}'",
                    member.id
                ),
            ));
        }
        if revision != CURRENT_DATA_MODEL_REVISION {
            return Err(refused(
                "revision-unsupported",
                format!(
                    "repository '{}' is at dataModelRevision {revision}; this build reads {CURRENT_DATA_MODEL_REVISION}",
                    member.id
                ),
            ));
        }
        Ok(Resolved {
            id: member.id.clone(),
            path,
        })
    }
}

fn refused(code: &'static str, message: String) -> RepositoryError {
    RepositoryError::BoundaryRefused { code, message }
}

/// The repository's own `manifest.repositoryId` and `dataModelRevision` (absent = 0).
fn read_identity(repo_path: &Path) -> Result<(String, u64), RepositoryError> {
    let path = repo_path.join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|_| RepositoryError::NotFound { path: path.clone() })?;
    let raw: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| RepositoryError::RegistryMalformed {
            path: Some(path.clone()),
            message: e.to_string(),
        })?;
    let id = raw
        .get("repositoryId")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let revision = raw
        .get("dataModelRevision")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Ok((id, revision))
}
