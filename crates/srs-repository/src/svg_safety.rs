//! Refuse an SVG that could run a script or reach outside itself (decision
//! 866167c1, srs-rust#1607). The check only reads: a file is stored byte for
//! byte or refused with `svg-unsafe`, never rewritten.

use crate::error::RepositoryError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

const SVG: &str = "image/svg+xml";

fn refuse(rule: &'static str, offending: impl Into<String>) -> RepositoryError {
    RepositoryError::SvgUnsafe {
        rule,
        offending: offending.into(),
    }
}

/// Check `content` when it is an SVG: declared `image/svg+xml`, or any XML
/// document whose root element is `<svg>`. Anything else passes untouched.
pub fn check_svg(content: &[u8], content_type: &str) -> Result<(), RepositoryError> {
    let declared = content_type
        .split(';')
        .next()
        .is_some_and(|t| t.trim().eq_ignore_ascii_case(SVG));
    let bytes = content.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(content);
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) if declared => return Err(refuse("not-well-formed", "not UTF-8")),
        Err(_) => return Ok(()),
    };
    if !declared && !text.trim_start().starts_with('<') {
        return Ok(());
    }
    scan(text, declared)
}

fn scan(text: &str, declared: bool) -> Result<(), RepositoryError> {
    let mut reader = Reader::from_str(text);
    let mut is_svg = declared;
    let mut root_seen = false;
    // A finding before the root element counts only if the root is an <svg>.
    let mut pending: Option<RepositoryError> = None;
    let mut style_depth = 0usize;
    let mut style = String::new();
    loop {
        let event = match reader.read_event() {
            Ok(e) => e,
            Err(e) if is_svg => return Err(refuse("not-well-formed", e.to_string())),
            Err(_) => return Ok(()),
        };
        let finding = match &event {
            Event::Start(e) | Event::Empty(e) => {
                if !root_seen {
                    root_seen = true;
                    is_svg = declared || e.local_name().as_ref().eq_ignore_ascii_case("svg");
                    if !is_svg {
                        return Ok(());
                    }
                    if let Some(p) = pending.take() {
                        return Err(p);
                    }
                }
                let is_style = e.local_name().as_ref().eq_ignore_ascii_case("style");
                if is_style && matches!(event, Event::Start(_)) {
                    style_depth += 1;
                }
                check_element(e).err()
            }
            Event::End(e) => {
                if style_depth > 0 && e.local_name().as_ref().eq_ignore_ascii_case("style") {
                    style_depth -= 1;
                    if style_depth == 0 {
                        check_css(&std::mem::take(&mut style))?;
                    }
                }
                None
            }
            Event::Text(t) if style_depth > 0 => {
                style.push_str(&t.clone().into_inner());
                None
            }
            Event::CData(t) if style_depth > 0 => {
                style.push_str(&t.clone().into_inner());
                None
            }
            Event::GeneralRef(r) => match r.resolve_char_ref() {
                Ok(Some(c)) => {
                    if style_depth > 0 {
                        style.push(c);
                    }
                    None
                }
                _ => match predefined(&r.clone().into_inner()) {
                    Some(c) => {
                        if style_depth > 0 {
                            style.push(c);
                        }
                        None
                    }
                    None => Some(refuse("entity", format!("&{};", r.clone().into_inner()))),
                },
            },
            Event::DocType(d) => {
                let d = d.clone().into_inner();
                // An external-id-only DOCTYPE is inert (never fetched); the internal
                // subset is where entity declarations, XXE and billion laughs live.
                d.contains('[')
                    .then(|| refuse("doctype-subset", format!("<!DOCTYPE {}>", d.trim())))
            }
            Event::PI(p) => {
                let p = p.clone().into_inner();
                let target = p.split_whitespace().next().unwrap_or("");
                target
                    .eq_ignore_ascii_case("xml-stylesheet")
                    .then(|| refuse("external-reference", format!("<?{}?>", p.trim())))
            }
            Event::Eof => {
                return match (root_seen && is_svg, pending) {
                    (true, _) => Ok(()),
                    (false, Some(p)) if declared => Err(p),
                    (false, _) if declared => Err(refuse("not-well-formed", "no root element")),
                    _ => Ok(()),
                };
            }
            _ => None,
        };
        if let Some(f) = finding {
            if root_seen {
                return Err(f);
            }
            pending.get_or_insert(f);
        }
    }
}

fn predefined(name: &str) -> Option<char> {
    Some(match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => return None,
    })
}

