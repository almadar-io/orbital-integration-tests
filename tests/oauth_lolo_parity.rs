// H.6 parity for oauth. The recorded fixtures are 60 stateful step scenarios (105 calls). Each is replayed through
// the kernel against `std-oauth.lolo` on one session (pending grants, mock tokens and connections carry over).
//
// The atom runs on the canonical `oauth-client` auth (D-A85), whose failures are the fixed safe messages and public
// codes of `OAuthFailureReason`, never a vendor message. Every recorded failure is therefore compared against the
// canonical failure of its reason (`CANONICAL`), and each such divergence is listed explicitly; requests, exchanges
// and successful outcomes are compared exactly. The host draws state, PKCE verifier and mock tokens through
// journaled `Random` asks, so the test supplies those bytes per step.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, FixtureAuthority, ParitySession, ParitySpec};
use orbital_core::oauth::ask::OAuthFailureReason as Reason;
use serde_json::{json, Value};
use std::path::PathBuf;

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-oauth.lolo",
    trait_name: "OAuthClient",
    fixtures_dir: "oauth",
    events: &[
        ActionEvents { action: "authorize", input: "OAUTH_AUTHORIZE", result: "OAUTH_AUTHORIZED", failure: Some("OAUTH_AUTHORIZE_ERROR"), data_field: None },
        ActionEvents { action: "token", input: "OAUTH_TOKEN", result: "OAUTH_TOKEN_RESULT", failure: Some("OAUTH_TOKEN_ERROR"), data_field: None },
        ActionEvents { action: "refresh", input: "OAUTH_REFRESH", result: "OAUTH_REFRESHED", failure: Some("OAUTH_REFRESH_ERROR"), data_field: None },
        ActionEvents { action: "revoke", input: "OAUTH_REVOKE", result: "OAUTH_REVOKED", failure: Some("OAUTH_REVOKE_ERROR"), data_field: None },
        ActionEvents { action: "userinfo", input: "OAUTH_USERINFO", result: "OAUTH_USERINFO_RESULT", failure: Some("OAUTH_USERINFO_ERROR"), data_field: None },
        ActionEvents { action: "connect", input: "OAUTH_CONNECT", result: "OAUTH_CONNECTED", failure: Some("OAUTH_CONNECT_ERROR"), data_field: None },
        ActionEvents { action: "connection", input: "OAUTH_CONNECTION", result: "OAUTH_CONNECTION_RESULT", failure: Some("OAUTH_CONNECTION_ERROR"), data_field: None },
        ActionEvents { action: "disconnect", input: "OAUTH_DISCONNECT", result: "OAUTH_DISCONNECTED", failure: Some("OAUTH_DISCONNECT_ERROR"), data_field: None },
    ],
    env_knobs: &[],
    secret_env: &["OAUTH_CLIENT_ID", "OAUTH_CLIENT_SECRET"],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 30_000,
    base: None,
    ..ParitySpec::DEFAULT
};

const KNOBS: [(&str, &str); 3] = [("OAUTH_MODE", "mode"), ("OAUTH_REDIRECT_URI", "redirectUri"), ("OIDC_ISSUER_URL", "issuerUrl")];

const RECORDED_CALLS: usize = 105;

// D-A80: the registry's param check rejects these before anything is sent; the typed event payloads replace it.
const VALIDATION_ONLY: [&str; 4] = [
    "mock-unknown-action",
    "mock-validation-missing-required",
    "mock-validation-unknown-param",
    "mock-validation-wrong-type",
];

// The zod issue dump of a list whose items are not all strings: the atom's `scopes : [string]` payload cannot hold
// the recorded input, so nothing of it reaches a provider path.
const UNREPRESENTABLE_INPUT: [&str; 1] = ["mock-authorize-scopes-not-strings"];

const EXCLUDED: [&str; 5] = [
    "mock-unknown-action",
    "mock-validation-missing-required",
    "mock-validation-unknown-param",
    "mock-validation-wrong-type",
    "mock-authorize-scopes-not-strings",
];

