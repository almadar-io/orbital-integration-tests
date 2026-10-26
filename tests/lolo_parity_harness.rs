#[path = "common/lolo_parity.rs"]
mod lolo_parity;

mod session_regressions {
    use super::lolo_parity;

    use std::time::Duration;

    use lolo_parity::{
        ActionEvents, Deadline, ExchangeContract, FixtureAuthority, FixtureCredential,
        ParitySession, ParitySpec, RecordedBody, ReplayStep,
    };
    use orbital_core::runtime::http_request::{CredentialSlot, SecretScope};
    use orbital_core::{OrbitalSchema, Value};
    use serde_json::{json, Value as Json};

    const SPEC: ParitySpec = ParitySpec {
        atom: "almadar-std/behaviors/lolo/connect/atoms/std-github.lolo",
        trait_name: "Harness",
        fixtures_dir: "iconify",
        events: &[
            ActionEvents {
                action: "status",
                input: "STATUS",
                result: "RESULT",
                failure: Some("FAILED"),
                data_field: None,
            },
            ActionEvents {
                action: "forget",
                input: "FORGET",
                result: "RESULT",
                failure: Some("FAILED"),
                data_field: None,
            },
            ActionEvents {
                action: "random",
                input: "RANDOM",
                result: "RESULT",
                failure: Some("FAILED"),
                data_field: None,
            },
            ActionEvents {
                action: "request",
                input: "REQUEST",
                result: "RESULT",
                failure: Some("FAILED"),
                data_field: None,
            },
        ],
        env_knobs: &[],
        secret_env: &["TOKEN"],
        compared_headers: &["authorization", "content-type"],
        default_timeout_ms: 1000,
        base: None,
        ..ParitySpec::DEFAULT
    };

    fn schema(request: Json) -> OrbitalSchema {
        serde_json::from_value(json!({
        "name": "Harness",
        "egress": ["https://api.example.test", "https://token.example.test"],
        "orbitals": [{
            "name": "HarnessOrbital",
            "entity": { "name": "HarnessEntity", "persistence": "runtime", "fields": [
                { "name": "id", "type": "string" },
                { "name": "count", "type": "number", "default": 0 }
            ] },
            "traits": [{
                "name": "Harness", "linkedEntity": "HarnessEntity",
                "stateMachine": {
                    "states": [{ "name": "idle", "isInitial": true }],
                    "transitions": [
                        { "from": "idle", "to": "idle", "event": "STATUS", "effects": [
                            ["credential-status", "TOKEN", { "emit": { "success": "RESULT", "failure": "FAILED" } }]
                        ] },
                        { "from": "idle", "to": "idle", "event": "FORGET", "effects": [
                            ["credential-forget", "TOKEN", { "emit": { "success": "RESULT", "failure": "FAILED" } }]
                        ] },
                        { "from": "idle", "to": "idle", "event": "RANDOM", "effects": [
                            ["random-bytes", 2, { "emit": { "success": "RESULT", "failure": "FAILED" } }]
                        ] },
                        { "from": "idle", "to": "idle", "event": "REQUEST", "effects": [request] },
                        { "from": "idle", "to": "idle", "event": "HTTP_OK", "effects": [
                            ["set", "@entity.count", ["+", "@entity.count", 1]],
                            ["emit", "RESULT", { "status": "@payload.status", "body": "@payload.body", "count": "@entity.count" }]
                        ] },
                        { "from": "idle", "to": "idle", "event": "HTTP_BAD", "effects": [
                            ["emit", "FAILED", "@payload"]
                        ] }
                    ]
                }
            }]
        }]
    }))
    .expect("typed harness schema")
    }

    fn http(request: Json) -> Json {
        json!(["http", "POST", "https://api.example.test", "/resource", request,
        { "emit": { "success": "HTTP_OK", "failure": "HTTP_BAD" } }])
    }

    fn fixture(action: &str, exchanges: Vec<Json>, data: Json) -> Json {
        json!({
            "case": "shared-harness-regression", "env": { "TOKEN": "test-app-token" },
            "action": action, "params": { "principal": "payload-impostor" },
            "exchanges": exchanges, "outcome": { "ok": true, "data": data }
        })
    }

    fn exchange(url: &str, headers: Json, body: &str, response_body: &str) -> Json {
        json!({
            "request": { "method": "POST", "url": url, "headers": headers, "body": body },
            "response": { "status": 200, "headers": {}, "body": response_body }
        })
    }

    fn user(id: &str) -> Value {
        serde_json::from_value(json!({ "id": id, "role": "member" })).expect("host user")
    }

    fn credential(name: &str, principal: Option<&str>, value: Option<&str>) -> FixtureCredential {
        FixtureCredential {
            slot: CredentialSlot {
                name: name.to_string(),
                scope: if principal.is_some() {
                    SecretScope::User
                } else {
                    SecretScope::App
                },
            },
            principal: principal.map(str::to_string),
            value: value.map(str::to_string),
            ttl_ms: None,
        }
    }

    fn session(
        schema: OrbitalSchema,
        initial: &Json,
        credentials: Vec<FixtureCredential>,
    ) -> ParitySession<'static> {
        ParitySession::with_schema(
            &SPEC,
            schema,
            initial,
            FixtureAuthority {
                credentials,
                now_ms: Some(1000),
            },
        )
    }

    #[test]
    fn credential_status_uses_fixture_authority_without_app_or_payload_fallback() {
        let present = fixture("status", vec![], json!({ "connected": true }));
        let mut replay = session(
            schema(http(json!({}))),
            &present,
            vec![
                credential("TOKEN", Some("alice"), Some("test-user-token")),
                credential("TOKEN", Some("bob"), None),
                credential("TOKEN", Some("carol"), Some("")),
                credential("TOKEN", Some("payload-impostor"), Some("test-wrong-token")),
            ],
        );
        replay.check(
            &present,
            ReplayStep {
                user: user("alice"),
                ..Default::default()
            },
        );
        let absent = fixture("status", vec![], json!({ "connected": false }));
        replay.check(
            &absent,
            ReplayStep {
                user: user("bob"),
                ..Default::default()
            },
        );
        replay.check(
            &present,
            ReplayStep {
                user: user("carol"),
                ..Default::default()
            },
        );
        let signed_out = replay.run(&present, ReplayStep::default());
        assert_eq!(
            signed_out.failures,
            vec![json!({ "error": "credential-status: nobody is signed in" })]
        );
        assert!(signed_out.results.is_empty());
    }

    #[test]
    fn explicit_app_empty_status_is_false_without_changing_raw_credentials() {
        for (scope, principal, value, connected) in [
            ("app", None, "test-authoritative-token", true),
            ("user", Some("alice"), "", true),
            ("app", None, "", false),
        ] {
            let status = fixture("status", vec![], json!({ "connected": connected }));
            let mut program = serde_json::to_value(schema(http(json!({
                "auth": {
                    "scheme": "bearer",
                    "credential": { "credential": "TOKEN", "scope": scope }
                },
                "timeoutMs": 1000
            }))))
            .expect("schema json");
            program["orbitals"][0]["traits"][0]["stateMachine"]["transitions"][0]["effects"][0]
                [1] = json!({ "credential": "TOKEN", "scope": scope });
            let mut replay = session(
                serde_json::from_value(program).expect("scoped status schema"),
                &status,
                vec![credential("TOKEN", principal, Some(value))],
            );
            let viewer = principal.map(user).unwrap_or_default();
            replay.check(
                &status,
                ReplayStep {
                    user: viewer.clone(),
                    ..Default::default()
                },
            );
            let request = fixture(
                "request",
                vec![exchange(
                    "https://api.example.test/resource",
                    json!({ "authorization": "<redacted>" }),
                    "",
                    "ok",
                )],
                json!({ "status": 200, "body": "ok", "count": 1 }),
            );
            let run = replay.check(
                &request,
                ReplayStep {
                    user: viewer,
                    ..Default::default()
                },
            );
            assert!(run.sent[0]
                .headers
                .contains(&("Authorization".to_string(), format!("Bearer {value}"))));
        }
    }

    #[test]
    fn credential_forget_removes_only_the_explicit_principal() {
        let present = fixture("status", vec![], json!({ "connected": true }));
        let mut replay = session(
            schema(http(json!({}))),
            &present,
            vec![
                credential("TOKEN", Some("alice"), Some("test-alice")),
                credential("TOKEN", Some("bob"), Some("test-bob")),
            ],
        );
        replay.check(
            &fixture("forget", vec![], json!({ "connected": false })),
            ReplayStep {
                user: user("alice"),
                ..Default::default()
            },
        );
        replay.check(
            &fixture("status", vec![], json!({ "connected": false })),
            ReplayStep {
                user: user("alice"),
                ..Default::default()
            },
        );
        replay.check(
            &present,
            ReplayStep {
                user: user("bob"),
                ..Default::default()
            },
        );
    }

    #[test]
    fn seeded_credential_expiry_is_inclusive_and_uses_the_session_clock() {
        let present = fixture("status", vec![], json!({ "connected": true }));
        let mut seed = credential("TOKEN", Some("alice"), Some("test-expiring"));
        seed.ttl_ms = Some(10);
        let mut replay = session(schema(http(json!({}))), &present, vec![seed]);
        replay.check(
            &present,
            ReplayStep {
                user: user("alice"),
                at_ms: Some(1009),
                ..Default::default()
            },
        );
        replay.check(
            &fixture("status", vec![], json!({ "connected": false })),
            ReplayStep {
                user: user("alice"),
                at_ms: Some(1010),
                ..Default::default()
            },
        );
    }

    #[test]
    fn entropy_is_explicit_ordered_and_exhausted() {
        let first = fixture("random", vec![], json!({ "value": "-_8" }));
        let mut program = serde_json::to_value(schema(http(json!({})))).expect("schema json");
        let effects = program["orbitals"][0]["traits"][0]["stateMachine"]["transitions"][2]
            ["effects"]
            .as_array_mut()
            .expect("random effects");
        effects.push(effects[0].clone());
        let mut replay = session(
            serde_json::from_value(program).expect("schema"),
            &first,
            vec![],
        );
        let first_run = replay.run(
            &first,
            ReplayStep {
                entropy: vec![vec![0xfb, 0xff], vec![0, 1]],
                ..Default::default()
            },
        );
        assert_eq!(
            first_run.results,
            vec![json!({ "value": "-_8" }), json!({ "value": "AAE" })]
        );
        assert_eq!(first_run.unused_entropy, 0);
        let second_run = replay.run(
            &first,
            ReplayStep {
                entropy: vec![vec![0, 1], vec![0xfb, 0xff]],
                ..Default::default()
            },
        );
        assert_eq!(
            second_run.results,
            vec![json!({ "value": "AAE" }), json!({ "value": "-_8" })]
        );
        assert_eq!(second_run.unused_entropy, 0);
    }

    #[test]
    #[should_panic(expected = "unrecorded entropy")]
    fn missing_entropy_never_falls_back_to_the_os_rng() {
        let f = fixture("random", vec![], json!({ "value": "-_8" }));
        session(schema(http(json!({}))), &f, vec![]).run(&f, ReplayStep::default());
    }

    #[test]
    #[should_panic(expected = "entropy byte count")]
    fn entropy_length_must_match_the_actual_host_ask() {
        let f = fixture("random", vec![], json!({ "value": "-_8" }));
        session(schema(http(json!({}))), &f, vec![]).run(
            &f,
            ReplayStep {
                entropy: vec![vec![0xff]],
                ..Default::default()
            },
        );
    }

    #[test]
    #[should_panic(expected = "unused entropy")]
    fn unused_entropy_is_a_failed_recording() {
        let f = fixture("random", vec![], json!({ "value": "-_8" }));
        session(schema(http(json!({}))), &f, vec![]).check(
            &f,
            ReplayStep {
                entropy: vec![vec![0xfb, 0xff], vec![0, 1]],
                ..Default::default()
            },
        );
    }

    fn binary_fixture() -> Json {
        fixture(
            "request",
            vec![exchange(
                "https://api.example.test/resource",
                json!({ "content-type": "application/octet-stream" }),
                "�",
                "�",
            )],
            json!({ "status": 200, "body": "/w==", "count": 1 }),
        )
    }

    fn binary_contract(bytes: Vec<u8>) -> ExchangeContract {
        ExchangeContract {
            request_body: RecordedBody::Bytes(bytes),
            response_body: RecordedBody::Bytes(vec![0xff]),
            timeout: Deadline::Exact(Some(Duration::from_millis(1234))),
        }
    }

    #[test]
    fn exact_binary_bodies_and_per_exchange_deadlines_replay() {
        let f = binary_fixture();
        session(
            schema(http(
                json!({ "bytes": "/w==", "responseAs": "bytes", "timeoutMs": 1234 }),
            )),
            &f,
            vec![],
        )
        .check(
            &f,
            ReplayStep {
                exchanges: vec![binary_contract(vec![0xff])],
                ..Default::default()
            },
        );
    }

    #[test]
    #[should_panic(expected = "request body bytes")]
    fn two_invalid_utf8_bodies_with_the_same_lossy_text_are_different() {
        let f = binary_fixture();
        session(
            schema(http(
                json!({ "bytes": "/g==", "responseAs": "bytes", "timeoutMs": 1234 }),
            )),
            &f,
            vec![],
        )
        .check(
            &f,
            ReplayStep {
                exchanges: vec![binary_contract(vec![0xff])],
                ..Default::default()
            },
        );
    }

    #[test]
    #[should_panic(expected = "timeout")]
    fn a_drifted_per_exchange_deadline_fails() {
        let f = binary_fixture();
        session(
            schema(http(
                json!({ "bytes": "/w==", "responseAs": "bytes", "timeoutMs": 1235 }),
            )),
            &f,
            vec![],
        )
        .check(
            &f,
            ReplayStep {
                exchanges: vec![binary_contract(vec![0xff])],
                ..Default::default()
            },
        );
    }

    #[test]
    #[should_panic(expected = "exchange contract count")]
    fn an_out_of_range_exchange_override_fails() {
        let f = binary_fixture();
        session(
            schema(http(
                json!({ "bytes": "/w==", "responseAs": "bytes", "timeoutMs": 1234 }),
            )),
            &f,
            vec![],
        )
        .run(
            &f,
            ReplayStep {
                exchanges: vec![binary_contract(vec![0xff]), binary_contract(vec![0xff])],
                ..Default::default()
            },
        );
    }

    fn token_request() -> Json {
        http(json!({
            "auth": {
                "scheme": "token-exchange", "tokenUrl": "https://token.example.test/token",
                "body": { "form": { "refresh_token": { "credential": "TOKEN", "scope": "user" } } },
                "storeAs": { "name": "CACHE", "scope": "user" }
            },
            "timeoutMs": 1000
        }))
    }

    fn token_fixture(grant: bool, count: u64, token: &str) -> Json {
        let mut exchanges = Vec::new();
        if grant {
            exchanges.push(exchange(
                "https://token.example.test/token",
                json!({ "content-type": "application/x-www-form-urlencoded" }),
                "refresh_token=test-refresh",
                &format!("{{\"access_token\":\"{token}\",\"expires_in\":2}}"),
            ));
        }
        exchanges.push(exchange(
            "https://api.example.test/resource",
            json!({ "authorization": "<redacted>" }),
            "",
            "ok",
        ));
        fixture(
            "request",
            exchanges,
            json!({ "status": 200, "body": "ok", "count": count }),
        )
    }

    fn token_step(grant: bool, principal: &str, at_ms: u64) -> ReplayStep {
        let mut exchanges = Vec::new();
        if grant {
            exchanges.push(ExchangeContract {
                timeout: Deadline::Exact(None),
                ..Default::default()
            });
        }
        exchanges.push(ExchangeContract::default());
        ReplayStep {
            user: user(principal),
            at_ms: Some(at_ms),
            exchanges,
            ..Default::default()
        }
    }

    #[test]
    fn secret_set_survives_dispatches_is_scoped_and_expires_at_the_boundary() {
        let first = token_fixture(true, 1, "test-first");
        let mut replay = session(
            schema(token_request()),
            &first,
            vec![
                credential("TOKEN", Some("alice"), Some("test-refresh")),
                credential("TOKEN", Some("bob"), Some("test-refresh")),
                credential("CACHE", Some("alice"), None),
                credential("CACHE", Some("bob"), None),
            ],
        );
        replay.check(&first, token_step(true, "alice", 1000));
        let cached = replay.check(
            &token_fixture(false, 2, "unused"),
            token_step(false, "alice", 2999),
        );
        assert_eq!(
            cached.results,
            vec![json!({ "status": 200, "body": "ok", "count": 2 })]
        );
        assert!(cached.sent[0]
            .headers
            .contains(&("Authorization".to_string(), "Bearer test-first".to_string())));
        replay.check(
            &token_fixture(true, 3, "test-bob"),
            token_step(true, "bob", 2999),
        );
        replay.check(
            &token_fixture(true, 4, "test-renewed"),
            token_step(true, "alice", 3000),
        );
    }

    #[test]
    fn zero_lifetime_tokens_are_never_reused() {
        let mut first = token_fixture(true, 1, "test-zero");
        first["exchanges"][0]["response"]["body"] =
            json!(json!({ "access_token": "test-zero", "expires_in": 0 }).to_string());
        let mut replay = session(
            schema(token_request()),
            &first,
            vec![
                credential("TOKEN", Some("alice"), Some("test-refresh")),
                credential("CACHE", Some("alice"), None),
            ],
        );
        replay.check(&first, token_step(true, "alice", 1000));
        let mut second = first.clone();
        second["outcome"]["data"]["count"] = json!(2);
        replay.check(&second, token_step(true, "alice", 1000));
    }

    #[test]
    #[should_panic(expected = "undeclared credential")]
    fn a_user_credential_requires_an_explicit_fixture_declaration() {
        let f = fixture("status", vec![], json!({ "connected": true }));
        session(schema(http(json!({}))), &f, vec![]).run(
            &f,
            ReplayStep {
                user: user("alice"),
                ..Default::default()
            },
        );
    }

    fn rotating_request() -> Json {
        http(json!({
            "auth": {
                "scheme": "token-exchange", "tokenUrl": "https://token.example.test/token", "grant": "refresh_token",
                "body": { "form": { "refresh_token": { "credential": "TOKEN", "scope": "user" } } },
                "rotate": { "field": "refresh_token", "credential": { "credential": "TOKEN", "scope": "user" } },
                "storeAs": { "name": "CACHE", "scope": "user" }
            },
            "timeoutMs": 1000
        }))
    }

    fn rotating_fixture(sent_refresh: &str, rotated: &str, count: u64) -> Json {
        fixture(
            "request",
            vec![
                exchange(
                    "https://token.example.test/token",
                    json!({ "content-type": "application/x-www-form-urlencoded" }),
                    &format!("grant_type=refresh_token&refresh_token={sent_refresh}"),
                    &format!("{{\"access_token\":\"test-access\",\"expires_in\":2,\"refresh_token\":\"{rotated}\"}}"),
                ),
                exchange("https://api.example.test/resource", json!({ "authorization": "<redacted>" }), "", "ok"),
            ],
            json!({ "status": 200, "body": "ok", "count": count }),
        )
    }

    #[test]
    fn a_rotated_refresh_token_is_written_durably_and_used_by_the_next_grant() {
        let first = rotating_fixture("test-refresh", "test-refresh-2", 1);
        let mut replay = session(
            schema(rotating_request()),
            &first,
            vec![
                credential("TOKEN", Some("alice"), Some("test-refresh")),
                credential("CACHE", Some("alice"), None),
            ],
        );
        replay.check(&first, token_step(true, "alice", 1000));
        assert_eq!(replay.held_credential("TOKEN", Some("alice")).as_deref(), Some("test-refresh-2"));
        assert_eq!(replay.rotations(), vec![("TOKEN".to_string(), Some("alice".to_string()))]);
        let second = rotating_fixture("test-refresh-2", "test-refresh-3", 2);
        replay.check(&second, token_step(true, "alice", 3000));
        assert_eq!(replay.held_credential("TOKEN", Some("alice")).as_deref(), Some("test-refresh-3"));
        assert_eq!(replay.rotations().len(), 2);
    }

    #[test]
    #[should_panic(expected = "request body bytes")]
    fn a_grant_that_still_sends_the_unrotated_token_fails_the_replay() {
        let first = rotating_fixture("test-refresh", "test-refresh-2", 1);
        let mut replay = session(
            schema(rotating_request()),
            &first,
            vec![
                credential("TOKEN", Some("alice"), Some("test-refresh")),
                credential("CACHE", Some("alice"), None),
            ],
        );
        replay.check(&first, token_step(true, "alice", 1000));
        let stale = rotating_fixture("test-refresh", "test-refresh-3", 2);
        replay.check(&stale, token_step(true, "alice", 3000));
    }

    #[test]
    fn allowed_secret_values_are_opt_in_and_the_default_stays_strict() {
        lolo_parity::assert_no_secrets(&SPEC);
        lolo_parity::assert_no_secrets_allowing(&SPEC, &["test-anything"]);
    }

    #[test]
    #[should_panic(expected = "an allowed test secret appears in the atom")]
    fn an_allowed_secret_value_that_is_in_the_atom_source_fails() {
        lolo_parity::assert_no_secrets_allowing(&SPEC, &["secret"]);
    }
}

