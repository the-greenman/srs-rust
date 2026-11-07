//! # Bundled readme (RFC-045 [R1], [R2], [R4], [R5]; srs-rust#1164)
//!
//! A package bundle (`.srspkg`) and a `.srsj` envelope may carry one optional
//! `readme: {path, content}`: the file's actual path within the artifact plus
//! its UTF-8 text. The content is opaque ([R1]): nothing here reads it beyond
//! the content rule. The path rules ([R4]) are enforced here, not by the
//! schema. An invalid readme is the error `readme-invalid`; it is neither
//! written nor carried, and the rest of the load proceeds.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The diagnostic code for an invalid readme (RFC-045 [R4]).
pub const README_INVALID: &str = "readme-invalid";

/// `readme: {path, content}` exactly as carried (RFC-045 [R2]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundledReadme {
    pub path: String,
    pub content: String,
}

/// The diagnostic code for a readme that could not be written (RFC-045 [R6]).
pub const README_WRITE_CONFLICT: &str = "readme-write-conflict";

/// Write a carried readme at `{dir}/{path}` (RFC-045 [R3], [R6]).
///
/// Call it after every other carried file of the operation. It never writes
/// over an existing entry, nor through a symbolic link, nor where a parent is
/// not a directory: it leaves what is there and returns the
/// `readme-write-conflict` message. An existing regular file holding exactly
/// the UTF-8 bytes of `content` is not a conflict and is left untouched.
/// Stores that are not file trees (memory sessions) write nothing here.
pub fn write_readme(
    store: &dyn crate::store::RepositoryStore,
    dir: &str,
    readme: &BundledReadme,
) -> Option<String> {
    if !store.is_file_tree_store() {
        return None;
    }
    let target = if dir.is_empty() {
        readme.path.clone()
    } else {
        format!("{dir}/{}", readme.path)
    };
    let conflict = |why: &str| {
        Some(format!(
            "{README_WRITE_CONFLICT}: readme not written at `{target}`: {why}"
        ))
    };
    if store.path_has_symlink(&target) {
        return conflict("a component of the path is a symbolic link");
    }
    match store.load_binary_file(&target) {
        Ok(existing) if existing == readme.content.as_bytes() => None,
        Ok(_) => conflict("a file with different content is already there"),
        Err(e) if e.is_not_found() => match store.save_text_file(&target, &readme.content) {
            Ok(()) => None,
            Err(e) => conflict(&format!("it cannot be written ({e})")),
        },
        Err(_) => conflict("the path is not a regular file"),
    }
}

/// Pick up the readme file at `{dir}/{path}` for a pack or export (RFC-045
/// tooling convention): a regular file (never through a symbolic link) whose
/// bytes are valid UTF-8, carried unchanged; nothing is synthesised. Anything
/// else (absent, a directory, a link, non-UTF-8) yields `None`.
pub fn pick_up_readme(
    store: &dyn crate::store::RepositoryStore,
    dir: &str,
    path: &str,
) -> Option<BundledReadme> {
    let target = if dir.is_empty() {
        path.to_string()
    } else {
        format!("{dir}/{path}")
    };
    if store.path_has_symlink(&target) {
        return None;
    }
    let bytes = store.load_binary_file(&target).ok()?;
    let content = String::from_utf8(bytes).ok()?;
    Some(BundledReadme {
        path: path.to_string(),
        content,
    })
}

/// What the path is checked against ([R4]).
pub enum ReadmeForm<'a> {
    /// A package bundle: relative to the package directory.
    Bundle,
    /// A `.srsj`: relative to the repository root; the envelope's `data` keys
    /// and manifest decide the reserved locations.
    Srsj {
        data_keys: &'a [String],
        manifest: &'a Value,
    },
}

/// The result of reading the optional `readme` property of a form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadmeRead {
    /// The valid readme, to be kept unchanged ([R5]).
    pub readme: Option<BundledReadme>,
    /// The `readme-invalid` error message when it was dropped ([R4]).
    pub invalid: Option<String>,
}

/// Read the `readme` property of a travelling form. Absent is never a finding.
pub fn read_readme(value: Option<&Value>, form: &ReadmeForm<'_>, label: &str) -> ReadmeRead {
    let Some(value) = value else {
        return ReadmeRead::default();
    };
    let bad = |rule: String| ReadmeRead {
        readme: None,
        invalid: Some(format!("{README_INVALID}: {label} readme dropped: {rule}")),
    };
    let readme: BundledReadme = match serde_json::from_value(value.clone()) {
        Ok(r) => r,
        Err(e) => return bad(format!("`readme` must be {{path, content}} strings ({e})")),
    };
    match check_path(&readme.path, form) {
        Ok(()) => ReadmeRead {
            readme: Some(readme),
            invalid: None,
        },
        Err(rule) => bad(format!("path `{}`: {rule}", readme.path)),
    }
}