// Recorded failure -> the canonical failure of the reason the one core flow now reports. The message and code
// change (vendor text and a `null` code become the fixed safe text and the public code); the exchanges before it
// are unchanged and still compared exactly.
const CANONICAL: &[(&str, Reason)] = &[
    ("mock-connect-needs-a-caller#0", Reason::NotSignedIn),
    ("mock-connection-needs-a-caller#0", Reason::NotSignedIn),
    ("mock-disconnect-needs-a-caller#0", Reason::NotSignedIn),
    ("mock-flow-token-userinfo-refresh-revoke#4", Reason::TokenRejected),
    ("mock-flow-token-userinfo-refresh-revoke#7", Reason::TokenRejected),
    ("mock-refresh-invalid#0", Reason::TokenRejected),
    ("mock-revoke-by-refresh-token#3", Reason::TokenRejected),
    ("mock-revoke-by-refresh-token#4", Reason::TokenRejected),
    ("mock-token-replayed-state#2", Reason::StateUnknown),
    ("mock-token-subject-mismatch-consumes-the-grant#1", Reason::SubjectMismatch),
    ("mock-token-subject-mismatch-consumes-the-grant#2", Reason::StateUnknown),
    ("mock-token-subject-missing#1", Reason::SubjectMismatch),
    ("mock-token-unknown-state#0", Reason::StateUnknown),
    ("mock-userinfo-expired#2", Reason::TokenRejected),
    ("mock-userinfo-expired#3", Reason::TokenRejected),
    ("mock-userinfo-invalid#0", Reason::TokenRejected),
    ("real-authorize-discovery-issuer-mismatch#0", Reason::MetadataInvalid),
    ("real-authorize-discovery-server-error#0", Reason::MetadataInvalid),
    ("real-authorize-discovery-without-endpoint#0", Reason::MetadataInvalid),
    ("real-connect-needs-a-caller#0", Reason::NotSignedIn),
    ("real-refresh-invalid-grant#0", Reason::TokenRejected),
    ("real-revoke-rejected#0", Reason::TokenRejected),
    ("real-revoke-without-endpoint#0", Reason::MetadataInvalid),
    ("real-token-id-token-expired#1", Reason::ResponseInvalid),
    ("real-token-id-token-missing-subject#1", Reason::ResponseInvalid),
    ("real-token-id-token-multiple-audiences#1", Reason::ResponseInvalid),
    ("real-token-id-token-wrong-audience#1", Reason::ResponseInvalid),
    ("real-token-id-token-wrong-issuer#1", Reason::ResponseInvalid),
    ("real-token-invalid-grant#1", Reason::TokenRejected),
    ("real-token-missing-access-token#1", Reason::ResponseInvalid),
    ("real-token-not-json#1", Reason::ResponseInvalid),
    ("real-token-server-error-plain-body#1", Reason::TokenRejected),
    ("real-token-subject-mismatch-consumes-the-grant#1", Reason::SubjectMismatch),
    ("real-token-subject-mismatch-consumes-the-grant#2", Reason::StateUnknown),
    ("real-token-unknown-state#0", Reason::StateUnknown),
    ("real-token-unsupported-token-type#1", Reason::ResponseInvalid),
    ("real-userinfo-challenge#0", Reason::TokenRejected),
    ("real-userinfo-missing-subject#0", Reason::ResponseInvalid),
    ("real-userinfo-subject-differs-from-id-token#2", Reason::UserinfoMismatch),
    ("real-userinfo-unauthorized#0", Reason::TokenRejected),
];

// An unknown provider is refused by the atom itself with a typed VALIDATION_ERROR (the recorded text was a zod dump).
const UNKNOWN_PROVIDER: [&str; 2] = ["mock-authorize-unknown-provider#0", "mock-connection-unknown-provider#0"];

