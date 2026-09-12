//! Structural guard for the transport-neutral Streamable HTTP conformance vectors.
//!
//! This is intentionally not an HTTP-server test: `srs-mcp` currently owns stdio
//! only. The future native HTTP and browser/WASM adapters execute the exact same
//! manifest after the compatibility spike selects this profile.

use std::collections::BTreeSet;

use serde::Deserialize;

const FIXTURES: &str = include_str!("fixtures/streamable-http-2025-06-18.json");

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    profile: String,
    specification: String,
    #[serde(rename = "adapterContract")]
    adapter_contract: AdapterContract,
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct AdapterContract {
    #[serde(rename = "requestMetadata")]
    request_metadata: Vec<String>,
    #[serde(rename = "successHeaders")]
    success_headers: std::collections::BTreeMap<String, String>,
    #[serde(rename = "unsupportedMethodHeaders")]
    unsupported_method_headers: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    id: String,
    request: FixtureRequest,
    expect: FixtureExpectation,
}

#[derive(Debug, Deserialize)]
struct FixtureRequest {
    method: String,
    headers: std::collections::BTreeMap<String, String>,
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FixtureExpectation {
    status: u16,
    #[serde(rename = "bodyKind")]
    body_kind: String,
    dispatch: String,
    headers: std::collections::BTreeMap<String, String>,
}

#[test]
fn streamable_http_profile_fixture_manifest_is_complete_and_well_formed() {
    let manifest: FixtureManifest = serde_json::from_str(FIXTURES)
        .expect("Streamable HTTP fixture manifest must be valid JSON");

    assert_eq!(
        manifest.profile, "mcp-streamable-http-2025-06-18-json-only",
        "a revision/profile change needs an explicit new fixture manifest"
    );
    assert_eq!(
        manifest.specification,
        "https://modelcontextprotocol.io/specification/2025-06-18/basic/transports"
    );
    assert_eq!(
        manifest.adapter_contract.request_metadata,
        [
            "content-type",
            "accept",
            "mcp-protocol-version",
            "mcp-session-id",
            "origin"
        ]
    );
    assert_eq!(
        manifest
            .adapter_contract
            .success_headers
            .get("cache-control"),
        Some(&"no-store".to_owned())
    );
    assert_eq!(
        manifest
            .adapter_contract
            .success_headers
            .get("referrer-policy"),
        Some(&"no-referrer".to_owned())
    );
    assert_eq!(
        manifest
            .adapter_contract
            .unsupported_method_headers
            .get("allow"),
        Some(&"POST".to_owned())
    );

    let required_case_ids = BTreeSet::from([
        "initialize-without-http-version",
        "request-with-negotiated-http-version",
        "notification-is-accepted-empty",
        "cancellation-is-forwarded-notification",
        "json-rpc-response-is-accepted-empty",
        "batch-is-rejected-before-core",
        "malformed-json-is-parse-error",
        "invalid-json-rpc-is-invalid-request",
        "unsupported-content-type",
        "accept-must-offer-json-and-sse",
        "missing-version-after-initialize-is-rejected",
        "unsupported-version-is-rejected",
        "session-header-is-rejected",
        "get-sse-is-deliberately-unsupported",
        "delete-session-is-deliberately-unsupported",
        "unexpected-browser-origin-is-rejected",
    ]);
    let actual_case_ids: BTreeSet<&str> =
        manifest.cases.iter().map(|case| case.id.as_str()).collect();
    assert_eq!(
        actual_case_ids.len(),
        manifest.cases.len(),
        "fixture case IDs must be unique"
    );
    assert_eq!(actual_case_ids, required_case_ids);

    for case in &manifest.cases {
        assert!(
            matches!(case.expect.status, 200 | 202 | 400 | 403 | 405 | 406 | 415),
            "{} uses a status outside this profile: {}",
            case.id,
            case.expect.status
        );
        assert!(
            matches!(
                case.expect.body_kind.as_str(),
                "empty" | "json-rpc-response" | "json-rpc-parse-error" | "json-rpc-invalid-request"
            ),
            "{} has an unknown response body kind",
            case.id
        );
        assert!(
            matches!(
                case.expect.dispatch.as_str(),
                "none" | "core" | "core-error"
            ),
            "{} has an unknown dispatch disposition",
            case.id
        );

        if let Some(body) = &case.request.body {
            if case.id != "malformed-json-is-parse-error" {
                serde_json::from_str::<serde_json::Value>(body)
                    .unwrap_or_else(|error| panic!("{} body must be valid JSON: {error}", case.id));
            }
        }

        if case.request.method == "POST" {
            assert!(case.request.body.is_some(), "{} POST needs a body", case.id);
            assert!(
                case.request.headers.contains_key("content-type"),
                "{} POST must make its content-type gate executable",
                case.id
            );
            assert!(
                case.request.headers.contains_key("accept"),
                "{} POST must make its Accept gate executable",
                case.id
            );
        } else {
            assert_eq!(
                case.expect.status, 405,
                "{} must be a rejected method",
                case.id
            );
            assert_eq!(
                case.expect.dispatch, "none",
                "{} must not dispatch",
                case.id
            );
            assert_eq!(case.expect.headers.get("allow"), Some(&"POST".to_owned()));
        }

        assert!(
            case.request
                .headers
                .keys()
                .all(|header| header == &header.to_ascii_lowercase()),
            "{} request headers must use canonical lowercase fixture keys",
            case.id
        );
    }
}
