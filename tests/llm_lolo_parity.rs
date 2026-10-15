// H.6 parity: every recorded llm fixture the atom can express, replayed through the kernel against `std-llm.lolo`,
// makes the request the manual provider made (method, URL, compared headers, body) and emits the recorded outcome.
// A fixture records the provider's env-driven configuration; the atom takes it as config knobs, so each fixture's
// env is translated into the knob env first and its `{base}` mock host into the provider's real origin.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};
use serde_json::{json, Value as Json};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-llm.lolo",
    trait_name: "LlmClient",
    fixtures_dir: "llm",
    events: &[
        ActionEvents { action: "generate", input: "LLM_GENERATE", result: "LLM_GENERATE_RESULT", failure: Some("LLM_FAILED") },
        ActionEvents { action: "classify", input: "LLM_CLASSIFY", result: "LLM_CLASSIFY_RESULT", failure: Some("LLM_FAILED") },
        ActionEvents { action: "classifyMany", input: "LLM_CLASSIFY_MANY", result: "LLM_CLASSIFY_MANY_RESULT", failure: Some("LLM_FAILED") },
        ActionEvents { action: "summarize", input: "LLM_SUMMARIZE", result: "LLM_SUMMARIZE_RESULT", failure: Some("LLM_FAILED") },
        ActionEvents { action: "embed", input: "LLM_EMBED", result: "LLM_EMBED_RESULT", failure: Some("LLM_FAILED") },
        ActionEvents { action: "choose", input: "LLM_CHOOSE", result: "LLM_CHOOSE_RESULT", failure: Some("LLM_FAILED") },
    ],
    env_knobs: &[
        ("KNOB_PROVIDER", "provider"),
        ("KNOB_ORIGIN", "origin"),
        ("KNOB_BASE_PATH", "basePath"),
        ("KNOB_MODEL", "model"),
        ("KNOB_API_KEY", "apiKey"),
        ("KNOB_TIMEOUT_MS", "timeoutMs"),
        ("KNOB_EMBED_PROVIDER", "embedProvider"),
        ("KNOB_EMBED_ORIGIN", "embedOrigin"),
        ("KNOB_EMBED_BASE_PATH", "embedBasePath"),
        ("KNOB_EMBED_MODEL", "embedModel"),
        ("KNOB_EMBED_API_KEY", "embedApiKey"),
        ("KNOB_JEV_API_KEY", "jevApiKey"),
    ],
    secret_env: &["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "DEEPSEEK_API_KEY", "KIMI_API_KEY", "OPEN_ROUTER_API_KEY", "EMBEDDING_API_KEY"],
    compared_headers: &["content-type", "accept", "authorization", "x-api-key", "anthropic-version", "anthropic-beta", "openai-organization", "user-agent"],
    default_timeout_ms: 300_000,
    base: None,
};

// D-A80: the registry param checks (nothing sent). `openai-classify-many-no-categories` is replayed too, as a failure the atom answers itself.
const VALIDATION_ONLY: [&str; 7] = [
    "jev-choose-missing-state",
    "jev-choose-options-not-an-array",
    "openai-classify-many-no-categories",
    "unknown-action",
    "validation-missing-user-prompt",
    "validation-unknown-param",
    "validation-wrong-type",
];
// A typed event payload rejects these before the atom runs (a stronger rung than a message).
const TYPED_PAYLOAD: [&str; 1] = ["jev-choose-option-value-not-a-string"];
// Not ported: the reply is shaped by the caller's JSON schema, which no event payload type can name.
const NOT_PORTED: [&str; 1] = ["openai-extract"];

struct Provider {
    origin: &'static str,
    base_path: &'static str,
    model: &'static str,
    key: Option<&'static str>,
}

fn provider(name: &str) -> Provider {
    match name {
        "anthropic" => Provider { origin: "https://api.anthropic.com", base_path: "/v1", model: "claude-sonnet-4-5-20250929", key: Some("ANTHROPIC_API_KEY") },
        "deepseek" => Provider { origin: "https://api.deepseek.com", base_path: "/v1", model: "deepseek-v4-flash", key: Some("DEEPSEEK_API_KEY") },
        "kimi" => Provider { origin: "https://api.moonshot.ai", base_path: "/v1", model: "kimi-k2.5", key: Some("KIMI_API_KEY") },
        "openrouter" => Provider { origin: "https://openrouter.ai", base_path: "/api/v1", model: "qwen/qwen-2.5-72b-instruct", key: Some("OPEN_ROUTER_API_KEY") },
        "lmstudio" => Provider { origin: "http://127.0.0.1:1234", base_path: "/v1", model: "qwen3-30b-a3b", key: None },
        _ => Provider { origin: "https://api.openai.com", base_path: "/v1", model: "gpt-4o", key: Some("OPENAI_API_KEY") },
    }
}