// Recorded failures the atom reproduces verbatim (the provider's own configuration message, no HTTP).
const VERBATIM_FAILURES: [&str; 1] = ["real-authorize-auth0-without-issuer#0"];

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    lolo_parity::text(value, key)
}

fn scenarios() -> Vec<Value> {
    lolo_parity::step_scenarios(&SPEC)
}

fn is_mock(env: &Value) -> bool {
    let set = |name: &str| env[name].as_str().is_some_and(|v| !v.is_empty());
    env["OAUTH_MODE"] == "mock" || !(set("OAUTH_CLIENT_ID") && set("OAUTH_CLIENT_SECRET"))
}

// Entropy sizes the host is asked for by one step, in order.
fn draws(action: &str, mock: bool, ok: bool, key: &str) -> Vec<usize> {
    match (action, mock) {
        ("authorize", true) if ok => vec![16],
        ("authorize", false) if ok => vec![32, 32],
        ("authorize", false) if key == "real-authorize-discovery-without-endpoint#0" => vec![32, 32],
        ("token" | "connect", true) if ok => vec![32, 32, 8],
        ("refresh", true) => vec![32],
        _ => Vec::new(),
    }
}

fn bytes(counter: &mut usize, len: usize) -> Vec<u8> {
    *counter += 1;
    (0..len).map(|i| u8::try_from((*counter * 7 + i * 3) % 251).expect("byte")).collect()
}

fn failure_of(reason: Reason) -> Value {
    json!({ "ok": false, "code": reason.code(), "message": reason.message() })
}

fn canonical_outcome(key: &str, recorded: &Value) -> Option<Value> {
    if let Some((_, reason)) = CANONICAL.iter().find(|(name, _)| *name == key) {
        return Some(failure_of(*reason));
    }
    if UNKNOWN_PROVIDER.contains(&key) {
        return Some(json!({
            "ok": false,
            "code": "VALIDATION_ERROR",
            "message": "Invalid enum value. Expected 'google' | 'github' | 'auth0', received 'myspace'",
        }));
    }
    assert!(
        VERBATIM_FAILURES.contains(&key) || recorded["ok"] == true,
        "{key}: a recorded failure with no canonical mapping"
    );
    None
}

// The scenario as the atom replays it: the failures canonicalized and the entropy of every step supplied.
fn prepared(scenario: &Value) -> Value {
    let case = text(scenario, "case");
    let mock = is_mock(&scenario["env"]);
    let mut counter = 0;
    let mut out = scenario.clone();
    for (index, step) in out["steps"].as_array_mut().expect("steps").iter_mut().enumerate() {
        let key = format!("{case}#{index}");
        let ok = step["outcome"]["ok"] == true;
        let sizes = draws(text(step, "action"), mock, ok, &key);
        step["randoms"] = json!(sizes.iter().map(|len| bytes(&mut counter, *len)).collect::<Vec<_>>());
        if let Some(outcome) = canonical_outcome(&key, &step["outcome"]) {
            step["outcome"] = outcome;
        }
    }
    out
}

fn session(scenario: &Value) -> ParitySession<'static> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SPEC.atom);
    let schema = orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower: {e}"));
    let mut schema = serde_json::to_value(&schema).expect("schema json");
    for (variable, knob) in KNOBS {
        let Some(raw) = scenario["env"][variable].as_str() else { continue };
        let mut patched = false;
        for orbital in schema["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                if let Some(entry) = t["config"].get_mut(knob) {
                    entry["default"] = json!(raw);
                    patched = true;
                }
            }
        }
        assert!(patched, "{}: no knob {knob}", text(scenario, "case"));
        if knob == "issuerUrl" {
            let origin: String = raw.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
            schema["egress"].as_array_mut().expect("egress").push(json!(origin));
        }
    }
    let schema = serde_json::from_value(schema).expect("schema back");
    let authority = FixtureAuthority {
        now_ms: scenario.get("nowMs").map(|ms| ms.as_u64().expect("nowMs")),
        ..FixtureAuthority::default()
    };
    ParitySession::with_schema(&SPEC, schema, scenario, authority)
}