use orbital_core::runtime::http_request::{CredentialSlot, SecretScope};
use orbital_core::runtime::http_service::HttpRequest;
use orbital_core::runtime::resumable::{IoAnswer, IoRequest};
use orbital_core::OrbitalSchema;
use serde_json::json;

const LEGACY_SPEC: lolo_parity::ParitySpec = lolo_parity::ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/atoms/std-iconify.lolo",
    trait_name: "Harness",
    fixtures_dir: "iconify",
    events: &[],
    env_knobs: &[],
    secret_env: &["TOKEN"],
    compared_headers: &[],
    default_timeout_ms: 1000,
    base: None,
    ..lolo_parity::ParitySpec::DEFAULT
};

fn legacy_fixture(body: &str) -> serde_json::Value {
    json!({ "case": "legacy-harness-regression", "env": { "TOKEN": "test-original" },
        "exchanges": [{
            "request": { "method": "POST", "url": "https://api.example.test", "headers": {}, "body": body },
            "response": { "status": 200, "body": "ok", "headers": {} }
        }]
    })
}

fn request(body: Vec<u8>) -> IoRequest {
    IoRequest::Http {
        request: HttpRequest {
            method: "POST".into(),
            url: "https://api.example.test".into(),
            headers: vec![],
            body,
            timeout: Some(std::time::Duration::from_millis(1000)),
        },
        key: None,
        delay_ms: None,
        site: None,
    }
}

#[test]
#[should_panic(expected = "request body bytes")]
fn existing_harness_rejects_lossy_binary_equivalence() {
    lolo_parity::probe_fixture_host(&LEGACY_SPEC, &legacy_fixture("�"), &[request(vec![0xff])]);
}

#[test]
fn existing_harness_accepts_identical_utf8_bytes() {
    let answers = lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &legacy_fixture("ok"),
        &[request(b"ok".to_vec())],
    );
    assert!(matches!(&answers[0], IoAnswer::Http(Ok(_))));
}

#[test]
fn existing_harness_retains_secret_set_for_the_next_read() {
    let answers = lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &legacy_fixture(""),
        &[
            IoRequest::SecretSet {
                name: "TOKEN".into(),
                value: "test-minted".into(),
                ttl_ms: 60_000,
                principal: None,
            },
            IoRequest::Secret {
                name: "TOKEN".into(),
                principal: None,
            },
            IoRequest::SecretSet {
                name: "TOKEN".into(),
                value: String::new(),
                ttl_ms: 60_000,
                principal: None,
            },
            IoRequest::Secret {
                name: "TOKEN".into(),
                principal: None,
            },
            IoRequest::SecretSet {
                name: "TOKEN".into(),
                value: String::new(),
                ttl_ms: 0,
                principal: None,
            },
            IoRequest::Secret {
                name: "TOKEN".into(),
                principal: None,
            },
        ],
    );
    assert_eq!(answers[1], IoAnswer::Secret(Ok(Some("test-minted".into()))));
    assert_eq!(answers[3], IoAnswer::Secret(Ok(Some(String::new()))));
    assert_eq!(
        answers[5],
        IoAnswer::Secret(Ok(Some("test-original".into())))
    );
}

#[test]
fn existing_harness_reads_the_declared_fixture_secret() {
    let answers = lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &legacy_fixture(""),
        &[IoRequest::Secret {
            name: "TOKEN".into(),
            principal: None,
        }],
    );
    assert_eq!(
        answers[0],
        IoAnswer::Secret(Ok(Some("test-original".into())))
    );
    let explicit_empty = lolo_parity::probe_fixture_host_with_authority(
        &LEGACY_SPEC,
        &legacy_fixture(""),
        lolo_parity::FixtureAuthority {
            credentials: vec![lolo_parity::FixtureCredential {
                slot: orbital_core::runtime::http_request::CredentialSlot::from("TOKEN"),
                principal: None,
                value: Some(String::new()),
                ttl_ms: None,
            }],
            now_ms: None,
        },
        &[IoRequest::Secret {
            name: "TOKEN".into(),
            principal: None,
        }],
    );
    assert_eq!(explicit_empty[0], IoAnswer::Secret(Ok(Some(String::new()))));
}

#[test]
fn existing_harness_answers_credential_status_from_the_same_authority() {
    let answers = lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &legacy_fixture(""),
        &[IoRequest::CredentialStatus {
            name: "TOKEN".into(),
            principal: None,
        }],
    );
    assert_eq!(answers[0], IoAnswer::CredentialStatus(Ok(orbital_core::credential_admin::CredentialStanding::of(true))));
}

#[test]
#[should_panic(expected = "undeclared credential")]
fn existing_harness_cannot_answer_a_user_secret_from_app_env() {
    lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &legacy_fixture(""),
        &[IoRequest::Secret {
            name: "TOKEN".into(),
            principal: Some("alice".into()),
        }],
    );
}

#[test]
fn existing_harness_treats_an_empty_credential_as_absent() {
    let mut f = legacy_fixture("");
    f["env"]["TOKEN"] = json!("");
    let answers = lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &f,
        &[IoRequest::Secret {
            name: "TOKEN".into(),
            principal: None,
        }],
    );
    assert_eq!(answers[0], IoAnswer::Secret(Ok(None)));
}

