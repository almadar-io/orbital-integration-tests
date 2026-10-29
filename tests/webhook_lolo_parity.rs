// H.6 parity for webhook: every recorded webhook fixture, replayed through the kernel against `std-webhook.lolo`,
// sends the request the manual provider sent (method, URL, compared headers, body, timeout) and emits the recorded
// outcome. The harness patches the knobs the recording names before it resolves the atom with std-retry:
// `signed` follows the presence of a signing secret, `timeoutMs` the recorded number (an unreadable one keeps the
// default), and `maxAttempts` is 1 for the 5xx cases because each fixture records one exchange where the atom retries.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, KnobValue, ParitySpec, Run, UnreadableKnob};
use serde_json::{json, Value};

const DEFAULT_TIMEOUT_MS: u64 = 10_000;

// A 5xx answer is retried through std-retry; each fixture records one exchange.
const SERVER_ERRORS: [&str; 2] = ["send-server-error-fails", "send-server-error-500-fails"];

fn failure_extras(fixture: &Value) -> Vec<(&'static str, Value)> {
    vec![("attempts", json!(fixture["exchanges"].as_array().expect("exchanges").len()))]
}

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/net/atoms/std-webhook.lolo",
    trait_name: "WebhookClient",
    fixtures_dir: "webhook",
    events: &[ActionEvents {
        action: "send",
        input: "WEBHOOK_SEND",
        result: "WEBHOOK_RESULT",
        failure: Some("WEBHOOK_FAILED"),
        data_field: None,
    }],
    env_knobs: &[("WEBHOOK_TIMEOUT_MS", "timeoutMs")],
    secret_env: &["WEBHOOK_SIGNING_SECRET"],
    compared_headers: &["accept", "content-type", "x-almadar-event", "x-almadar-signature"],
    default_timeout_ms: DEFAULT_TIMEOUT_MS,
    fixture_origin: Some("https://hooks.example.test"),
    presence_knobs: &[("WEBHOOK_SIGNING_SECRET", "signed")],
    case_knobs: &[
        ("send-server-error-fails", "maxAttempts", KnobValue::Number(1)),
        ("send-server-error-500-fails", "maxAttempts", KnobValue::Number(1)),
    ],
    unreadable_knob: UnreadableKnob::UseDefault,
    failure_extras,
    ..ParitySpec::DEFAULT
};

// D-A80: the registry's param check rejects these before anything is sent; the typed event payload replaces it.
const VALIDATION_ONLY: [&str; 4] = ["unknown-action", "validation-missing-event", "validation-unknown-param", "validation-wrong-type"];

const UNPARSEABLE_URL: &str = "send-to-an-unparseable-url";

fn name(fixture: &Value) -> &str {
    lolo_parity::text(fixture, "case")
}

fn recorded() -> Vec<Value> {
    lolo_parity::fixtures(&SPEC)
        .into_iter()
        .map(|mut f| {
            if sent(&f) && f["params"].get("payload").is_none() {
                f["params"]["payload"] = json!({});
            }
            f
        })
        .collect()
}

// The atom's `payload` is a required typed field (`p!`) where the provider defaulted an absent one to `{}`: the
// kernel refuses the event before any transition, so an absent payload is replayed as the `{}` the provider sent.
fn sent(fixture: &Value) -> bool {
    !VALIDATION_ONLY.contains(&name(fixture))
}

fn check(fixture: &Value) -> lolo_parity::Run {
    lolo_parity::ParitySession::new(&SPEC, fixture, lolo_parity::FixtureAuthority::default())
        .check(fixture, lolo_parity::ReplayStep::default())
}

fn named(case: &str) -> Value {
    recorded().into_iter().find(|f| name(f) == case).unwrap_or_else(|| panic!("fixture {case}"))
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn every_recorded_webhook_outcome_is_replayed_by_the_atom() {
    let all = recorded();
    assert_eq!(all.len(), 16, "the recorded webhook cases");
    let mut replayed = 0;
    for fixture in all.iter().filter(|f| sent(f)) {
        check(fixture);
        replayed += 1;
    }
    assert_eq!(replayed, 12, "the cases with a recorded outcome");
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_5xx_reply_fails_after_the_one_allowed_attempt() {
    for case in SERVER_ERRORS {
        let run = check(&named(case));
        assert_eq!(run.sent.len(), 1, "{case}");
    }
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn an_unparseable_url_fails_with_no_exchange() {
    let fixture = named(UNPARSEABLE_URL);
    assert!(fixture["exchanges"].as_array().expect("exchanges").is_empty());
    let run = check(&fixture);
    assert!(run.sent.is_empty());
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_per_call_secret_signs_instead_of_the_env_secret() {
    let per_call = check(&named("send-signed-with-a-per-call-secret"));
    let env = check(&named("send-signed-with-the-env-secret"));
    let header = |run: &Run| {
        run.sent[0].headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("x-almadar-signature")).map(|(_, v)| v.clone())
    };
    assert_ne!(header(&per_call), header(&env));
    assert!(header(&per_call).expect("signature").starts_with("sha256="));
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn an_unsigned_send_carries_no_signature_header() {
    let run = check(&named("send-unsigned-without-any-secret"));
    assert!(run.sent[0].headers.iter().all(|(k, _)| !k.eq_ignore_ascii_case("x-almadar-signature")));
}

#[test]
#[should_panic(expected = "url")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = named("send-signed-with-the-env-secret");
    let url = lolo_parity::text(&fixture["exchanges"][0]["request"], "url").replace("/hooks/orders", "/hooks/other");
    fixture["exchanges"][0]["request"]["url"] = json!(url);
    check(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = named("send-unauthorized-is-returned-not-thrown");
    fixture["outcome"]["data"]["status"] = json!(403);
    check(&fixture);
}

#[test]
#[should_panic(expected = "headers")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_signature_fails_the_replay() {
    let mut fixture = named("send-signed-with-the-env-secret");
    fixture["exchanges"][0]["request"]["headers"]["x-almadar-signature"] = json!("sha256=00");
    check(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