fn replay(scenario: &Value) -> usize {
    let scenario = prepared(scenario);
    session(&scenario).check_steps(&scenario).len()
}

fn named(case: &str) -> Value {
    scenarios().into_iter().find(|s| text(s, "case") == case).unwrap_or_else(|| panic!("scenario {case}"))
}

#[test]
fn the_recorded_set_is_60_scenarios_and_105_calls() {
    let all = scenarios();
    assert_eq!(all.len(), 60);
    let calls: usize = all.iter().map(|s| s["steps"].as_array().expect("steps").len()).sum();
    assert_eq!(calls, RECORDED_CALLS);
}

#[test]
fn every_recorded_request_and_outcome_is_replayed_by_the_atom() {
    let mut replayed = 0;
    for scenario in scenarios() {
        if EXCLUDED.contains(&text(&scenario, "case")) {
            continue;
        }
        replayed += replay(&scenario);
    }
    assert_eq!(replayed, RECORDED_CALLS - EXCLUDED.len(), "every call but the excluded ones");
}

#[test]
fn the_excluded_calls_are_validation_errors_that_send_nothing() {
    for case in EXCLUDED {
        let scenario = named(case);
        let steps = scenario["steps"].as_array().expect("steps");
        assert_eq!(steps.len(), 1, "{case}");
        assert_eq!(steps[0]["exchanges"].as_array().expect("exchanges").len(), 0, "{case}");
        assert_eq!(steps[0]["outcome"]["ok"], false, "{case}");
        if VALIDATION_ONLY.contains(&case) {
            assert_eq!(steps[0]["outcome"]["code"], "VALIDATION_ERROR", "{case}");
        }
    }
    let by_code: Vec<String> = scenarios()
        .iter()
        .filter(|s| s["steps"].as_array().expect("steps").iter().any(|st| st["outcome"]["code"] == "VALIDATION_ERROR"))
        .map(|s| text(s, "case").to_string())
        .collect();
    let mut listed: Vec<String> = VALIDATION_ONLY.iter().map(|c| c.to_string()).collect();
    listed.sort();
    let mut found = by_code;
    found.sort();
    assert_eq!(found, listed, "the registry validation-only cases");
    let items = named(UNREPRESENTABLE_INPUT[0])["steps"][0]["params"]["scopes"].clone();
    assert!(items.as_array().expect("scopes").iter().any(|v| !v.is_string()), "the recorded scopes hold a non-string");
}

#[test]
fn every_canonical_mapping_names_a_recorded_failure() {
    let mut names: Vec<String> = Vec::new();
    for scenario in scenarios() {
        for (index, step) in scenario["steps"].as_array().expect("steps").iter().enumerate() {
            if step["outcome"]["ok"] == false {
                names.push(format!("{}#{index}", text(&scenario, "case")));
            }
        }
    }
    let mapped = CANONICAL.iter().map(|(name, _)| *name).chain(UNKNOWN_PROVIDER).chain(VERBATIM_FAILURES);
    let excluded_failures = EXCLUDED.iter().map(|c| format!("{c}#0"));
    let mut accounted: Vec<String> = mapped.map(str::to_string).chain(excluded_failures).collect();
    accounted.sort();
    names.sort();
    assert_eq!(names, accounted, "every recorded failure is mapped, verbatim or excluded exactly once");
}

#[test]
fn the_incomplete_credentials_fixture_is_replayed_on_the_mock_backend() {
    let scenario = prepared(&named("mock-incomplete-credentials-stay-mock"));
    assert_eq!(scenario["steps"][0]["randoms"].as_array().expect("randoms").len(), 1);
    assert_eq!(scenario["steps"][0]["exchanges"].as_array().expect("exchanges").len(), 0);
    session(&scenario).check_steps(&scenario);
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut scenario = named("real-flow-google");
    let url = text(&scenario["steps"][0]["exchanges"][0]["request"], "url").replace("openid-configuration", "openid-configurations");
    scenario["steps"][0]["exchanges"][0]["request"]["url"] = json!(url);
    replay(&scenario);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut scenario = named("real-flow-github");
    scenario["steps"][1]["outcome"]["data"]["expiresIn"] = json!(1);
    replay(&scenario);
}