fn beacon(body: &[u8]) -> IoRequest {
    IoRequest::HttpBeacon {
        request: HttpRequest {
            method: "POST".into(),
            url: "https://api.example.test".into(),
            headers: vec![],
            body: body.to_vec(),
            timeout: Some(std::time::Duration::from_millis(1000)),
        },
        key: None,
        site: None,
    }
}

#[test]
fn beacon_fetches_against_the_recorded_exchange_and_reports_its_status() {
    let answers = lolo_parity::probe_fixture_host(&LEGACY_SPEC, &legacy_fixture("ok"), &[beacon(b"ok")]);
    assert!(matches!(
        &answers[0],
        IoAnswer::HttpBeacon(Ok(orbital_core::runtime::http_request::BeaconOutcome::Fetched { status: 200 }))
    ));
}

#[test]
fn beacon_queue_fixture_is_accepted_by_the_queue_and_replays_no_exchange() {
    let mut fixture = legacy_fixture("ok");
    fixture["beacon"] = json!("queue");
    let answers = lolo_parity::probe_fixture_host(&LEGACY_SPEC, &fixture, &[beacon(b"ok")]);
    assert!(matches!(
        &answers[0],
        IoAnswer::HttpBeacon(Ok(orbital_core::runtime::http_request::BeaconOutcome::Queued))
    ));
}

#[test]
#[should_panic(expected = "request body bytes")]
fn beacon_fetch_with_a_drifted_body_fails_the_replay() {
    lolo_parity::probe_fixture_host(&LEGACY_SPEC, &legacy_fixture("ok"), &[beacon(b"drifted")]);
}

#[test]
#[should_panic(expected = "`beacon` must be")]
fn beacon_fixture_key_is_a_closed_choice() {
    let mut fixture = legacy_fixture("ok");
    fixture["beacon"] = json!("fetch");
    lolo_parity::probe_fixture_host(&LEGACY_SPEC, &fixture, &[beacon(b"ok")]);
}

fn facts(fixture: &serde_json::Value, fields: Vec<orbital_core::ir::PageFactField>) -> orbital_core::runtime::resumable::PageFacts {
    match lolo_parity::probe_fixture_host(&LEGACY_SPEC, fixture, &[IoRequest::PageFacts { fields }]).remove(0) {
        IoAnswer::PageFacts(Ok(facts)) => facts,
        other => panic!("{other:?}"),
    }
}

#[test]
fn beacon_page_facts_answer_only_the_requested_fields_from_the_fixture_page() {
    use orbital_core::ir::PageFactField::{Hostname, Referrer, ScreenWidth};
    let mut fixture = legacy_fixture("ok");
    fixture["page"] = json!({ "hostname": "orb.almadar.io", "referrer": "https://news.ycombinator.com/", "screenWidth": 1440 });
    let both = facts(&fixture, vec![Referrer, ScreenWidth]);
    assert!(both.available);
    assert_eq!((both.hostname.as_str(), both.referrer.as_str(), both.screen_width), ("", "https://news.ycombinator.com/", Some(1440.0)));
    let host = facts(&fixture, vec![Hostname]);
    assert_eq!((host.hostname.as_str(), host.referrer.as_str(), host.screen_width), ("orb.almadar.io", "", None));
}

#[test]
fn beacon_page_facts_without_a_fixture_page_are_the_no_page_defaults() {
    let none = facts(&legacy_fixture("ok"), vec![orbital_core::ir::PageFactField::Hostname]);
    assert!(!none.available);
    assert_eq!((none.hostname.as_str(), none.referrer.as_str(), none.screen_width), ("", "", None));
}

#[test]
fn oauth_asks_are_answered_by_one_store_on_the_fixture_clock() {
    use orbital_core::oauth::ask::{OAuthAsk, OAuthReply};
    let ask = |ask: OAuthAsk, principal: Option<&str>| IoRequest::OAuth { ask, principal: principal.map(str::to_string) };
    let mut fixture = legacy_fixture("ok");
    fixture["now"] = json!("2027-01-15T08:00:00Z");
    let answers = lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &fixture,
        &[
            ask(OAuthAsk::Clock, None),
            ask(OAuthAsk::SubjectRemember { access_token: "at-1".into(), subject: "user-1".into() }, None),
            ask(OAuthAsk::SubjectLookup { access_token: "at-1".into() }, None),
            ask(OAuthAsk::SubjectLookup { access_token: "never-seen".into() }, None),
            ask(OAuthAsk::ConnectionStatus { provider: orbital_core::oauth::ask::OAuthProvider::Google }, None),
        ],
    );
    let expected = chrono::DateTime::parse_from_rfc3339("2027-01-15T08:00:00Z").unwrap().timestamp_millis();
    assert!(matches!(&answers[0], IoAnswer::OAuth(Ok(OAuthReply::Millis { value })) if *value == expected));
    assert!(matches!(&answers[2], IoAnswer::OAuth(Ok(OAuthReply::Subject { subject: Some(s) })) if s == "user-1"));
    assert!(matches!(&answers[3], IoAnswer::OAuth(Ok(OAuthReply::Subject { subject: None }))), "control: an unknown token has no subject");
    assert!(matches!(&answers[4], IoAnswer::OAuth(Err(_))), "control: a connection needs a signed-in caller");
}

#[test]
fn oauth_store_state_survives_between_asks_of_one_session() {
    use orbital_core::oauth::ask::{OAuthAsk, OAuthReply};
    let one = |token: &str| IoRequest::OAuth { ask: OAuthAsk::SubjectLookup { access_token: token.into() }, principal: None };
    let remember = IoRequest::OAuth {
        ask: OAuthAsk::SubjectRemember { access_token: "at".into(), subject: "u".into() },
        principal: None,
    };
    let together = lolo_parity::probe_fixture_host(&LEGACY_SPEC, &legacy_fixture("ok"), &[remember, one("at")]);
    assert!(matches!(&together[1], IoAnswer::OAuth(Ok(OAuthReply::Subject { subject: Some(_) }))));
    let apart = lolo_parity::probe_fixture_host(&LEGACY_SPEC, &legacy_fixture("ok"), &[one("at")]);
    assert!(matches!(&apart[0], IoAnswer::OAuth(Ok(OAuthReply::Subject { subject: None }))), "a fresh host has no state");
}

#[test]
fn steps_tokens_match_generated_values_of_their_shape_only() {
    use lolo_parity::matches_recorded;
    assert!(matches_recorded("state=<state>&x=1", "state=Ab-_9&x=1"));
    assert!(!matches_recorded("state=<state>&x=1", "state=&x=1"), "a token matches at least one character");
    assert!(!matches_recorded("state=<state>&x=1", "state=a b&x=1"));
    assert!(matches_recorded("<hex8>", "0a1b2c3d"));
    assert!(!matches_recorded("<hex8>", "0a1b2c3"));
    assert!(!matches_recorded("<hex8>", "0A1B2C3D"), "lowercase only");
    assert!(!matches_recorded("<hex8>", "0a1b2c3dd"));
    assert!(matches_recorded("user-<hex8>@google.example.com", "user-0a1b2c3d@google.example.com"));
    assert!(matches_recorded("<redacted>", "<redacted>"));
    assert!(!matches_recorded("<redacted>", "secret"), "<redacted> is compared exactly");
    assert!(matches_recorded("plain", "plain") && !matches_recorded("plain", "plainer"));
}

const STEPS_SPEC: lolo_parity::ParitySpec = lolo_parity::ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/atoms/std-iconify.lolo",
    trait_name: "Harness",
    fixtures_dir: "iconify",
    events: &[
        lolo_parity::ActionEvents { action: "status", input: "STATUS", result: "RESULT", failure: Some("FAILED"), data_field: None },
        lolo_parity::ActionEvents { action: "forget", input: "FORGET", result: "RESULT", failure: Some("FAILED"), data_field: None },
        lolo_parity::ActionEvents { action: "random", input: "RANDOM", result: "RESULT", failure: Some("FAILED"), data_field: None },
    ],
    env_knobs: &[],
    secret_env: &["TOKEN"],
    compared_headers: &[],
    default_timeout_ms: 1000,
    base: None,
    ..lolo_parity::ParitySpec::DEFAULT
};

fn steps_scenario(steps: Vec<serde_json::Value>) -> serde_json::Value {
    json!({ "service": "iconify", "case": "steps-regression", "nowMs": 1000, "env": { "TOKEN": "test-app-token" }, "steps": steps })
}

fn credential_step(action: &str, connected: bool) -> serde_json::Value {
    json!({ "action": action, "params": {}, "exchanges": [], "user": { "id": "alice", "role": "member" },
        "outcome": { "ok": true, "data": { "connected": connected } } })
}

fn steps_schema() -> OrbitalSchema {
    let effect = |name: &str| json!([[name, "TOKEN", { "emit": { "success": "RESULT", "failure": "FAILED" } }]]);
    serde_json::from_value(json!({
        "name": "Harness", "egress": [],
        "orbitals": [{ "name": "O", "entity": { "name": "E", "persistence": "runtime", "fields": [{ "name": "id", "type": "string" }] },
            "traits": [{ "name": "Harness", "linkedEntity": "E", "stateMachine": {
                "states": [{ "name": "idle", "isInitial": true }],
                "transitions": [
                    { "from": "idle", "to": "idle", "event": "STATUS", "effects": effect("credential-status") },
                    { "from": "idle", "to": "idle", "event": "FORGET", "effects": effect("credential-forget") },
                    { "from": "idle", "to": "idle", "event": "RANDOM", "effects": [["random-bytes", 2, { "emit": { "success": "RESULT", "failure": "FAILED" } }]] }
                ] } }] }]
    }))
    .expect("steps schema")
}

fn alice_session(scenario: &serde_json::Value, ttl_ms: Option<u64>) -> lolo_parity::ParitySession<'static> {
    lolo_parity::ParitySession::with_schema(
        &STEPS_SPEC,
        steps_schema(),
        scenario,
        lolo_parity::FixtureAuthority {
            credentials: vec![lolo_parity::FixtureCredential {
                slot: CredentialSlot { name: "TOKEN".into(), scope: SecretScope::User },
                principal: Some("alice".into()),
                value: Some("test-user-token".into()),
                ttl_ms,
            }],
            now_ms: Some(1000),
        },
    )
}

#[test]
fn steps_replay_in_order_on_one_stateful_session() {
    // status, forget, status: the second status only sees "disconnected" because the forget carried over.
    let scenario = steps_scenario(vec![credential_step("status", true), credential_step("forget", false), credential_step("status", false)]);
    let runs = alice_session(&scenario, None).check_steps(&scenario);
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[2].1.results, vec![json!({ "connected": false })]);
}

#[test]
#[should_panic(expected = "emitted result")]
fn steps_control_a_step_that_expects_the_unforgotten_state_fails() {
    let scenario = steps_scenario(vec![credential_step("status", true), credential_step("forget", false), credential_step("status", true)]);
    alice_session(&scenario, None).check_steps(&scenario);
}

#[test]
fn steps_placeholders_take_earlier_emitted_fields_and_randoms_feed_entropy() {
    let scenario = steps_scenario(vec![
        json!({ "action": "random", "params": {}, "exchanges": [], "randoms": [[1, 2]], "outcome": { "ok": true, "data": { "value": "<state>" } } }),
        json!({ "action": "random", "params": { "echo": "{{0.value}}", "inline": "v={{0.value}}!" }, "exchanges": [], "randoms": ["AwQ="],
            "outcome": { "ok": true, "data": { "value": "AwQ" } } }),
    ]);
    let runs = alice_session(&scenario, None).check_steps(&scenario);
    assert_eq!(runs[0].1.results, vec![json!({ "value": "AQI" })]);
    assert_eq!(runs[1].0["params"]["echo"], "AQI");
    assert_eq!(runs[1].0["params"]["inline"], "v=AQI!");
}

#[test]
#[should_panic(expected = "names a step that has not run")]
fn steps_control_a_placeholder_for_a_later_step_panics() {
    let mut step = credential_step("status", true);
    step["params"] = json!({ "x": "{{1.value}}" });
    let scenario = steps_scenario(vec![step]);
    alice_session(&scenario, None).run_steps(&scenario);
}

#[test]
fn steps_advance_ms_moves_the_clock_on_from_the_previous_step() {
    // the credential lives 200 ms from t=1000: live at 1000, gone after advancing 500
    let mut scenario = steps_scenario(vec![credential_step("status", true), credential_step("status", false)]);
    scenario["steps"][1]["advanceMs"] = json!(500);
    alice_session(&scenario, Some(200)).check_steps(&scenario);
    // control: without the advance the credential is still live at the second step
    let mut still_live = steps_scenario(vec![credential_step("status", true), credential_step("status", true)]);
    still_live["steps"][1]["advanceMs"] = json!(0);
    alice_session(&still_live, Some(200)).check_steps(&still_live);
}

fn os_ask(op: orbital_core::runtime::os::OsOp, args: Vec<orbital_core::Value>) -> IoRequest {
    IoRequest::Os { op, args, key: None, site: None }
}

fn os_fixture(os_exchanges: serde_json::Value) -> serde_json::Value {
    let mut fixture = legacy_fixture("");
    fixture["exchanges"] = json!([]);
    fixture["osExchanges"] = os_exchanges;
    fixture
}

fn os_send_exchange(data: &str, answer: serde_json::Value) -> serde_json::Value {
    json!({ "os": { "op": "os/write", "args": ["conn-1", data] }, "answer": answer })
}

