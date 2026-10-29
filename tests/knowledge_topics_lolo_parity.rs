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
use orbital_core::runtime::http_wire::CredentialSlot;
use serde_json::{json, Value as Json};

const ORIGIN: &str = "https://kflow.example.com";
const REFRESH: &str = "KNOWLEDGE_REFRESH_TOKEN";
const ID_TOKEN: &str = "KNOWLEDGE_ID_TOKEN";

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-behaviors/behaviors/lolo/kflow/atoms/kflow-topics.lolo",
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
    ..ParitySpec::DEFAULT
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
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
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
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
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
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
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
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
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
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "topics-compose");
    fixture["exchanges"][2]["request"]["url"] = json!(format!("{ORIGIN}/api/user/statistics/recent-activity?limit=51"));
    replay(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "topics-compose");
    fixture["outcome"]["data"]["topics"][0]["label"] = json!("Prim");
    replay(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}

// The concept icon and persona organisms replace the host actions `conceptIcon` and `conceptPersona`. Their recorded
// exchanges are the host's: the Firestore cache read/write (`{base}`) is the kernel's entity store here, so those
// exchanges are dropped and a cache hit is staged by running a miss first. Every other exchange is replayed exactly.

const ICON_SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/organisms/std-concept-icon.lolo",
    trait_name: "ConceptIconResolver",
    fixtures_dir: "knowledge",
    events: &[ActionEvents {
        action: "conceptIcon",
        input: "CONCEPT_ICON",
        result: "CONCEPT_ICON_RESULT",
        failure: Some("CONCEPT_ICON_FAILED"),
        data_field: None,
    }],
    env_knobs: &[],
    secret_env: &[],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 6000,
    base: None,
    ..ParitySpec::DEFAULT
};

const PERSONA_SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/organisms/std-concept-persona.lolo",
    trait_name: "ConceptPersonaResolver",
    fixtures_dir: "knowledge",
    events: &[ActionEvents {
        action: "conceptPersona",
        input: "CONCEPT_PERSONA",
        result: "CONCEPT_PERSONA_RESULT",
        failure: Some("CONCEPT_PERSONA_FAILED"),
        data_field: None,
    }],
    env_knobs: &[],
    secret_env: &["DEEPSEEK_API_KEY"],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 6000,
    base: None,
    ..ParitySpec::DEFAULT
};

const MODEL_TIMEOUT_MS: u64 = 300_000;
const HTTP_TIMEOUT_MS: u64 = 6000;

fn knowledge_fixture(case: &str) -> Json {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/knowledge")
        .join(format!("{case}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn without_cache_exchanges(mut fixture: Json) -> Json {
    fixture.as_object_mut().expect("fixture").remove("now");
    fixture["exchanges"]
        .as_array_mut()
        .expect("exchanges")
        .retain(|e| !e["request"]["url"].as_str().expect("url").contains("{base}"));
    fixture
}

fn organism_contracts(fixture: &Json) -> Vec<ExchangeContract> {
    fixture["exchanges"]
        .as_array()
        .expect("exchanges")
        .iter()
        .map(|e| {
            let ms = if e["request"]["url"].as_str().expect("url").contains("api.deepseek.com") {
                MODEL_TIMEOUT_MS
            } else {
                HTTP_TIMEOUT_MS
            };
            ExchangeContract {
                timeout: Deadline::Exact(Some(std::time::Duration::from_millis(ms))),
                ..Default::default()
            }
        })
        .collect()
}

// The organisms compose std atoms (`Alias.traits.X`); the lowered file leaves those as references, so the program is
// resolved the way `orb compile` resolves it from the package directory before the kernel registers it.
fn resolved_organism(spec: &ParitySpec) -> orbital_core::OrbitalSchema {
    orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(spec.atom);
    let mut options = orbital_compiler::CompilerOptions::from_env();
    let start = path.parent().expect("atom dir").to_path_buf();
    let user_store = orbital_core::behavior_packages::user_store_dir().unwrap_or_else(|| start.clone());
    options.inline.loader_config.packages =
        orbital_core::behavior_packages::discover(&start, &user_store).expect("behavior package discovery");
    let schema = orbital_compiler::phases::parse_file(&path).expect("parse");
    orbital_compiler::phases::resolve_compiled_program_with_path(schema, &options, Some(&path)).expect("resolve")
}

fn organism_session<'a>(spec: &'a ParitySpec, fixture: &Json) -> ParitySession<'a> {
    ParitySession::with_schema(spec, resolved_organism(spec), fixture, FixtureAuthority::default())
}