#[test]
#[should_panic(expected = "emitted failure")]
fn a_drifted_failure_reason_fails_the_replay() {
    let mut scenario = prepared(&named("real-token-invalid-grant"));
    scenario["steps"][1]["outcome"] = failure_of(Reason::ResponseInvalid);
    session(&scenario).check_steps(&scenario);
}

fn step(action: &str, params: Value, user: Option<&str>, randoms: Vec<Vec<u8>>, outcome: Value) -> Value {
    let mut step = json!({ "action": action, "params": params, "exchanges": [], "randoms": randoms, "outcome": outcome });
    if let Some(id) = user {
        step["user"] = json!({ "id": id, "role": "member" });
    }
    step
}

fn row(connected: bool, account: &str, scope: &[&str], connected_at: &str) -> Value {
    json!({ "ok": true, "data": {
        "provider": "google", "connected": connected, "account": account, "avatarUrl": "", "scope": scope, "connectedAt": connected_at,
    } })
}

fn connection_scenario() -> Value {
    let at = "2027-01-15T08:00:00.000Z";
    let authorize = |counter: usize| {
        step(
            "authorize",
            json!({ "provider": "google", "scopes": ["openid"], "redirectUri": "https://app.example.com/callback", "subject": "alice" }),
            None,
            vec![vec![u8::try_from(counter).expect("byte"); 16]],
            json!({ "ok": true, "data": { "authUrl": "https://accounts.google.com/o/oauth2/v2/auth?response_type=code&scope=openid&redirect_uri=https%3A%2F%2Fapp.example.com%2Fcallback&state=<hex32>&client_id=mock-client-id", "state": "<hex32>" } }),
        )
    };
    let tokens = |counter: usize| vec![vec![u8::try_from(counter).expect("byte"); 32], vec![u8::try_from(counter + 1).expect("byte"); 32], vec![u8::try_from(counter + 2).expect("byte"); 8]];
    let provider = json!({ "provider": "google" });
    let denied = failure_of(Reason::NotSignedIn);
    json!({
        "service": "oauth",
        "case": "principal-isolation",
        "nowMs": 1_800_000_000_000_u64,
        "env": { "OAUTH_MODE": "mock" },
        "steps": [
            authorize(1),
            step("connect", json!({ "provider": "google", "code": "c", "state": "{{0.state}}" }), Some("alice"), tokens(10), row(true, "alice", &["openid"], at)),
            step("connection", provider.clone(), Some("alice"), vec![], row(true, "alice", &["openid"], at)),
            step("connection", provider.clone(), Some("bob"), vec![], row(false, "", &[], "")),
            step("disconnect", provider.clone(), Some("bob"), vec![], row(false, "", &[], "")),
            step("connection", provider.clone(), Some("alice"), vec![], row(true, "alice", &["openid"], at)),
            step("connection", provider.clone(), None, vec![], denied.clone()),
            step("disconnect", provider.clone(), None, vec![], denied.clone()),
            step("connect", json!({ "provider": "google", "code": "c", "state": "anything" }), None, vec![], denied),
            authorize(30),
            step("connect", json!({ "provider": "google", "code": "c", "state": "{{9.state}}" }), Some("bob"), vec![], failure_of(Reason::SubjectMismatch)),
            step("connect", json!({ "provider": "google", "code": "c", "state": "{{9.state}}" }), Some("alice"), vec![], failure_of(Reason::StateUnknown)),
            step("connection", provider.clone(), Some("alice"), vec![], row(true, "alice", &["openid"], at)),
            step("disconnect", provider.clone(), Some("alice"), vec![], row(false, "", &[], "")),
            step("connection", provider, Some("alice"), vec![], row(false, "", &[], "")),
        ],
    })
}