#[test]
fn os_asks_replay_in_order_with_the_recorded_answers() {
    use orbital_core::runtime::os::OsOp;
    use orbital_core::Value;
    let fixture = os_fixture(json!([
        os_send_exchange("PING", json!({ "ok": 4 })),
        { "os": { "op": "os/read", "args": ["conn-1", 16] }, "answer": { "ok": "+PONG\r\n" } },
        os_send_exchange("QUIT", json!({ "error": { "kind": "closed", "message": "gone" } })),
    ]));
    let answers = lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &fixture,
        &[
            os_ask(OsOp::Write, vec![Value::String("conn-1".into()), Value::String("PING".into())]),
            os_ask(OsOp::Read, vec![Value::String("conn-1".into()), Value::Number(16.0)]),
            os_ask(OsOp::Write, vec![Value::String("conn-1".into()), Value::String("QUIT".into())]),
        ],
    );
    assert!(matches!(&answers[0], IoAnswer::Os(Ok(Value::Number(n))) if *n == 4.0));
    assert!(matches!(&answers[1], IoAnswer::Os(Ok(Value::String(s))) if s == "+PONG\r\n"));
    match &answers[2] {
        IoAnswer::Os(Err(e)) => {
            assert_eq!(e.kind, orbital_core::runtime::os::OsErrorKind::Closed);
            assert_eq!(e.message, "gone");
        }
        other => panic!("expected the recorded os error, got {other:?}"),
    }
}

#[test]
fn os_args_match_recorded_tokens() {
    use orbital_core::runtime::os::OsOp;
    use orbital_core::Value;
    let fixture = os_fixture(json!([os_send_exchange("id-<hex8>", json!({ "ok": null }))]));
    let answers = lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &fixture,
        &[os_ask(OsOp::Write, vec![Value::String("conn-1".into()), Value::String("id-0a1b2c3d".into())])],
    );
    assert!(matches!(&answers[0], IoAnswer::Os(Ok(Value::Null))));
}

#[test]
#[should_panic(expected = "args")]
fn os_asks_with_drifted_args_fail_the_replay() {
    use orbital_core::runtime::os::OsOp;
    use orbital_core::Value;
    let fixture = os_fixture(json!([os_send_exchange("PING", json!({ "ok": 4 }))]));
    lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &fixture,
        &[os_ask(OsOp::Write, vec![Value::String("conn-1".into()), Value::String("PONG".into())])],
    );
}

#[test]
#[should_panic(expected = "os op")]
fn os_asks_out_of_order_fail_the_replay() {
    use orbital_core::runtime::os::OsOp;
    use orbital_core::Value;
    let fixture = os_fixture(json!([os_send_exchange("PING", json!({ "ok": 4 }))]));
    lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &fixture,
        &[os_ask(OsOp::Read, vec![Value::String("conn-1".into()), Value::Number(16.0)])],
    );
}

#[test]
#[should_panic(expected = "unrecorded os ask")]
fn an_unrecorded_os_ask_fails_the_replay() {
    use orbital_core::runtime::os::OsOp;
    use orbital_core::Value;
    lolo_parity::probe_fixture_host(
        &LEGACY_SPEC,
        &os_fixture(json!([])),
        &[os_ask(OsOp::Close, vec![Value::String("conn-1".into())])],
    );
}

const KNOB_SPEC: lolo_parity::ParitySpec = lolo_parity::ParitySpec {
    atom: "almadar-std/behaviors/lolo/net/atoms/std-webhook.lolo",
    trait_name: "WebhookClient",
    fixtures_dir: "webhook",
    env_knobs: &[("WEBHOOK_TIMEOUT_MS", "timeoutMs")],
    default_timeout_ms: 10_000,
    ..lolo_parity::ParitySpec::DEFAULT
};

#[test]
fn an_unreadable_numeric_knob_is_a_typed_error_the_spec_can_expect() {
    let fixture = json!({ "case": "unreadable", "env": { "WEBHOOK_TIMEOUT_MS": "soon" } });
    assert_eq!(
        lolo_parity::patched_json(&KNOB_SPEC, &fixture).err(),
        Some(lolo_parity::KnobError::Unreadable {
            case: "unreadable".into(),
            variable: "WEBHOOK_TIMEOUT_MS".into(),
            knob: "timeoutMs".into(),
            raw: "soon".into(),
        })
    );
    let keeps_default = lolo_parity::ParitySpec { unreadable_knob: lolo_parity::UnreadableKnob::UseDefault, ..KNOB_SPEC };
    assert!(lolo_parity::patched_json(&keeps_default, &fixture).is_ok());
    let readable = json!({ "case": "readable", "env": { "WEBHOOK_TIMEOUT_MS": "250" } });
    assert!(lolo_parity::patched_json(&KNOB_SPEC, &readable).is_ok());
}

// Parity for `std-http1.lolo` against the retired `http/listen` + `http/respond` host operators: RECORDED is what
// the operators delivered and wrote for each case (captured from a twin run before they were deleted, 2026-10-20).
mod http1_listen_parity {
    use orbital_core::effects::Effect;
    use orbital_core::runtime::os_host::completion::{CompletionAnswerer, CompletionDone, KernelCompletionSink};
    use orbital_core::runtime::os_host::{answer_os, OsGrants, OsHandles, OsHost};
    use orbital_core::runtime::resumable::{BlockingHost, CallSite, DispatchError, DispatchOutcomes, IoRequest};
    use orbital_core::runtime::{EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value};
    use serde_json::{json, Value as Json};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    // The operator's 411-for-chunked recording is dropped: chunked request bodies are decoded (owner ruling "HTTP scope", S52).
    const RECORDED: &str = r#"[{"record":{"method":"GET","path":"/ok","query":"","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 11\r\nconnection: close\r\n\r\nhello there"},{"record":{"method":"GET","path":"/missing","query":"x=1","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 404 Not Found\r\ncontent-length: 9\r\nconnection: close\r\n\r\nnot found"},{"record":{"method":"GET","path":"/ka","query":"","headers":{"host":"a","connection":"keep-alive"},"body":""},"reply":"HTTP/1.1 200 OK\r\nx-a: 1\r\ncontent-length: 2\r\nconnection: close\r\n\r\nka"},{"record":{"method":"POST","path":"/items/7","query":"x=1&y=two","headers":{"host":"a","x-two":"a, b","content-length":"5"},"body":"aGVsbG8="},"reply":"HTTP/1.1 201 Created\r\ncontent-type: text/plain\r\ncontent-length: 4\r\nconnection: close\r\n\r\nmade"},{"record":{"method":"DELETE","path":"/x","query":"","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n"},{"record":{"method":"GET","path":"/tea","query":"","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 418 I'm a teapot\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"},{"record":{"method":"GET","path":"/full","query":"","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 507 Insufficient Storage\r\ncontent-length: 1\r\nconnection: close\r\n\r\nx"},{"record":null,"reply":"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"},{"record":null,"reply":"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"}]"#;

    const ATOM: &str = "almadar-std/behaviors/lolo/net/atoms/std-http1.lolo";

    struct Case {
        name: &'static str,
        raw: &'static [u8],
        answer: Option<(u16, &'static [(&'static str, &'static str)], &'static str)>,
    }

    const CASES: &[Case] = &[
        Case { name: "GET 200 with a body", raw: b"GET /ok HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((200, &[("content-type", "text/plain")], "hello there")) },
        Case { name: "404", raw: b"GET /missing?x=1 HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((404, &[], "not found")) },
        Case { name: "keep-alive asked, closed anyway", raw: b"GET /ka HTTP/1.1\r\nHost: a\r\nConnection: keep-alive, close\r\n\r\n", answer: Some((200, &[("x-a", "1")], "ka")) },
        Case { name: "POST with content-length body", raw: b"POST /items/7?x=1&y=two HTTP/1.1\r\nHost: a\r\nX-Two: a\r\nx-two: b\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello", answer: Some((201, &[("content-type", "text/plain")], "made")) },
        Case { name: "204 has no content-length", raw: b"DELETE /x HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((204, &[], "")) },
        Case { name: "418 reason phrase", raw: b"GET /tea HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((418, &[], "")) },
        Case { name: "507 reason phrase", raw: b"GET /full HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((507, &[], "x")) },
        Case { name: "malformed head is 400", raw: b"NOT HTTP AT ALL\r\nConnection: close\r\n\r\n", answer: None },
        Case { name: "two different content-lengths is 400", raw: b"POST /d HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\nConnection: close\r\n\r\nab", answer: None },
    ];

    fn v(json: Json) -> Value {
        serde_json::from_value(json).expect("a kernel value")
    }

    fn b64(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in bytes.chunks(3) {
            let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                out.push(if i <= c.len() { T[((n >> (18 - 6 * i)) & 63) as usize] as char } else { '=' });
            }
        }
        out
    }

    fn send(port: u16, raw: &'static [u8]) -> std::thread::JoinHandle<Vec<u8>> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).expect("timeout");
        stream.write_all(raw).expect("write");
        std::thread::spawn(move || {
            let mut got = Vec::new();
            let _ = stream.read_to_end(&mut got);
            let _ = stream.shutdown(Shutdown::Both);
            got
        })
    }

    fn json_of(value: &Value) -> Json {
        serde_json::to_value(value).expect("json")
    }

    fn emits(outcomes: &DispatchOutcomes) -> Vec<(String, Json)> {
        outcomes
            .iter()
            .flat_map(|(_, result)| result.effect_results.iter())
            .filter_map(|r| match &r.effect {
                Effect::Emit { event, payload } => Some((event.clone(), json_of(payload))),
                _ => None,
            })
            .collect()
    }

    const SSE_PROGRAM: &str = "fixtures/http1/sse-over-http1.lolo";
    /// The port the fixture's composed Http1 sets; a composed trait's knobs are not reachable from the schema.
    const SSE_PORT: u16 = 47391;

    fn with_knobs(schema: OrbitalSchema, port: u16, knobs: &[(&str, Json)]) -> OrbitalSchema {
        let mut json = serde_json::to_value(&schema).expect("schema json");
        for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                for (knob, value) in knobs.iter().cloned().chain([("port", json!(port))]) {
                    if let Some(entry) = t["config"].get_mut(knob) {
                        entry["default"] = value;
                    }
                }
            }
        }
        serde_json::from_value(json).expect("schema back from json")
    }

    /// Knobs are set before resolve (the atom's own config) and after it (config a composed trait inherits).
    fn schema(path: &std::path::Path, port: u16, knobs: &[(&str, Json)]) -> OrbitalSchema {
        let schema = with_knobs(orbital_test_support::lower_lolo_file(path).unwrap_or_else(|e| panic!("lower: {e}")), port, knobs);
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let options = orbital_compiler::CompilerOptions::from_env();
        let resolved = orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, Some(path))
            .unwrap_or_else(|e| panic!("resolve: {e}"))
            .0;
        with_knobs(resolved, port, knobs)
    }

    /// A program on the kernel over `os/tcp-listen`, `os/recv`, `os/send`, `os/close`, listening on a free loopback port.
    struct Server {
        port: u16,
        kernel: Arc<RuntimeKernel>,
        sink: Arc<KernelCompletionSink>,
        handles: Arc<OsHandles>,
        events: Arc<Mutex<Vec<(String, Json)>>>,
        errors: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn boot(path: std::path::PathBuf) -> Server {
            Server::boot_with(path, &[])
        }

        fn boot_with(path: std::path::PathBuf, knobs: &[(&str, Json)]) -> Server {
            let port = TcpListener::bind("127.0.0.1:0").expect("free port").local_addr().expect("addr").port();
            Server::boot_on(path, port, knobs)
        }

        fn boot_on(path: std::path::PathBuf, port: u16, knobs: &[(&str, Json)]) -> Server {
            let grants: Arc<OsGrants> = Arc::new(serde_json::from_value(json!({ "net": [{ "target": format!("127.0.0.1:{port}"), "kinds": ["listen"] }] })).expect("grants"));
            let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
            let kernel = Arc::new(RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), KernelConfig::default()));
            kernel.register_schema(schema(&path, port, knobs)).expect("register");
            let handles = Arc::new(OsHandles::default());
            let answer: Arc<CompletionAnswerer> = {
                let (handles, grants) = (handles.clone(), grants.clone());
                Arc::new(move |request: &IoRequest| match request {
                    IoRequest::Os { .. } => {
                        let on_event = |_: Option<&CallSite>, _: Value| {};
                        answer_os(Some(&OsHost { grants: &grants, handles: &handles, on_event: &on_event }), request)
                    }
                    other => BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(other),
                })
            };
            let events: Arc<Mutex<Vec<(String, Json)>>> = Arc::default();
            let errors: Arc<Mutex<Vec<String>>> = Arc::default();
            let on_done: Arc<CompletionDone> = {
                let (events, errors) = (events.clone(), errors.clone());
                Arc::new(move |_: &CallSite, event: &str, _: &orbital_core::Value, _: Option<&str>, result: Result<DispatchOutcomes, DispatchError>| match result {
                    Ok(outcomes) => events.lock().expect("events").extend(emits(&outcomes)),
                    Err(error) => errors.lock().expect("errors").push(format!("{event}: {error}")),
                })
            };
            let sink = Arc::new(KernelCompletionSink::new(kernel.clone(), answer, on_done));
            handles.completions.set_sink(sink.clone());
            let server = Server { port, kernel, sink, handles, events, errors };
            server.fire("Http1", "LISTEN", json!({}));
            server.wait("HTTP1_LISTENING", 0);
            server
        }