// The manual provider's own selection rule (LLM_PROVIDER, else the first hosted key in priority order),
// applied to the recorded env so the atom's `provider` knob is set to what the provider would have chosen.
fn chosen_provider(env: &Json) -> &'static str {
    if let Some(name) = env["LLM_PROVIDER"].as_str() {
        return ["anthropic", "deepseek", "kimi", "openai", "openrouter", "lmstudio"].into_iter().find(|p| *p == name).expect("fixture provider");
    }
    [("anthropic", "ANTHROPIC_API_KEY"), ("deepseek", "DEEPSEEK_API_KEY"), ("kimi", "KIMI_API_KEY"), ("openai", "OPENAI_API_KEY"), ("openrouter", "OPEN_ROUTER_API_KEY")]
        .into_iter()
        .find(|(_, var)| env[*var].as_str().is_some())
        .map_or("openai", |(name, _)| name)
}

fn translate(fixture: &Json) -> Json {
    let env = &fixture["env"];
    let action = fixture["action"].as_str().expect("action");
    let mut knobs = serde_json::Map::new();
    let mock_origin = match action {
        "embed" => match env["EMBEDDING_PROVIDER"].as_str() {
            Some("lmstudio") => "http://127.0.0.1:1234",
            Some("openai") => "https://api.openai.com",
            _ => "https://openrouter.ai",
        },
        "choose" => "https://openrouter.ai",
        _ => provider(chosen_provider(env)).origin,
    };
    if action == "embed" {
        let name = env["EMBEDDING_PROVIDER"].as_str().unwrap_or("openrouter");
        let (origin, base_path, key) = match name {
            "openai" => ("https://api.openai.com", "/v1", "OPENAI_API_KEY"),
            "lmstudio" => ("http://127.0.0.1:1234", "/v1", "EMBEDDING_API_KEY"),
            _ => ("https://openrouter.ai", "/api/v1", "OPEN_ROUTER_API_KEY"),
        };
        knobs.insert("KNOB_EMBED_PROVIDER".into(), json!(name));
        knobs.insert("KNOB_EMBED_ORIGIN".into(), json!(origin));
        knobs.insert("KNOB_EMBED_BASE_PATH".into(), json!(base_path));
        knobs.insert("KNOB_EMBED_API_KEY".into(), json!(if env["EMBEDDING_API_KEY"].is_string() { "EMBEDDING_API_KEY" } else { key }));
        if let Some(model) = env["EMBEDDING_MODEL"].as_str() {
            knobs.insert("KNOB_EMBED_MODEL".into(), json!(model));
        }
    } else if action == "choose" {
        knobs.insert("KNOB_TIMEOUT_MS".into(), json!("30000"));
    } else {
        let name = chosen_provider(env);
        let p = provider(name);
        knobs.insert("KNOB_PROVIDER".into(), json!(name));
        knobs.insert("KNOB_ORIGIN".into(), json!(p.origin));
        knobs.insert("KNOB_BASE_PATH".into(), json!(p.base_path));
        knobs.insert("KNOB_MODEL".into(), json!(p.model));
        if let Some(key) = p.key {
            knobs.insert("KNOB_API_KEY".into(), json!(key));
        }
        let timeout = if action == "generate" && name == "lmstudio" { 1_200_000 } else { 300_000 };
        knobs.insert("KNOB_TIMEOUT_MS".into(), json!(timeout.to_string()));
    }
    let mut translated = fixture.clone();
    for (name, value) in knobs {
        translated["env"][name] = value;
    }
    let text = translated.to_string().replace("{base}", mock_origin);
    serde_json::from_str(&text).expect("translated fixture")
}

fn replayable() -> Vec<Json> {
    lolo_parity::fixtures(&SPEC)
        .iter()
        .filter(|f| {
            let case = lolo_parity::text(f, "case");
            !VALIDATION_ONLY.contains(&case) || case == "openai-classify-many-no-categories"
        })
        .filter(|f| {
            let case = lolo_parity::text(f, "case");
            !TYPED_PAYLOAD.contains(&case) && !NOT_PORTED.contains(&case)
        })
        .map(translate)
        .collect()
}

fn only() -> Option<Vec<String>> {
    std::env::var("LLM_PARITY_ONLY").ok().map(|v| v.split(',').map(str::to_string).collect())
}

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 44, "the recorded llm cases");
    let cases = replayable();
    let mut failed: Vec<String> = Vec::new();
    for fixture in &cases {
        if only().is_some_and(|names| !names.iter().any(|n| n == lolo_parity::text(fixture, "case"))) {
            continue;
        }
        let case = lolo_parity::text(fixture, "case").to_string();
        if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lolo_parity::check(&SPEC, fixture))) {
            let message = panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            failed.push(format!("{case}: {message}"));
        }
    }
    assert!(failed.is_empty(), "failing cases:\n{}", failed.join("\n"));
    assert_eq!(cases.len(), 44 - VALIDATION_ONLY.len() - TYPED_PAYLOAD.len() - NOT_PORTED.len() + 1);
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = translate(&lolo_parity::fixture_named(&SPEC, "openai-generate"));
    let url = lolo_parity::text(&fixture["exchanges"][0]["request"], "url").replace("/chat/completions", "/chat/completion");
    fixture["exchanges"][0]["request"]["url"] = json!(url);
    lolo_parity::check(&SPEC, &fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = translate(&lolo_parity::fixture_named(&SPEC, "openai-generate"));
    fixture["outcome"]["data"]["content"] = json!("NOT OK");
    lolo_parity::check(&SPEC, &fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