fn github_userinfo_scenario(domain: &str) -> Value {
    let hex = |len: usize, seed: u8| vec![seed; len];
    json!({
        "service": "oauth",
        "case": "github-issued-mock-userinfo",
        "nowMs": 1_800_000_000_000_u64,
        "env": { "OAUTH_MODE": "mock" },
        "steps": [
            step("authorize", json!({ "provider": "github", "scopes": ["repo"], "redirectUri": "https://app.example.com/callback" }), None, vec![hex(16, 3)],
                json!({ "ok": true, "data": { "authUrl": "https://github.com/login/oauth/authorize?response_type=code&scope=repo&redirect_uri=https%3A%2F%2Fapp.example.com%2Fcallback&state=<hex32>&client_id=mock-client-id", "state": "<hex32>" } })),
            step("token", json!({ "code": "c", "state": "{{0.state}}" }), None, vec![hex(32, 4), hex(32, 5), hex(8, 6)],
                json!({ "ok": true, "data": { "accessToken": "<hex64>", "refreshToken": "<hex64>", "expiresIn": 3600, "tokenType": "bearer", "scope": ["repo"] } })),
            step("userinfo", json!({ "accessToken": "{{1.accessToken}}" }), None, vec![],
                json!({ "ok": true, "data": {
                    "sub": "<hex16>",
                    "email": format!("user-<hex8>@{domain}.example.com"),
                    "name": format!("Mock User ({domain})"),
                    "picture": format!("https://{domain}.example.com/avatar/<hex8>.png"),
                } })),
        ],
    })
}

#[test]
fn a_github_issued_mock_token_yields_a_github_profile() {
    let scenario = github_userinfo_scenario("github");
    assert_eq!(session(&scenario).check_steps(&scenario).len(), 3);
}

#[test]
#[should_panic(expected = "emitted result")]
fn control_a_github_issued_mock_token_is_not_a_google_profile() {
    let scenario = github_userinfo_scenario("google");
    session(&scenario).check_steps(&scenario);
}

#[test]
fn github_with_incomplete_credentials_authorizes_on_the_mock_backend() {
    let scenario = json!({
        "service": "oauth",
        "case": "github-incomplete-credentials",
        "nowMs": 1_800_000_000_000_u64,
        "env": { "OAUTH_CLIENT_ID": "client-id-1" },
        "steps": [step(
            "authorize",
            json!({ "provider": "github", "scopes": ["repo"], "redirectUri": "https://app.example.com/callback" }),
            None,
            vec![vec![9; 16]],
            json!({ "ok": true, "data": { "authUrl": "https://github.com/login/oauth/authorize?response_type=code&scope=repo&redirect_uri=https%3A%2F%2Fapp.example.com%2Fcallback&state=<hex32>&client_id=mock-client-id", "state": "<hex32>" } }),
        )],
    });
    session(&scenario).check_steps(&scenario);
}

#[test]
fn a_connection_belongs_to_its_signed_in_caller_and_anonymous_is_denied() {
    let scenario = connection_scenario();
    let runs = session(&scenario).check_steps(&scenario);
    assert_eq!(runs.len(), 15);
    assert!(runs.iter().all(|(_, run)| run.sent.is_empty()), "the mock backend sends nothing");
}

#[test]
#[should_panic(expected = "emitted result")]
fn control_user_b_would_see_the_connection_if_it_were_shared() {
    let mut scenario = connection_scenario();
    scenario["steps"][3]["outcome"] = row(true, "alice", &["openid"], "2027-01-15T08:00:00.000Z");
    session(&scenario).check_steps(&scenario);
}

#[test]
#[should_panic(expected = "unexpected failure event")]
fn control_an_anonymous_caller_is_not_given_a_row() {
    let mut scenario = connection_scenario();
    scenario["steps"][6]["outcome"] = row(false, "", &[], "");
    session(&scenario).check_steps(&scenario);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