        fn fire(&self, on: &str, event: &str, payload: Json) {
            let progress = self.kernel.start_dispatch(on, None, event, v(payload), None, None).unwrap_or_else(|e| panic!("{event}: {e:?}"));
            let outcomes = self.sink.drive(progress).unwrap_or_else(|e| panic!("{event} drive: {e:?}"));
            self.events.lock().expect("events").extend(emits(&outcomes));
        }

        fn wait(&self, what: &str, seen: usize) -> Json {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some((_, p)) = self.events.lock().expect("events").iter().filter(|(e, _)| e == what).nth(seen) {
                    return p.clone();
                }
                assert!(Instant::now() < deadline, "no {what} #{seen}; events {:?}; errors {:?}", self.events.lock().expect("events"), self.errors.lock().expect("errors"));
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn respond(&self, request: &Json, status: u16, headers: &[(&str, &str)], body: &str) {
            let headers: Vec<Json> = headers.iter().map(|(n, v)| json!({ "name": n, "value": v })).collect();
            self.fire("Http1", "SEND_RESPONSE", json!({ "requestId": request["requestId"], "status": status, "headers": headers, "body": b64(body.as_bytes()) }));
        }

        fn finish(self) {
            assert!(self.errors.lock().expect("errors").is_empty(), "dispatch errors {:?}", self.errors.lock().expect("errors"));
            self.handles.close_all();
        }
    }