fn check_element(e: &BytesStart) -> Result<(), RepositoryError> {
    let name = e.name().as_ref().to_string();
    let local = e.local_name().as_ref().to_ascii_lowercase();
    match local.as_str() {
        "script" | "handler" => return Err(refuse("script-element", format!("<{name}>"))),
        "foreignobject" | "iframe" | "embed" | "object" => {
            return Err(refuse("foreign-content", format!("<{name}>")))
        }
        _ => {}
    }
    let animates = matches!(local.as_str(), "animate" | "set" | "animatemotion");
    for attr in e.attributes() {
        let attr = attr.map_err(|err| refuse("not-well-formed", err.to_string()))?;
        let key = attr.key.as_ref().to_string();
        let key_local = attr.key.local_name().as_ref().to_ascii_lowercase();
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| refuse("entity", format!("<{name} {key}>: {err}")))?;
        let at = || format!("<{name} {key}=\"{value}\">");
        if key_local.starts_with("on") {
            return Err(refuse("event-attribute", at()));
        }
        if animates
            && key_local == "attributename"
            && value
                .rsplit(':')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("href"))
        {
            return Err(refuse("external-reference", at()));
        }
        // A URL scheme hides behind whitespace or control characters ("java\tscript:").
        let squashed: String = value
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && !c.is_ascii_control())
            .collect::<String>()
            .to_ascii_lowercase();
        for part in squashed.split(';') {
            if let Some(rule) = bad_scheme(part) {
                return Err(refuse(rule, at()));
            }
        }
        if key_local == "href" && !value.trim().starts_with('#') {
            return Err(refuse("external-reference", at()));
        }
        if key_local == "style" || squashed.contains("url(") {
            check_css(&value).map_err(|e| match e {
                RepositoryError::SvgUnsafe { rule, .. } => refuse(rule, at()),
                e => e,
            })?;
        }
    }
    Ok(())
}

fn bad_scheme(s: &str) -> Option<&'static str> {
    if s.starts_with("javascript:") || s.starts_with("vbscript:") {
        Some("script-url")
    } else if s.starts_with("data:") {
        Some("data-url")
    } else {
        None
    }
}

/// CSS from a `style` attribute, a presentation attribute or a `<style>`
/// element: every `url()` must name a same-document `#fragment`; `@import`,
/// `image-set()` and CSS escapes (which could hide either) are refused.
fn check_css(css: &str) -> Result<(), RepositoryError> {
    let lower = css.to_ascii_lowercase();
    let short = || css.trim().chars().take(120).collect::<String>();
    if lower.contains('\\') {
        return Err(refuse("css-escape", short()));
    }
    if lower.contains("@import") || lower.contains("image-set(") {
        return Err(refuse("external-reference", short()));
    }
    for (i, _) in lower.match_indices("url(") {
        let inner = lower[i + 4..]
            .trim_start()
            .trim_start_matches(['"', '\''])
            .trim_start();
        if !inner.starts_with('#') {
            let rule = bad_scheme(inner).unwrap_or("external-reference");
            return Err(refuse(rule, short()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures(kind: &str) -> Vec<(String, Vec<u8>)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/svg")
            .join(kind);
        let mut out: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read(&p).unwrap(),
                )
            })
            .collect();
        out.sort();
        assert!(!out.is_empty(), "no fixtures in {dir:?}");
        out
    }

    /// Each hostile fixture is named `<rule>--<case>.svg` and must be refused
    /// with exactly that rule, both when declared and when only sniffed.
    #[test]
    fn hostile_fixtures_are_refused_with_their_rule() {
        for (name, bytes) in fixtures("hostile") {
            let want = name.split("--").next().unwrap();
            for ct in [SVG, "application/octet-stream"] {
                match check_svg(&bytes, ct) {
                    Err(e @ RepositoryError::SvgUnsafe { rule, .. }) => {
                        assert_eq!(rule, want, "{name} ({ct}): {e}");
                        assert_eq!(e.code(), "svg-unsafe");
                        let d = e.report().details.unwrap();
                        assert_eq!(d["rule"], want);
                        assert!(d["offending"].as_str().is_some_and(|s| !s.is_empty()));
                    }
                    other => panic!("{name} ({ct}) not refused: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn real_exports_are_accepted() {
        for (name, bytes) in fixtures("real") {
            check_svg(&bytes, SVG).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn non_svg_content_is_untouched() {
        // PNG magic, an HTML page with a script, plain text, and an XML file with a DOCTYPE subset.
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        check_svg(png, "image/png").unwrap();
        check_svg(png, "application/octet-stream").unwrap();
        check_svg(
            b"<!DOCTYPE html><html><script>x()</script></html>",
            "text/html",
        )
        .unwrap();
        check_svg(b"just text", "text/plain").unwrap();
        check_svg(
            b"<?xml version=\"1.0\"?><!DOCTYPE a [<!ENTITY e \"x\">]><a>&e;</a>",
            "application/xml",
        )
        .unwrap();
    }

    #[test]
    fn declared_svg_must_parse() {
        for bad in [&b"\xff\xfe<svg/>"[..], b"<svg><g></svg>", b"", b"not xml"] {
            let e = check_svg(bad, "image/svg+xml; charset=utf-8").unwrap_err();
            assert!(
                matches!(
                    e,
                    RepositoryError::SvgUnsafe {
                        rule: "not-well-formed",
                        ..
                    }
                ),
                "{e}"
            );
        }
    }
}