fn organism_check(session: &mut ParitySession<'_>, fixture: &Json) {
    session.check(fixture, ReplayStep { exchanges: organism_contracts(fixture), ..Default::default() });
}

fn organism_replay(spec: &ParitySpec, case: &str) {
    let fixture = without_cache_exchanges(knowledge_fixture(case));
    let mut session = organism_session(spec, &fixture);
    organism_check(&mut session, &fixture);
}

fn organism_stage(session: &mut ParitySession<'_>, fixture: &Json) {
    session.run(fixture, ReplayStep { exchanges: organism_contracts(fixture), ..Default::default() });
}

fn replace_text(fixture: &Json, from: &str, to: &str) -> Json {
    serde_json::from_str(&fixture.to_string().replace(from, to)).expect("fixture json")
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn every_recorded_concept_icon_miss_is_replayed_by_the_organism() {
    for case in [
        "concept-icon-blank",
        "concept-icon-devicon",
        "concept-icon-found-by-search",
        "concept-icon-nothing-found-is-cached",
    ] {
        organism_replay(&ICON_SPEC, case);
    }
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_cached_concept_icon_is_answered_without_a_request() {
    let found = without_cache_exchanges(knowledge_fixture("concept-icon-found-by-search"));
    let cached = without_cache_exchanges(knowledge_fixture("concept-icon-cached"));
    assert!(cached["exchanges"].as_array().expect("exchanges").is_empty());
    let mut session = organism_session(&ICON_SPEC, &found);
    organism_stage(&mut session, &found);
    organism_check(&mut session, &cached);
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_cached_miss_answers_an_empty_icon_without_a_request() {
    let nothing = replace_text(
        &without_cache_exchanges(knowledge_fixture("concept-icon-nothing-found-is-cached")),
        "zzzz",
        "zzz",
    );
    let nothing = replace_text(&nothing, "Zzzz", "Zzz");
    let cached = without_cache_exchanges(knowledge_fixture("concept-icon-cached-miss"));
    assert!(cached["exchanges"].as_array().expect("exchanges").is_empty());
    let mut session = organism_session(&ICON_SPEC, &nothing);
    organism_stage(&mut session, &nothing);
    organism_check(&mut session, &cached);
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn every_recorded_concept_persona_miss_is_replayed_by_the_organism() {
    for case in [
        "concept-persona",
        "concept-persona-echo-then-corrected",
        "concept-persona-stays-degenerate-uses-a-tutor",
        "concept-persona-wikipedia-miss",
        "concept-persona-label-is-an-id",
    ] {
        organism_replay(&PERSONA_SPEC, case);
    }
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_cached_persona_is_answered_from_its_stored_bio_without_a_request() {
    let mut miss = without_cache_exchanges(knowledge_fixture("concept-persona-wikipedia-miss"));
    let draft = json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "deepseek-v4-flash",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": json!({
            "name": "Isaac Newton", "description": "English physicist.", "greeting": "Hi"
        }).to_string() }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4 }
    });
    miss["exchanges"][0]["response"]["body"] = json!(draft.to_string());
    miss["exchanges"][1]["response"] = json!({
        "status": 200,
        "body": json!({ "query": { "pages": { "1": { "title": "Isaac Newton", "description": "English physicist.", "extract": "Born 1643." } } } }).to_string()
    });
    let cached = without_cache_exchanges(knowledge_fixture("concept-persona-cached"));
    assert!(cached["exchanges"].as_array().expect("exchanges").is_empty());
    let mut session = organism_session(&PERSONA_SPEC, &miss);
    organism_stage(&mut session, &miss);
    organism_check(&mut session, &cached);
}

#[test]
fn a_drifted_persona_outcome_fails_the_organism_replay() {
    let result = std::panic::catch_unwind(|| {
        let mut fixture = without_cache_exchanges(knowledge_fixture("concept-persona"));
        fixture["outcome"]["data"]["name"] = json!("Albert Einstein");
        let mut session = organism_session(&PERSONA_SPEC, &fixture);
        organism_check(&mut session, &fixture);
    });
    assert!(result.is_err());
}