    fn connect(port: u16) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).expect("timeout");
        stream
    }

    /// Read exactly `want.len()` bytes, so a connection kept open is not waited on to its end.
    fn read_exact_text(stream: &mut TcpStream, len: usize) -> String {
        let mut got = vec![0u8; len];
        let mut at = 0;
        while at < len {
            match stream.read(&mut got[at..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => at += n,
            }
        }
        got.truncate(at);
        String::from_utf8_lossy(&got).into_owned()
    }

    fn read_rest(stream: &mut TcpStream) -> String {
        let mut got = Vec::new();
        let _ = stream.read_to_end(&mut got);
        String::from_utf8_lossy(&got).into_owned()
    }

    fn without_connection(record: &Json) -> Json {
        let mut record = record.clone();
        if let Some(headers) = record["headers"].as_object_mut() {
            headers.remove("connection");
        }
        record
    }

    /// The atom: `std-http1` answering the recorded cases, each request asking `Connection: close`.
    fn atom() -> Vec<(Option<Json>, Vec<u8>)> {
        let server = Server::boot(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(ATOM));
        let mut out = Vec::new();
        let mut delivered = 0;
        for case in CASES {
            let client = send(server.port, case.raw);
            let mut record = None;
            if let Some((status, headers, body)) = case.answer {
                let request = server.wait("REQUEST_RECEIVED", delivered);
                delivered += 1;
                server.respond(&request, status, headers, body);
                record = Some(json!({ "method": request["method"], "path": request["path"], "query": request["query"], "headers": request["headers"], "body": request["body"] }));
            }
            out.push((record, client.join().expect("client")));
        }
        server.finish();
        out
    }

    #[test]
    fn std_http1_answers_every_case_with_the_bytes_the_http_operators_wrote() {
        let want: Vec<Json> = serde_json::from_str(RECORDED).expect("recorded");
        let got = atom();
        assert_eq!(want.len(), CASES.len(), "control: one recording per case");
        for ((case, want), (got_record, got_bytes)) in CASES.iter().zip(&want).zip(&got) {
            if let Some(got_record) = got_record {
                let asked = got_record["headers"]["connection"].as_str().expect("the connection header is delivered");
                assert!(asked.split(", ").any(|t| t == "close"), "{}: Connection: close was sent, got {asked}", case.name);
                assert_eq!(without_connection(got_record), without_connection(&want["record"]), "{}: delivered request record", case.name);
            } else {
                assert_eq!(Json::Null, want["record"], "{}: delivered request record", case.name);
            }
            assert_eq!(String::from_utf8_lossy(got_bytes), want["reply"].as_str().expect("reply"), "{}: reply bytes", case.name);
        }
    }

    fn http1() -> Server {
        Server::boot(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(ATOM))
    }

    /// RFC 7230 §6.3: an HTTP/1.1 connection persists unless `close` is sent; replies come back in request order.
    #[test]
    fn std_http1_keeps_an_http11_connection_alive_until_the_client_sends_close() {
        let server = http1();
        let mut conn = connect(server.port);
        let replies = [
            (&b"GET /one HTTP/1.1\r\nHost: a\r\n\r\n"[..], "one", "HTTP/1.1 200 OK\r\ncontent-length: 3\r\nconnection: keep-alive\r\n\r\none"),
            (&b"GET /two HTTP/1.1\r\nHost: a\r\n\r\n"[..], "two", "HTTP/1.1 200 OK\r\ncontent-length: 3\r\nconnection: keep-alive\r\n\r\ntwo"),
        ];
        for (n, (raw, body, want)) in replies.iter().enumerate() {
            conn.write_all(raw).expect("write");
            let request = server.wait("REQUEST_RECEIVED", n);
            assert_eq!(request["path"], json!(format!("/{body}")), "request {n} delivered in order");
            server.respond(&request, 200, &[], body);
            assert_eq!(read_exact_text(&mut conn, want.len()), *want, "reply {n} on the kept-alive connection");
        }
        conn.write_all(b"GET /three HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n").expect("write");
        let request = server.wait("REQUEST_RECEIVED", 2);
        assert_eq!(request["requestId"], server.wait("REQUEST_RECEIVED", 0)["requestId"], "the same connection carried all three");
        server.respond(&request, 200, &[], "three");
        assert_eq!(read_rest(&mut conn), "HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nthree", "the close reply, then the server closes");
        server.finish();
    }

    /// RFC 7230 §6.3: HTTP/1.0 without `keep-alive` closes after the reply.
    #[test]
    fn std_http1_closes_an_http10_connection_that_did_not_ask_keep_alive() {
        let server = http1();
        let mut conn = connect(server.port);
        conn.write_all(b"GET /old HTTP/1.0\r\nHost: a\r\n\r\n").expect("write");
        let request = server.wait("REQUEST_RECEIVED", 0);
        server.respond(&request, 200, &[], "ok");
        assert_eq!(read_rest(&mut conn), "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
        server.finish();
    }

    /// RFC 7230 §4.1: chunk-size in hex, CRLF, data, CRLF; `0` CRLF CRLF ends the body.
    #[test]
    fn std_http1_streams_a_chunked_reply_with_exact_framing() {
        let server = http1();
        let mut conn = connect(server.port);
        conn.write_all(b"GET /s HTTP/1.1\r\nHost: a\r\n\r\n").expect("write");
        let request = server.wait("REQUEST_RECEIVED", 0);
        let id = request["requestId"].clone();
        server.fire("Http1", "RESPOND_HEAD", json!({ "requestId": id, "status": 200, "headers": [{ "name": "Content-Type", "value": "text/plain" }] }));
        server.wait("STREAM_OPENED", 0);
        server.fire("Http1", "RESPOND_CHUNK", json!({ "requestId": id, "body": b64(b"hello") }));
        server.wait("CHUNK_SENT", 0);
        server.fire("Http1", "RESPOND_CHUNK", json!({ "requestId": id, "body": b64(b"0123456789abcdefg") }));
        server.wait("CHUNK_SENT", 1);
        server.fire("Http1", "RESPOND_END", json!({ "requestId": id }));
        server.wait("RESPONDED", 0);
        assert_eq!(
            read_rest(&mut conn),
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n5\r\nhello\r\n11\r\n0123456789abcdefg\r\n0\r\n\r\n"
        );
        server.finish();
    }

    /// HTML Living Standard §9.2: text/event-stream, `retry:`, `id:`/`event:` lines, one `data:` line per data line, a blank line.
    #[test]
    fn std_sse_over_std_http1_frames_events_and_reports_a_client_disconnect() {
        let server = Server::boot_on(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SSE_PROGRAM), SSE_PORT, &[]);
        let mut conn = connect(server.port);
        conn.write_all(b"GET /events HTTP/1.1\r\nHost: a\r\nAccept: text/event-stream\r\n\r\n").expect("write");
        let request = server.wait("REQUEST_RECEIVED", 0);
        let id = request["requestId"].clone();
        server.fire("Sse", "SSE_OPEN", json!({ "requestId": id }));
        server.wait("CHUNK_SENT", 0);
        server.fire("Sse", "SSE_SEND", json!({ "requestId": id, "id": "7", "event": "tick", "data": "a\nb" }));
        server.wait("CHUNK_SENT", 1);
        let want = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nx-accel-buffering: no\r\nconnection: close\r\n\r\nretry: 3000\n\nid: 7\nevent: tick\ndata: a\ndata: b\n\n";
        assert_eq!(read_exact_text(&mut conn, want.len()), want);
        conn.shutdown(Shutdown::Both).expect("client leaves");
        drop(conn);
        assert_eq!(server.wait("SSE_CLOSED", 0), json!({ "requestId": id, "reason": "client" }));
        server.finish();
    }

    const CHUNKED_HEAD: &[u8] = b"POST /u HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
    const KEPT: &str = "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: keep-alive\r\n\r\nok";

    fn refusal(status: u16, reason: &str) -> String {
        format!("HTTP/1.1 {status} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
    }

    fn joined(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    /// RFC 7230 §4.1 chunked request bodies (owner ruling "HTTP scope", S52): decoded and delivered once, whatever the reads.
    #[test]
    fn std_http1_decodes_a_chunked_request_body() {
        let server = http1();
        let cases: Vec<(&str, Vec<Vec<u8>>, &str)> = vec![
            ("two chunks", vec![joined(&[CHUNKED_HEAD, b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"])], "hello world"),
            ("a chunk extension is ignored", vec![joined(&[CHUNKED_HEAD, b"5;ext=1\r\nhello\r\n0\r\n\r\n"])], "hello"),
            (
                "split inside the size line and inside data",
                vec![CHUNKED_HEAD.to_vec(), b"5\r\nhel".to_vec(), b"lo\r\n".to_vec(), b"6".to_vec(), b"\r\n wo".to_vec(), b"rld\r\n0\r".to_vec(), b"\n\r\n".to_vec()],
                "hello world",
            ),
            ("a trailer is read and dropped", vec![joined(&[CHUNKED_HEAD, b"5\r\nhello\r\n0\r\nX-T: 1\r\n\r\n"])], "hello"),
        ];
        let mut wrong = Vec::new();
        for (n, (name, pieces, body)) in cases.iter().enumerate() {
            let mut conn = connect(server.port);
            for piece in pieces {
                conn.write_all(piece).expect("write");
                std::thread::sleep(Duration::from_millis(100));
            }
            let request = server.wait("REQUEST_RECEIVED", n);
            if request["body"] != json!(b64(body.as_bytes())) {
                wrong.push(format!("{name}: body {} want {}", request["body"], b64(body.as_bytes())));
            }
            if request["headers"].get("x-t").is_some() {
                wrong.push(format!("{name}: a trailer reached the request headers {}", request["headers"]));
            }
            server.respond(&request, 200, &[], "ok");
            let reply = read_exact_text(&mut conn, KEPT.len());
            if reply != KEPT {
                wrong.push(format!("{name}: reply {reply:?}"));
            }
        }
        let delivered = server.events.lock().expect("events").iter().filter(|(e, _)| e == "REQUEST_RECEIVED").count();
        if delivered != cases.len() {
            wrong.push(format!("delivered {delivered} times for {} requests", cases.len()));
        }

        let mut conn = connect(server.port);
        conn.write_all(&joined(&[CHUNKED_HEAD, b"5\r\nhello\r\n0\r\n\r\nGET /next HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"])).expect("write");
        let first = server.wait("REQUEST_RECEIVED", cases.len());
        if first["body"] != json!(b64(b"hello")) {
            wrong.push(format!("pipelined: first body {}", first["body"]));
        }
        server.respond(&first, 200, &[], "ok");
        let reply = read_exact_text(&mut conn, KEPT.len());
        if reply != KEPT {
            wrong.push(format!("pipelined: first reply {reply:?}"));
        }
        let next = server.wait("REQUEST_RECEIVED", cases.len() + 1);
        if next["path"] != json!("/next") || next["method"] != json!("GET") {
            wrong.push(format!("pipelined: the bytes after the final blank line were not the next request: {next}"));
        }
        server.respond(&next, 200, &[], "ok");
        let rest = read_rest(&mut conn);
        if rest != "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok" {
            wrong.push(format!("pipelined: second reply {rest:?}"));
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
        server.finish();
    }

    /// RFC 7230 §3.3.1, §3.3.3, §4.1: framing the atom cannot decode is refused and the connection closed.
    #[test]
    fn std_http1_refuses_chunked_framing_it_cannot_decode() {
        let long_line = joined(&[CHUNKED_HEAD, b"5;", &[b'a'; 130][..], b"\r\nhello\r\n0\r\n\r\n"]);
        let cases: Vec<(&str, Vec<u8>, String)> = vec![
            ("a coding other than chunked is 501", b"POST /u HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip\r\n\r\nxxxxx".to_vec(), refusal(501, "Not Implemented")),
            ("chunked with a content-length is 400", b"POST /u HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec(), refusal(400, "Bad Request")),
            ("chunked twice is 400", b"POST /u HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked, chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec(), refusal(400, "Bad Request")),
            ("chunked on HTTP/1.0 is 400", b"POST /u HTTP/1.0\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec(), refusal(400, "Bad Request")),
            ("a size that is not hex is 400", joined(&[CHUNKED_HEAD, b"zz\r\nhello\r\n0\r\n\r\n"]), refusal(400, "Bad Request")),
            ("a chunk not followed by CRLF is 400", joined(&[CHUNKED_HEAD, b"5\r\nhelloXX0\r\n\r\n"]), refusal(400, "Bad Request")),
            ("a size line over 128 bytes is 400", long_line, refusal(400, "Bad Request")),
        ];
        let server = http1();
        let mut wrong = Vec::new();
        for (name, raw, want) in &cases {
            let mut conn = connect(server.port);
            conn.write_all(raw).expect("write");
            let got = read_rest(&mut conn);
            if &got != want {
                wrong.push(format!("{name}: got {got:?} want {want:?}"));
            }
        }
        if server.events.lock().expect("events").iter().any(|(e, _)| e == "REQUEST_RECEIVED") {
            wrong.push("a refused request was delivered".to_string());
        }
        server.finish();

        let small = Server::boot_with(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(ATOM), &[("maxBody", json!(8))]);
        let mut conn = connect(small.port);
        conn.write_all(&joined(&[CHUNKED_HEAD, b"5\r\nhello\r\n9\r\n"])).expect("write");
        let got = read_rest(&mut conn);
        let want = refusal(413, "Payload Too Large");
        if got != want {
            wrong.push(format!("a decoded body over maxBody is 413 at the size line: got {got:?} want {want:?}"));
        }
        small.finish();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    const SERVER: &str = "almadar-std/behaviors/lolo/net/organisms/std-http-server.lolo";

    /// The organism's own `net` grant and Http1 port; a composed trait's knobs are not reachable from the schema.
    const SERVER_PORT: u16 = 3000;
    /// The organism's tests share its one port, so they run one at a time.
    static SERVER_PORT_LOCK: Mutex<()> = Mutex::new(());

    /// Closes the listener even when a case panics, then frees the port for the next case.
    struct PortHeld(Arc<OsHandles>, #[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl Drop for PortHeld {
        fn drop(&mut self) {
            self.0.close_all();
        }
    }

    fn http_server() -> (Server, PortHeld) {
        let guard = SERVER_PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let server = Server::boot_on(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SERVER), SERVER_PORT, &[]);
        let held = PortHeld(server.handles.clone(), guard);
        (server, held)
    }

    /// (status line, lower-cased headers, body) of one HTTP/1.1 reply.
    fn parsed(reply: &str) -> (String, Vec<(String, String)>, String) {
        let (head, body) = reply.split_once("\r\n\r\n").unwrap_or_else(|| panic!("no head in {reply:?}"));
        let mut lines = head.split("\r\n");
        let status = lines.next().unwrap_or("").to_string();
        let headers = lines.filter_map(|l| l.split_once(": ")).map(|(n, v)| (n.to_ascii_lowercase(), v.to_string())).collect();
        (status, headers, body.to_string())
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn ask(port: u16, raw: &str) -> String {
        let mut conn = connect(port);
        conn.write_all(raw.as_bytes()).expect("write");
        read_rest(&mut conn)
    }

    fn mint(port: u16) -> String {
        ask(port, "POST /api/events/ticket HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    }

    fn ticket_of(server: &Server, reply: &str) -> String {
        let (status, headers, body) = parsed(reply);
        assert_eq!(status, "HTTP/1.1 200 OK", "mint reply {reply:?}; events {:?}; errors {:?}", server.events.lock().expect("events").iter().filter(|(e, _)| e.starts_with("TICKET") || e.starts_with("SEED") || e == "MINT_REQUESTED").collect::<Vec<_>>(), server.errors.lock().expect("errors"));
        assert_eq!(header(&headers, "content-type"), Some("application/json"), "mint content-type in {reply:?}");
        let json: Json = serde_json::from_str(&body).unwrap_or_else(|e| panic!("mint body {body:?}: {e}"));
        let ticket = json["ticket"].as_str().unwrap_or_else(|| panic!("no ticket in {json}")).to_string();
        assert_eq!(json.as_object().map(|o| o.len()), Some(1), "the mint answers only {{ticket}}: {json}");
        assert_eq!(ticket.len(), 43, "32 bytes base64url unpadded: {ticket}");
        ticket
    }

    fn unauthorized(reply: &str, what: &str) {
        let (status, headers, body) = parsed(reply);
        assert_eq!(status, "HTTP/1.1 401 Unauthorized", "{what}: {reply:?}");
        assert_eq!(header(&headers, "content-type"), Some("application/json"), "{what}: {reply:?}");
        assert_eq!(serde_json::from_str::<Json>(&body).expect("401 body json"), json!({ "error": "Unauthorized", "code": "UNAUTHORIZED" }), "{what}");
    }

    /// The organism boots and routes: GET /api/health answers 200 { status, version }.
    #[test]
    fn std_http_server_answers_health() {
        let (server, _held) = http_server();
        let reply = ask(server.port, "GET /api/health HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n");
        let (status, headers, body) = parsed(&reply);
        assert_eq!(status, "HTTP/1.1 200 OK", "{reply:?}");
        assert_eq!(header(&headers, "content-type"), Some("application/json"), "{reply:?}");
        assert_eq!(serde_json::from_str::<Json>(&body).expect("health json"), json!({ "status": "healthy", "version": "0.0.0" }));
        server.finish();
    }

    /// An unauthenticated POST /api/events/ticket mints for the anonymous viewer and answers 200 { ticket }.
    #[test]
    fn std_http_server_mints_a_ticket_for_an_anonymous_request() {
        let (server, _held) = http_server();
        ticket_of(&server, &mint(server.port));
        server.finish();
    }

    /// Open the stream with a ticket minted on the atom, leave, and the server forgets it; the spent ticket and bad tickets are 401.
    #[test]
    fn std_http_server_opens_the_event_stream_on_a_ticket() {
        let (server, _held) = http_server();
        server.fire("StreamTicketer", "MINT_TICKET", json!({ "viewer": { "id": "u1", "email": "u1@a.test", "name": "U", "role": "user", "permissions": [] }, "requestId": 0 }));
        let ticket = server.wait("TICKET_MINTED", 0)["ticket"].as_str().expect("ticket").to_string();

        let mut conn = connect(server.port);
        conn.write_all(format!("GET /api/events?ticket={ticket} HTTP/1.1\r\nHost: a\r\nAccept: text/event-stream\r\n\r\n").as_bytes()).expect("write");
        let want = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nx-accel-buffering: no\r\nconnection: close\r\n\r\nretry: 3000\n\n";
        assert_eq!(read_exact_text(&mut conn, want.len()), want, "SSE head and retry frame");
        let authorized = server.wait("STREAM_AUTHORIZED", 0);
        let id = authorized["requestId"].clone();
        conn.shutdown(Shutdown::Both).expect("client leaves");
        drop(conn);
        assert_eq!(server.wait("SSE_CLOSED", 0), json!({ "requestId": id, "reason": "client" }));
        assert_eq!(server.wait("STREAM_CLOSED", 0)["requestId"], id, "std-http1 reported the close");

        unauthorized(&ask(server.port, &format!("GET /api/events?ticket={ticket} HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n")), "a spent ticket");
        unauthorized(&ask(server.port, "GET /api/events?ticket=bogus HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n"), "a bogus ticket");
        unauthorized(&ask(server.port, "GET /api/events HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n"), "no ticket");
        server.finish();
    }

    /// Two mints in flight at once are both answered, each with its own ticket.
    #[test]
    fn std_http_server_answers_two_concurrent_mints() {
        let (server, _held) = http_server();
        let raw = "POST /api/events/ticket HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let mut a = connect(server.port);
        let mut b = connect(server.port);
        a.write_all(raw.as_bytes()).expect("write a");
        b.write_all(raw.as_bytes()).expect("write b");
        let (ra, rb) = (read_rest(&mut a), read_rest(&mut b));
        let (ta, tb) = (ticket_of(&server, &ra), ticket_of(&server, &rb));
        assert_ne!(ta, tb, "two mints, two tickets");
        server.finish();
    }
}

mod machine_parity {
    use orbital_core::effects::Effect;
    use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoRequest};
    use orbital_core::runtime::{EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value};
    use serde_json::{json, Value as Json};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn packages(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(rel)
    }

    fn resolve(schema: OrbitalSchema, path: Option<&Path>) -> OrbitalSchema {
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let options = orbital_compiler::CompilerOptions::from_env();
        orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, path).unwrap_or_else(|e| panic!("resolve: {e}")).0
    }

    fn resolved_lolo(rel: &str) -> OrbitalSchema {
        let path = packages(rel);
        resolve(orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower {rel}: {e}")), Some(&path))
    }

    /// `std-machine` on the kernel: MACHINE_READ { program } → the MACHINE_ANSWERED rows.
    fn atom_rows(program: &OrbitalSchema) -> Json {
        let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
        let kernel = RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), KernelConfig::default());
        kernel.register_schema(resolved_lolo("almadar-std/behaviors/lolo/dev/atoms/std-machine.lolo")).expect("register");
        let payload: Value = serde_json::from_value(json!({ "program": serde_json::to_value(program).expect("program json") })).expect("payload");
        let progress = kernel.start_dispatch("Machine", None, "MACHINE_READ", payload, None, None).unwrap_or_else(|e| panic!("dispatch: {e:?}"));
        let answer = |request: &IoRequest| BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(request);
        let outcomes = drive_blocking(&kernel, progress, &answer).unwrap_or_else(|e| panic!("drive: {e:?}"));
        let answered: Vec<Json> = outcomes
            .iter()
            .flat_map(|(_, o)| {
                assert_eq!(o.failure, None, "an effect failed");
                o.effect_results.iter()
            })
            .filter_map(|r| match &r.effect {
                Effect::Emit { event, payload } if event == "MACHINE_ANSWERED" => Some(serde_json::to_value(payload).expect("json")["rows"].clone()),
                _ => None,
            })
            .collect();
        assert_eq!(answered.len(), 1, "one MACHINE_ANSWERED");
        answered.into_iter().next().expect("rows")
    }

    /// What the retired `program/machine` operator (`orbital_core::program_machine::machine_rows`) answered for each program.
    const RECORDED: &[(&str, &str)] = &[
        ("std-cooldown", r#"[{"orbital":"CooldownOrbital","trait":"Cooldown","from":"ready","event":"START","to":"cooling","guarded":false,"payload":[{"name":"durationMs","type":"number","required":false}]},{"orbital":"CooldownOrbital","trait":"Cooldown","from":"cooling","event":"EXPIRE","to":"ready","guarded":false,"payload":[]}]"#),
        ("std-retry", r#"[{"orbital":"RetryOrbital","trait":"Retry","from":"idle","event":"INIT","to":"idle","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"idle","event":"START","to":"running","guarded":false,"payload":[{"name":"key","type":"CallKey","required":true}]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"INIT","to":"idle","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"SUCCEEDED","to":"idle","guarded":true,"payload":[{"name":"key","type":"CallKey","required":true}]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"FAILED","to":"backoff","guarded":true,"payload":[{"name":"key","type":"CallKey","required":true},{"name":"error","type":"string","required":false}]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"FAILED","to":"idle","guarded":true,"payload":[{"name":"key","type":"CallKey","required":true},{"name":"error","type":"string","required":false}]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"CANCEL","to":"idle","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"backoff","event":"INIT","to":"idle","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"backoff","event":"TIMER_DUE","to":"running","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"backoff","event":"CANCEL","to":"idle","guarded":false,"payload":[]}]"#),
        ("almadar-analytics-collector", r#"[{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"INIT","to":"ready","guarded":false,"payload":[]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"BEACON_RECEIVED","to":"ready","guarded":true,"payload":[{"name":"headers","type":"Map<string,string>","required":false},{"name":"rawBody","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"BEACON_RECEIVED","to":"ready","guarded":true,"payload":[{"name":"headers","type":"Map<string,string>","required":false},{"name":"rawBody","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"BEACON_RECEIVED","to":"ready","guarded":true,"payload":[{"name":"headers","type":"Map<string,string>","required":false},{"name":"rawBody","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"BEACON_ACCEPTED","to":"ready","guarded":true,"payload":[{"name":"draft","type":"object","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"BEACON_ACCEPTED","to":"ready","guarded":true,"payload":[{"name":"draft","type":"object","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_LOADED","to":"ready","guarded":false,"payload":[{"name":"data","type":"object","required":true,"entity":"AnalyticsSalt"}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_ABSENT","to":"ready","guarded":true,"payload":[{"name":"error","type":"string","required":false},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_ABSENT","to":"ready","guarded":true,"payload":[{"name":"error","type":"string","required":false},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_DRAWN","to":"ready","guarded":false,"payload":[{"name":"value","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_CREATED","to":"ready","guarded":false,"payload":[{"name":"id","type":"string","required":true},{"name":"salt","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_TAKEN","to":"ready","guarded":true,"payload":[{"name":"error","type":"string","required":false},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_TAKEN","to":"ready","guarded":true,"payload":[{"name":"error","type":"string","required":false},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_UNREADABLE","to":"ready","guarded":false,"payload":[{"name":"error","type":"string","required":false},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_FAILED","to":"ready","guarded":false,"payload":[{"name":"error","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"SALT_READY","to":"ready","guarded":false,"payload":[]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"EVENT_RECORDED","to":"ready","guarded":false,"payload":[{"name":"id","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsIntake","from":"ready","event":"EVENT_UNRECORDED","to":"ready","guarded":false,"payload":[{"name":"error","type":"string","required":false},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsStats","from":"ready","event":"INIT","to":"ready","guarded":true,"payload":[]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsStats","from":"ready","event":"INIT","to":"ready","guarded":true,"payload":[]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsStats","from":"ready","event":"ANALYTICS_STATS","to":"ready","guarded":false,"payload":[{"name":"domain","type":"string","required":false},{"name":"from","type":"number","required":false},{"name":"to","type":"number","required":false},{"name":"limit","type":"number","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsStats","from":"ready","event":"STATS_LOADED","to":"ready","guarded":false,"payload":[{"name":"data","type":"[object]","required":true,"entity":"AnalyticsEvent"}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarAnalyticsStats","from":"ready","event":"STATS_UNREADABLE","to":"ready","guarded":false,"payload":[{"name":"error","type":"string","required":false},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"INIT","to":"ready","guarded":false,"payload":[]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"PERSONAS_CALLED","to":"ready","guarded":false,"payload":[{"name":"headers","type":"Map<string,string>","required":false},{"name":"rawBody","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"PERSONAS_HOST","to":"ready","guarded":true,"payload":[{"name":"value","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"PERSONAS_HOST","to":"ready","guarded":true,"payload":[{"name":"value","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"PERSONAS_ROWS","to":"ready","guarded":false,"payload":[{"name":"data","type":"[object]","required":true,"entity":"AlmadarViewer"}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"PERSONAS_FAILED","to":"ready","guarded":false,"payload":[{"name":"error","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_CALLED","to":"ready","guarded":true,"payload":[{"name":"headers","type":"Map<string,string>","required":false},{"name":"rawBody","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_CALLED","to":"ready","guarded":true,"payload":[{"name":"headers","type":"Map<string,string>","required":false},{"name":"rawBody","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_HOST","to":"ready","guarded":true,"payload":[{"name":"value","type":"string","required":false},{"name":"key","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_HOST","to":"ready","guarded":true,"payload":[{"name":"value","type":"string","required":false},{"name":"key","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_ROW","to":"ready","guarded":true,"payload":[{"name":"data","type":"object","required":true,"entity":"AlmadarViewer"}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_ROW","to":"ready","guarded":true,"payload":[{"name":"data","type":"object","required":true,"entity":"AlmadarViewer"}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_CREATED","to":"ready","guarded":true,"payload":[{"name":"status","type":"number","required":true},{"name":"body","type":"string","required":true},{"name":"key","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_CREATED","to":"ready","guarded":true,"payload":[{"name":"status","type":"number","required":true},{"name":"body","type":"string","required":true},{"name":"key","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_CLAIMS","to":"ready","guarded":false,"payload":[{"name":"data","type":"object","required":true,"entity":"AlmadarViewer"}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_UPDATED","to":"ready","guarded":true,"payload":[{"name":"status","type":"number","required":true},{"name":"body","type":"string","required":true},{"name":"key","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_UPDATED","to":"ready","guarded":true,"payload":[{"name":"status","type":"number","required":true},{"name":"body","type":"string","required":true},{"name":"key","type":"string","required":true}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_ROW_MISSED","to":"ready","guarded":true,"payload":[{"name":"error","type":"string","required":true},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_ROW_MISSED","to":"ready","guarded":true,"payload":[{"name":"error","type":"string","required":true},{"name":"code","type":"string","required":false}]},{"orbital":"AlmadarAnalyticsCollector","trait":"AlmadarViewerDevPersonas","from":"ready","event":"SIGN_IN_FAILED","to":"ready","guarded":false,"payload":[{"name":"error","type":"string","required":false}]}]"#),
        ("from-list", r#"[{"orbital":"DoorOrbital","trait":"Lock","from":"locked","event":"OPEN","to":"open","guarded":false,"payload":[]},{"orbital":"DoorOrbital","trait":"Lock","from":"open","event":"SHUT","to":"locked","guarded":false,"payload":[]},{"orbital":"DoorOrbital","trait":"Lock","from":"ajar","event":"SHUT","to":"locked","guarded":false,"payload":[]}]"#),
    ];

    fn recorded(name: &str) -> Json {
        let (_, rows) = RECORDED.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no recording for {name}"));
        serde_json::from_str(rows).expect("recorded")
    }

    fn assert_parity(name: &str, program: &OrbitalSchema) {
        let want = recorded(name);
        assert!(want.as_array().is_some_and(|r| !r.is_empty()), "{name}: control, the program has rows");
        let got = atom_rows(program);
        assert_eq!(got, want, "{name}: std-machine rows vs the recorded program/machine rows");
    }

    #[test]
    fn machine_rows_match_on_std_cooldown() {
        assert_parity("std-cooldown", &resolved_lolo("almadar-std/behaviors/lolo/flow/atoms/std-cooldown.lolo"));
    }

    #[test]
    fn machine_rows_match_on_guarded_std_retry() {
        let program = resolved_lolo("almadar-std/behaviors/lolo/flow/atoms/std-retry.lolo");
        assert!(recorded("std-retry").as_array().expect("rows").iter().any(|r| r["guarded"] == json!(true)), "control: std-retry has guarded rows");
        assert_parity("std-retry", &program);
    }

    #[test]
    fn machine_rows_match_on_composed_analytics_collector() {
        assert_parity("almadar-analytics-collector", &resolved_lolo("almadar-behaviors/behaviors/lolo/service/atoms/almadar-analytics-collector.lolo"));
    }

    #[test]
    fn machine_rows_match_on_a_from_list() {
        let program: OrbitalSchema = serde_json::from_value(json!({
            "name": "from-list",
            "orbitals": [{
                "name": "DoorOrbital",
                "entity": { "name": "Door", "fields": [{ "name": "id", "type": "string", "required": true }] },
                "traits": [{
                    "name": "Lock",
                    "linkedEntity": "Door",
                    "stateMachine": {
                        "states": [{ "name": "locked", "isInitial": true }, { "name": "open" }, { "name": "ajar" }],
                        "events": [{ "key": "SHUT", "name": "Shut" }, { "key": "OPEN", "name": "Open" }],
                        "transitions": [
                            { "from": "locked", "to": "open", "event": "OPEN" },
                            { "from": ["open", "ajar"], "to": "locked", "event": "SHUT" }
                        ]
                    }
                }]
            }]
        }))
        .expect("from-list program");
        let resolved = resolve(program, None);
        let from = serde_json::to_value(&resolved).expect("json")["orbitals"][0]["traits"][0]["stateMachine"]["transitions"][1]["from"].clone();
        assert!(from.is_array(), "the resolved IR keeps a from list: {from}");
        assert_parity("from-list", &resolved);
    }
}

mod crypto_vectors {
    use orbital_core::effects::Effect;
    use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoRequest};
    use orbital_core::runtime::{EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value};
    use serde_json::{json, Value as Json};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn atom(topic: &str, name: &str) -> OrbitalSchema {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages/almadar-std/behaviors/lolo").join(topic).join("atoms").join(format!("{name}.lolo"));
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let schema = orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower {name}: {e}"));
        let options = orbital_compiler::CompilerOptions::from_env();
        orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, Some(&path)).unwrap_or_else(|e| panic!("resolve {name}: {e}")).0
    }

    fn vectors(group: &str) -> Vec<Json> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/crypto/vectors.json");
        let all: Json = serde_json::from_str(&std::fs::read_to_string(&path).expect("crypto vectors")).expect("vectors json");
        all[group].as_array().unwrap_or_else(|| panic!("no {group} vectors")).clone()
    }

    /// One request on a fresh kernel → the (event, payload) of the single answer among `answers`.
    fn answer(program: &OrbitalSchema, trait_name: &str, event: &str, payload: Json, answers: &[&str]) -> (String, Json) {
        let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
        let kernel = RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), KernelConfig::default());
        kernel.register_schema(program.clone()).expect("register");
        let payload: Value = serde_json::from_value(payload).expect("payload");
        let progress = kernel.start_dispatch(trait_name, None, event, payload, None, None).unwrap_or_else(|e| panic!("dispatch: {e:?}"));
        let host = |request: &IoRequest| BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(request);
        let outcomes = drive_blocking(&kernel, progress, &host).unwrap_or_else(|e| panic!("drive: {e:?}"));
        let got: Vec<(String, Json)> = outcomes
            .iter()
            .flat_map(|(_, o)| {
                assert_eq!(o.failure, None, "an effect failed");
                o.effect_results.iter()
            })
            .filter_map(|r| match &r.effect {
                Effect::Emit { event, payload } if answers.contains(&event.as_str()) => Some((event.clone(), serde_json::to_value(payload).expect("json"))),
                _ => None,
            })
            .collect();
        assert_eq!(got.len(), 1, "exactly one answer, got {got:?}");
        got.into_iter().next().expect("answer")
    }

    /// `VERIFIED true|false` or `REFUSED <reason>` from a verifier answer.
    fn verdict(prefix: &str, (event, payload): (String, Json)) -> String {
        match event.strip_prefix(prefix) {
            Some("VERIFIED") => format!("VERIFIED {}", payload["valid"]),
            Some("REFUSED") => format!("REFUSED {}", payload["reason"].as_str().unwrap_or("?")),
            _ => panic!("unexpected answer {event}"),
        }
    }

    fn run_group(group: &str, judge: impl Fn(&Json) -> (String, String)) {
        let cases = vectors(group);
        assert!(!cases.is_empty(), "control: {group} has vectors");
        let started = std::time::Instant::now();
        let failures: Vec<String> = cases
            .iter()
            .filter_map(|case| {
                let t = std::time::Instant::now();
                let (want, got) = judge(case);
                eprintln!("{group}/{}: {got} ({:.1}s)", case["name"], t.elapsed().as_secs_f64());
                (want != got).then(|| format!("{}: want {want}, got {got}", case["name"]))
            })
            .collect();
        eprintln!("{group}: {} cases in {:.1}s", cases.len(), started.elapsed().as_secs_f64());
        assert!(failures.is_empty(), "{group} vectors failed:\n{}", failures.join("\n"));
    }

    #[test]
    #[ignore = "slow: run in checkpoint"]
    fn crypto_ecdsa_p256_vectors() {
        let program = atom("auth", "std-ecdsa");
        run_group("ecdsa", |c| {
            let payload = json!({ "x": c["x"], "y": c["y"], "message": c["message"], "signature": c["signature"] });
            let got = verdict("ECDSA_", answer(&program, "EcdsaVerifier", "ECDSA_VERIFY", payload, &["ECDSA_VERIFIED", "ECDSA_REFUSED"]));
            (c["expect"].as_str().expect("expect").to_string(), got)
        });
    }

    #[test]
    fn crypto_rsa_pkcs1_vectors() {
        let program = atom("auth", "std-rsa");
        run_group("rsa", |c| {
            let payload = json!({ "n": c["n"], "e": c["e"], "message": c["message"], "signature": c["signature"] });
            let got = verdict("RSA_", answer(&program, "RsaVerifier", "RSA_VERIFY", payload, &["RSA_VERIFIED", "RSA_REFUSED"]));
            (c["expect"].as_str().expect("expect").to_string(), got)
        });
    }

    #[test]
    fn crypto_jws_vectors() {
        let program = atom("auth", "std-jws");
        run_group("jws", |c| {
            let (event, payload) = answer(&program, "JwsVerifier", "JWS_VERIFY", json!({ "token": c["token"], "jwk": c["jwk"] }), &["JWS_VERIFIED", "JWS_REFUSED"]);
            let got = match payload["reason"].as_str() {
                Some(reason) => format!("{event} {reason}"),
                None => event,
            };
            (c["expect"].as_str().expect("expect").to_string(), got)
        });
    }

    /// Limbs come back as f64; a whole number prints as the integer python recorded.
    fn integral(value: Json) -> Json {
        match value {
            Json::Number(n) => match n.as_f64() {
                Some(f) if f.fract() == 0.0 && f.abs() < 9.007_199_254_740_992e15 => json!(f as i64),
                _ => Json::Number(n),
            },
            Json::Array(items) => Json::Array(items.into_iter().map(integral).collect()),
            Json::Object(map) => Json::Object(map.into_iter().map(|(k, v)| (k, integral(v))).collect()),
            other => other,
        }
    }

    #[test]
    fn crypto_bigint_vectors() {
        let program = atom("codec", "std-bigint");
        let answers = ["MODPOW_ANSWERED", "MODINV_ANSWERED", "DIVMOD_ANSWERED", "BIGINT_REFUSED"];
        run_group("bigint", |c| {
            let (event, payload) = answer(&program, "BigIntCodec", c["op"].as_str().expect("op"), c["args"].clone(), &answers);
            let want = format!("{} {}", c["answer"].as_str().expect("answer"), c["want"]);
            let got = format!("{event} {}", integral(payload));
            (want, got)
        });
    }
}

mod bytes_parity {
    use orbital_core::evaluator::{eval_expr, BindingContext};
    use orbital_core::schema::SExpression;
    use serde_json::Value as Json;

    fn shown(value: &Json) -> String {
        match value {
            Json::Null => "NONE".into(),
            Json::Number(n) => format!("VALUE {}", n.as_f64().expect("f64")),
            other => format!("VALUE {other}"),
        }
    }

    fn operator(form: &Json) -> String {
        let expr: SExpression = serde_json::from_value(form.clone()).expect("s-expression");
        match eval_expr(&expr, &BindingContext::default()) {
            Ok(v) => shown(&serde_json::to_value(v).expect("json")),
            Err(_) => "REFUSED".into(),
        }
    }

    /// Each form with what the `bytes/*` / `str/to-int` operators answer.
    const RECORDED: &[(&str, &str)] = &[
        (r#"["bytes/at", "", 0]"#, "NONE"),
        (r#"["bytes/at", "AAEC/w==", 0]"#, "VALUE 0"),
        (r#"["bytes/at", "AAEC/w==", 3]"#, "VALUE 255"),
        (r#"["bytes/at", "AAEC/w==", 4]"#, "NONE"),
        (r#"["bytes/at", "AAEC/w==", -1]"#, "NONE"),
        (r#"["bytes/at", "AAEC/w==", 1.5]"#, "NONE"),
        (r#"["bytes/at", "%%", 0]"#, "REFUSED"),
        (r#"["bytes/at", "aGk", 0]"#, "REFUSED"),
        (r#"["bytes/index-of", "aGVsbG8=", "aA=="]"#, "VALUE 0"),
        (r#"["bytes/index-of", "aGVsbG8=", "bw=="]"#, "VALUE 4"),
        (r#"["bytes/index-of", "aGVsbG8=", "bG8="]"#, "VALUE 3"),
        (r#"["bytes/index-of", "aGVsbG8=", "eg=="]"#, "VALUE -1"),
        (r#"["bytes/index-of", "aGVsbG8=", ""]"#, "VALUE 0"),
        (r#"["bytes/index-of", "aGVsbG8=", "", 2]"#, "VALUE 2"),
        (r#"["bytes/index-of", "aGVsbG8=", "", 9]"#, "VALUE 5"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", 3]"#, "VALUE 3"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", -2]"#, "VALUE 3"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", -99]"#, "VALUE 2"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", 2.7]"#, "VALUE 2"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", 9]"#, "VALUE -1"),
        (r#"["bytes/index-of", "aGVsbG8=", "aGVsbG8h"]"#, "VALUE -1"),
        (r#"["bytes/index-of", "", "YQ=="]"#, "VALUE -1"),
        (r#"["bytes/index-of", "", ""]"#, "VALUE 0"),
        (r#"["bytes/index-of", "%%", "YQ=="]"#, "REFUSED"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 1, "be"]"#, "VALUE 18"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 2, "be"]"#, "VALUE 4660"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 2, "le"]"#, "VALUE 13330"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 4, "be"]"#, "VALUE 305419896"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 4, "le"]"#, "VALUE 2018915346"),
        (r#"["bytes/read-uint", "EjRWeA==", 2, 2, "be"]"#, "VALUE 22136"),
        (r#"["bytes/read-uint", "EjRWeA==", 3, 2, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", -1, 1, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "", 0, 1, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 3, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 6, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 1, "middle"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", 0.5, 1, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "/////w==", 0, 4, "be"]"#, "VALUE 4294967295"),
        (r#"["bytes/read-uint", "AB////////8=", 0, 8, "be"]"#, "VALUE 9007199254740991"),
        (r#"["bytes/read-uint", "/x8AAAAAAAA=", 0, 8, "le"]"#, "VALUE 8191"),
        (r#"["bytes/read-uint", "ACAAAAAAAAA=", 0, 8, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "//////////8=", 0, 8, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 0, 1, "be"]"#, "VALUE \"AA==\""),
        (r#"["bytes/write-uint", 255, 1, "be"]"#, "VALUE \"/w==\""),
        (r#"["bytes/write-uint", 256, 1, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 4660, 2, "be"]"#, "VALUE \"EjQ=\""),
        (r#"["bytes/write-uint", 4660, 2, "le"]"#, "VALUE \"NBI=\""),
        (r#"["bytes/write-uint", 65535, 2, "le"]"#, "VALUE \"//8=\""),
        (r#"["bytes/write-uint", 65536, 2, "le"]"#, "NONE"),
        (r#"["bytes/write-uint", 305419896, 4, "be"]"#, "VALUE \"EjRWeA==\""),
        (r#"["bytes/write-uint", 4294967295, 4, "le"]"#, "VALUE \"/////w==\""),
        (r#"["bytes/write-uint", 4294967296, 4, "le"]"#, "NONE"),
        (r#"["bytes/write-uint", 0, 8, "le"]"#, "VALUE \"AAAAAAAAAAA=\""),
        (r#"["bytes/write-uint", 9007199254740991, 8, "be"]"#, "VALUE \"AB////////8=\""),
        (r#"["bytes/write-uint", 9007199254740991, 8, "le"]"#, "VALUE \"////////HwA=\""),
        (r#"["bytes/write-uint", 9007199254740992, 8, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", -1, 2, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 1.5, 2, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 1, 3, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 1, 6, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 1, 2, "big"]"#, "NONE"),
        (r#"["str/to-int", "0"]"#, "VALUE 0"),
        (r#"["str/to-int", "-0"]"#, "VALUE 0"),
        (r#"["str/to-int", "+5"]"#, "VALUE 5"),
        (r#"["str/to-int", "-42"]"#, "VALUE -42"),
        (r#"["str/to-int", "007"]"#, "VALUE 7"),
        (r#"["str/to-int", " 5"]"#, "NONE"),
        (r#"["str/to-int", "5 "]"#, "NONE"),
        (r#"["str/to-int", "5x"]"#, "NONE"),
        (r#"["str/to-int", ""]"#, "NONE"),
        (r#"["str/to-int", "+"]"#, "NONE"),
        (r#"["str/to-int", "1.0"]"#, "NONE"),
        (r#"["str/to-int", "1e3"]"#, "NONE"),
        (r#"["str/to-int", "0x10"]"#, "NONE"),
        (r#"["str/to-int", "9007199254740991"]"#, "VALUE 9007199254740991"),
        (r#"["str/to-int", "-9007199254740991"]"#, "VALUE -9007199254740991"),
        (r#"["str/to-int", "9007199254740992"]"#, "NONE"),
        (r#"["str/to-int", "9007199254740993"]"#, "NONE"),
        (r#"["str/to-int", "99999999999999999999"]"#, "NONE"),
    ];

    #[test]
    fn the_operators_answer_what_is_recorded() {
        let failures: Vec<String> = RECORDED
            .iter()
            .filter_map(|(form, want)| {
                let form: Json = serde_json::from_str(form).expect("form");
                let got = operator(&form);
                (got != *want).then(|| format!("{form}: recorded {want}, operator {got}"))
            })
            .collect();
        assert!(failures.is_empty(), "operators diverge from the recording:\n{}", failures.join("\n"));
    }
}

#[cfg(unix)]
mod finite_fs_completion {
    use orbital_core::runtime::inbound::InboundAnswer;
    use orbital_core::runtime::os_host::OsAnchors;
    use orbital_shell_rust::host::{boot::boot_file_with, HostConfig};
    use serde_json::Value as Json;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn run(name: &str, argv: &[&str]) -> (InboundAnswer, Vec<Json>) {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/finite-fs-completion").canonicalize().expect("fixtures");
        let source = fixtures.join(format!("{name}.lolo"));
        orbital_test_support::installed_core_language(&source);
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let schema = orbital_test_support::lower_lolo_file(&source).expect("lower fixture");
        let schema = orbital_compiler::phases::lazy::resolve_program_with_path(
            schema, &orbital_compiler::CompilerOptions::from_env(), Some(&source),
        ).expect("resolve fixture").0;
        let artifacts = std::env::temp_dir().join(format!(
            "almadar-finite-fs-{}-{}-{name}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir(&artifacts).expect("unique artifact directory");
        let program = artifacts.join("program.orb");
        std::fs::write(&program, serde_json::to_vec_pretty(&schema).expect("schema JSON")).expect("program artifact");
        let trace = artifacts.join("dispatches.jsonl");
        let host = boot_file_with(&program, &Default::default(), false, HostConfig {
            anchors: Some(OsAnchors { start: Some(fixtures.clone()), ..OsAnchors::of_process() }),
            ..Default::default()
        }).expect("boot native host");
        host.set_trace_file(&trace).expect("full dispatch trace");
        let body = serde_json::json!({ "argv": argv, "cwd": fixtures }).to_string();
        let answer = host.deliver_inbound_awaiting(
            "cli-boundary", std::iter::empty::<(&str, &str)>(), &body, Some(Duration::from_secs(30)),
        );
        let entries = std::fs::read_to_string(&trace).expect("read full dispatch trace")
            .lines().map(|line| serde_json::from_str(line).expect("trace JSON")).collect();
        eprintln!("finite FS {name}: status {}, complete trace {}", answer.status, trace.display());
        (answer, entries)
    }

    fn event_dispatches(entries: &[Json], event: &str) -> Vec<u64> {
        entries.iter().flat_map(|entry| {
            entry["outcomes"].as_array().expect("trace outcomes").iter()
                .filter(move |outcome| outcome["event"] == event && outcome["executed"] == true)
                .map(move |_| entry["dispatchId"].as_u64().expect("dispatch id"))
        }).collect()
    }

    fn keyed_finishes(name: &str) {
        let (answer, entries) = run(name, &[]);
        assert_eq!(answer.status, 200, "{name}: {}", answer.body);
        assert_eq!(answer.body, "2105");
        let answers = event_dispatches(&entries, "ANSWERED");
        assert_eq!(answers.len(), 2105, "each actual filesystem ask must answer");
        assert_eq!(answers.iter().collect::<BTreeSet<_>>().len(), 2105, "each keyed completion owns a fresh dispatch budget");
        let called = event_dispatches(&entries, "CALLED");
        assert_eq!(called.len(), 1);
        assert!(answers.iter().all(|id| *id != called[0]));
        assert!(entries.iter().all(|entry| entry["error"].is_null()));
        assert!(event_dispatches(&entries, "FAILED").is_empty());
    }

    fn inline_is_capped(name: &str) {
        let (answer, entries) = run(name, &[]);
        assert_eq!(answer.status, 500, "{name}: {}", answer.body);
        let body: Json = serde_json::from_str(&answer.body).expect("structured cascade refusal");
        assert_eq!(body["code"], "cascade-cap");
        assert_eq!(body["cap"], 2000);
        assert!(event_dispatches(&entries, "FINISHED").is_empty());
        assert!(!event_dispatches(&entries, "ANSWERED").is_empty(), "the fixture must execute real inline filesystem work before the cap");
    }

    #[test]
    fn finite_fs_completion_keyed_stat_finishes_beyond_one_dispatch_budget() { keyed_finishes("stat-keyed"); }
    #[test]
    fn finite_fs_completion_keyed_read_file_finishes_beyond_one_dispatch_budget() { keyed_finishes("read-file-keyed"); }
    #[test]
    fn finite_fs_completion_keyed_list_dir_finishes_beyond_one_dispatch_budget() { keyed_finishes("list-dir-keyed"); }
    #[test]
    fn finite_fs_completion_inline_stat_keeps_the_dispatch_budget() { inline_is_capped("stat-inline"); }
    #[test]
    fn finite_fs_completion_inline_read_file_keeps_the_dispatch_budget() { inline_is_capped("read-file-inline"); }
    #[test]
    fn finite_fs_completion_inline_list_dir_keeps_the_dispatch_budget() { inline_is_capped("list-dir-inline"); }

    #[test]
    fn finite_fs_completion_anchor_refusal_preserves_preparation_error_and_inline_route() {
        let (inline, inline_trace) = run("stat-inline-refused", &["../denied-probe.txt"]);
        let (keyed, keyed_trace) = run("stat-keyed-refused", &["../denied-probe.txt"]);
        assert_eq!(inline.status, 500);
        assert_eq!(keyed.status, 500);
        assert_eq!(keyed.body, inline.body);
        assert!(inline.body.starts_with("os/stat:"));
        assert!(inline.body.contains("stays under it"));
        for entries in [&inline_trace, &keyed_trace] {
            assert!(event_dispatches(entries, "ANSWERED").is_empty());
            let failed = event_dispatches(entries, "FAILED");
            assert_eq!(failed.len(), 1);
            assert_eq!(failed, event_dispatches(entries, "CALLED"));
        }
    }

    #[test]
    fn finite_fs_completion_grant_refusal_preserves_error_on_the_existing_failure_route() {
        let (inline, inline_trace) = run("stat-inline-ungranted", &["denied-probe.txt"]);
        let (keyed, keyed_trace) = run("stat-keyed-ungranted", &["denied-probe.txt"]);
        assert_eq!(inline.status, 500);
        assert_eq!(keyed.status, 500);
        assert_eq!(keyed.body, inline.body);
        assert!(inline.body.starts_with("os/stat:"));
        assert!(inline.body.contains("is not granted for Read"));
        assert_eq!(event_dispatches(&inline_trace, "FAILED"), event_dispatches(&inline_trace, "CALLED"));
        let failed = event_dispatches(&keyed_trace, "FAILED");
        assert_eq!(failed.len(), 1);
        assert_ne!(failed, event_dispatches(&keyed_trace, "CALLED"));
        assert!(event_dispatches(&keyed_trace, "ANSWERED").is_empty());
    }
}
