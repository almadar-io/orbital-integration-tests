// H.6 parity: the knowledge-web `currentTopics` recordings replayed through the kernel against
// `std-knowledge-topics.lolo` make the three KFlow reads the provider made and emit the recorded outcome.
// `status` and `connect` stay host actions (interactive sign-in, session storage), so their fixtures are not the atom's.
// The sign-in is the Firebase refresh-token exchange (D-A86): a connected learner holds a fresh cached ID token, so the
// recorded reads carry no refresh; the refresh, its rotation and its failure are pinned by the tests below.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{
    ActionEvents, Deadline, ExchangeContract, FixtureAuthority, FixtureCredential, ParitySession, ParitySpec,
    ReplayStep,
};
use orbital_core::runtime::http_request::CredentialSlot;
use serde_json::{json, Value as Json};

const ORIGIN: &str = "https://kflow.example.com";
const REFRESH: &str = "KNOWLEDGE_REFRESH_TOKEN";
const ID_TOKEN: &str = "KNOWLEDGE_ID_TOKEN";

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-knowledge-topics.lolo",
    trait_name: "KnowledgeTopicsClient",
    fixtures_dir: "knowledge-web",
    events: &[ActionEvents {
        action: "currentTopics",
        input: "KNOWLEDGE_CURRENT_TOPICS",
        result: "KNOWLEDGE_TOPICS_RESULT",
        failure: Some("KNOWLEDGE_TOPICS_FAILED"),
        data_field: None,
    }],
    env_knobs: &[],
    secret_env: &[REFRESH],
    compared_headers: &["accept", "authorization", "content-type"],
    default_timeout_ms: 0,
    base: Some(("kflow", ORIGIN)),
};

// The sign-in and the session record stay host actions of the knowledge provider.
const HOST_ACTIONS: [&str; 3] = ["connect", "status-connected", "status-signed-out"];
// Registry validation only, nothing sent: an unknown action and a non-numeric limit (the event payload is typed).
const VALIDATION_ONLY: [&str; 2] = ["unknown-action", "topics-limit-wrong-type"];
// Approved divergence (owner): labels with equal weights are ordered by code point, not ICU localeCompare.
const LOCALE_ORDER: &str = "topics-equal-weights-order-by-label";

fn connected(fixture: &Json) -> bool {
    fixture["env"]["KNOWLEDGE_TEST_CONNECTED"] == "true"
}

fn authority(fixture: &Json) -> FixtureAuthority {
    let held = |name: &str, value: &str| FixtureCredential {
        slot: CredentialSlot::from(name),
        principal: None,
        value: connected(fixture).then(|| value.to_string()),
        ttl_ms: None,
    };
    FixtureAuthority {
        credentials: vec![held(REFRESH, "test-refresh"), held(ID_TOKEN, "test-id-token")],
        now_ms: None,
    }
}

fn contracts(fixture: &Json) -> Vec<ExchangeContract> {
    let count = fixture["exchanges"].as_array().expect("exchanges").len();
    (0..count)
        .map(|_| ExchangeContract { timeout: Deadline::Exact(None), ..Default::default() })
        .collect()
}

fn replay(fixture: &Json) {
    ParitySession::new(&SPEC, fixture, authority(fixture))
        .check(fixture, ReplayStep { exchanges: contracts(fixture), ..Default::default() });
}

fn replayable() -> Vec<Json> {
    let mut excluded = HOST_ACTIONS.to_vec();
    excluded.extend(VALIDATION_ONLY);
    excluded.push(LOCALE_ORDER);
    lolo_parity::fixtures(&SPEC)
        .into_iter()
        .filter(|f| !excluded.contains(&lolo_parity::text(f, "case")))
        .collect()
}