const RESERVED: [&str; 5] = [
    "records",
    "notes",
    "typed-records",
    "relations",
    "containers",
];

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// `a` equals `b` or lies inside it, ASCII-case-insensitively.
fn at_or_inside(path: &str, loc: &str) -> bool {
    let (p, l) = (lower(path), lower(loc));
    p == l || p.starts_with(&format!("{l}/"))
}

fn declared(manifest: &Value, key: &str) -> Option<String> {
    let s = manifest.get(key)?.as_str()?;
    let s = s.strip_prefix("./").unwrap_or(s).trim_end_matches('/');
    (!s.is_empty()).then(|| s.to_string())
}

/// The [R4] path rules. `Err` names the rule that failed.
pub fn check_path(path: &str, form: &ReadmeForm<'_>) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    if path.starts_with('/') {
        return Err("absolute path".into());
    }
    if let Some(c) = path
        .chars()
        .find(|c| matches!(c, '\\' | ':') || ('\u{0}'..='\u{1f}').contains(c) || *c == '\u{7f}')
    {
        return Err(format!("forbidden character {c:?}"));
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(format!("empty, `.` or `..` segment in `{path}`"));
        }
        if seg.ends_with('.') || seg.ends_with(' ') {
            return Err(format!("segment `{seg}` ends in `.` or a space"));
        }
    }
    match form {
        ReadmeForm::Bundle => {
            if lower(path) == "package.json" {
                return Err("a bundle readme cannot be package.json".into());
            }
        }
        ReadmeForm::Srsj {
            data_keys,
            manifest,
        } => {
            if lower(path) == "manifest.json" || lower(path) == ".srs" {
                return Err("reserved path of the envelope".into());
            }
            for k in data_keys.iter() {
                let (p, k) = (path, k.as_str());
                if p == k || at_or_inside(p, k) {
                    return Err(format!("equals or lies inside data key `{k}`"));
                }
                if k.starts_with(&format!("{p}/")) {
                    return Err(format!("is an ancestor of data key `{k}`"));
                }
            }
            let mut reserved: Vec<String> = RESERVED.iter().map(|s| s.to_string()).collect();
            reserved.push(
                declared(manifest, "sourceDocumentsPath").unwrap_or("source-documents".into()),
            );
            reserved.extend(declared(manifest, "changelogPath"));
            for k in data_keys.iter() {
                if k == "package.json" {
                    reserved.push(String::new());
                } else if let Some(dir) = k.strip_suffix("/package.json") {
                    reserved.push(dir.to_string());
                }
            }
            for loc in reserved {
                if loc.is_empty() {
                    // The repository root is a local package root: every path lies in it.
                    return Err(
                        "lies inside a local package root (`package.json` is a data key)".into(),
                    );
                }
                if at_or_inside(path, &loc) {
                    return Err(format!(
                        "equals or lies inside the reserved location `{loc}`"
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundle(p: &str) -> Result<(), String> {
        check_path(p, &ReadmeForm::Bundle)
    }

    #[test]
    fn bundle_path_rules() {
        assert!(bundle("README.md").is_ok());
        assert!(bundle("docs/overview.md").is_ok());
        for bad in [
            "",
            "../x",
            "a/../x",
            "/x",
            "C:/x",
            "C:foo",
            "docs/x:y",
            "a\\b",
            "a//b",
            "a/./b",
            "a\u{1}b",
            "a/b.",
            "a/b ",
            "package.json",
            "Package.JSON",
        ] {
            assert!(bundle(bad).is_err(), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn srsj_path_rules() {
        let manifest = json!({"sourceDocumentsPath": "./attachments/"});
        let keys = vec![
            "x.json".to_string(),
            "themes/a.json".to_string(),
            "pkg/package.json".to_string(),
        ];
        let f = ReadmeForm::Srsj {
            data_keys: &keys,
            manifest: &manifest,
        };
        assert!(check_path("README.md", &f).is_ok());
        assert!(check_path(".srs/README.md", &f).is_ok());
        for bad in [
            ".srs",
            "manifest.json",
            "x.json",
            "x.json/README.md",
            "themes",
            "records",
            "Records/x.md",
            "records/x.md",
            "attachments/x.md",
            "pkg/README.md",
            "pkg",
        ] {
            assert!(check_path(bad, &f).is_err(), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn absent_is_silent_and_invalid_is_dropped() {
        let f = ReadmeForm::Bundle;
        assert_eq!(read_readme(None, &f, "bundle"), ReadmeRead::default());
        let r = read_readme(Some(&json!({"path": "../x", "content": "c"})), &f, "bundle");
        assert!(r.readme.is_none());
        assert!(r.invalid.unwrap().starts_with("readme-invalid:"));
        let r = read_readme(
            Some(&json!({"path": "README.md", "content": "c"})),
            &f,
            "bundle",
        );
        assert_eq!(r.readme.unwrap().content, "c");
    }
}
