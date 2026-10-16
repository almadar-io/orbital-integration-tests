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
        atom: "almadar-std/behaviors/lolo/infra/atoms/std-iconify.lolo",
        trait_name: "Harness",
        fixtures_dir: "iconify",
        events: &[
            ActionEvents {
                action: "status",
                input: "STATUS",
                result: "RESULT",
                failure: Some("FAILED"),
            },
            ActionEvents {
                action: "forget",
                input: "FORGET",
                result: "RESULT",
                failure: Some("FAILED"),
            },
            ActionEvents {
                action: "random",
                input: "RANDOM",
                result: "RESULT",
                failure: Some("FAILED"),
            },
            ActionEvents {
                action: "request",
                input: "REQUEST",
                result: "RESULT",
                failure: Some("FAILED"),
            },
        ],
        env_knobs: &[],
        secret_env: &["TOKEN"],
        compared_headers: &["authorization", "content-type"],
        default_timeout_ms: 1000,
        base: None,
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
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-iconify.lolo",
    trait_name: "Harness",
    fixtures_dir: "iconify",
    events: &[],
    env_knobs: &[],
    secret_env: &["TOKEN"],
    compared_headers: &[],
    default_timeout_ms: 1000,
    base: None,
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
    assert_eq!(answers[0], IoAnswer::CredentialStatus(Ok(true)));
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
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-iconify.lolo",
    trait_name: "Harness",
    fixtures_dir: "iconify",
    events: &[
        lolo_parity::ActionEvents { action: "status", input: "STATUS", result: "RESULT", failure: Some("FAILED") },
        lolo_parity::ActionEvents { action: "forget", input: "FORGET", result: "RESULT", failure: Some("FAILED") },
        lolo_parity::ActionEvents { action: "random", input: "RANDOM", result: "RESULT", failure: Some("FAILED") },
    ],
    env_knobs: &[],
    secret_env: &["TOKEN"],
    compared_headers: &[],
    default_timeout_ms: 1000,
    base: None,
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
    let scenario = steps_scenario(vec![credential_step("status", true), credential_step("forget", true), credential_step("status", false)]);
    let runs = alice_session(&scenario, None).check_steps(&scenario);
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[2].1.results, vec![json!({ "connected": false })]);
}

#[test]
#[should_panic(expected = "emitted result")]
fn steps_control_a_step_that_expects_the_unforgotten_state_fails() {
    let scenario = steps_scenario(vec![credential_step("status", true), credential_step("forget", true), credential_step("status", true)]);
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