fn with_grant(mut fixture: Json, grant_status: u64, grant_reply: &str) -> Json {
    let grant = json!({
        "request": {
            "method": "POST",
            "url": "https://securetoken.googleapis.com/v1/token",
            "headers": { "content-type": "application/x-www-form-urlencoded" },
            "body": "grant_type=refresh_token&refresh_token=test-refresh"
        },
        "response": { "status": grant_status, "body": grant_reply }
    });
    fixture["exchanges"].as_array_mut().expect("exchanges").insert(0, grant);
    fixture
}

fn expired_authority() -> FixtureAuthority {
    FixtureAuthority {
        credentials: vec![
            FixtureCredential { slot: CredentialSlot::from(REFRESH), principal: None, value: Some("test-refresh".into()), ttl_ms: None },
            FixtureCredential { slot: CredentialSlot::from(ID_TOKEN), principal: None, value: None, ttl_ms: None },
        ],
        now_ms: None,
    }
}

#[test]
fn every_recorded_currenttopics_outcome_is_replayed_by_the_atom() {
    assert_eq!(lolo_parity::fixtures(&SPEC).len(), 16, "the recorded knowledge-web cases");
    let cases = replayable();
    assert_eq!(cases.len(), 10, "the currentTopics cases with a request or a mapped outcome");
    for fixture in &cases {
        replay(fixture);
    }
}

#[test]
fn the_cases_the_atom_does_not_take_are_host_actions_or_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
    for case in HOST_ACTIONS {
        assert_ne!(lolo_parity::text(&lolo_parity::fixture_named(&SPEC, case), "action"), "currentTopics");
    }
}

#[test]
fn equal_weight_labels_are_ordered_by_code_point_an_approved_divergence() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, LOCALE_ORDER);
    let topics = fixture["outcome"]["data"]["topics"].as_array_mut().expect("topics");
    let label = |t: &Json| t["label"].as_str().expect("label").to_string();
    topics.sort_by_key(label);
    let order: Vec<String> = topics.iter().map(label).collect();
    assert_eq!(order, ["Apple", "apple", "banana", "cherry"]);
    replay(&fixture);
}

#[test]
fn an_expired_id_token_is_refreshed_once_rotated_and_used_for_all_three_reads() {
    let base = lolo_parity::fixture_named(&SPEC, "topics-compose");
    let fixture = with_grant(
        base,
        200,
        r#"{"id_token":"test-new-id","refresh_token":"test-rotated","expires_in":"3600"}"#,
    );
    let mut session = ParitySession::new(&SPEC, &fixture, expired_authority());
    let run = session.check(&fixture, ReplayStep { exchanges: contracts(&fixture), ..Default::default() });
    assert_eq!(run.sent.len(), 4, "one refresh, then the three reads");
    assert_eq!(session.rotations(), vec![(REFRESH.to_string(), None)]);
    assert_eq!(session.held_credential(REFRESH, None).as_deref(), Some("test-rotated"));
}

#[test]
fn a_refresh_the_sign_in_service_rejects_ends_the_call_before_any_read() {
    let mut fixture = with_grant(lolo_parity::fixture_named(&SPEC, "topics-compose"), 400, r#"{"error":"TOKEN_EXPIRED"}"#);
    fixture["exchanges"].as_array_mut().expect("exchanges").truncate(1);
    fixture["outcome"] = json!({ "ok": false, "code": "AUTH_ERROR", "message": "Sign-in failed (400)" });
    let mut session = ParitySession::new(&SPEC, &fixture, expired_authority());
    let run = session.check(&fixture, ReplayStep { exchanges: contracts(&fixture), ..Default::default() });
    assert_eq!(run.sent.len(), 1);
    assert!(session.rotations().is_empty(), "a rejected refresh rotates nothing");
    assert_eq!(session.held_credential(REFRESH, None).as_deref(), Some("test-refresh"));
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "topics-compose");
    fixture["exchanges"][2]["request"]["url"] = json!(format!("{ORIGIN}/api/user/statistics/recent-activity?limit=51"));
    replay(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "topics-compose");
    fixture["outcome"]["data"]["topics"][0]["label"] = json!("Prim");
    replay(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
