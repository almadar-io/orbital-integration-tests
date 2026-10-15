#[path = "common/lolo_parity.rs"]
mod lolo_parity;
#[test]
fn dbg() {
    use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoAnswer, IoRequest};
    use orbital_core::runtime::{EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{RuntimeEnvironment, Value};
    use std::sync::Arc;
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages/almadar-std/behaviors/lolo/infra/atoms/std-stripe.lolo");
    let schema = orbital_test_support::lower_lolo_file(&path).unwrap();
    let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
    let kernel = RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), KernelConfig::default());
    kernel.register_schema(schema).unwrap();
    let payload: Value = serde_json::from_value(serde_json::json!({"subscriptionId":"sub_1"})).unwrap();
    let progress = kernel.start_dispatch("StripeClient", None, "STRIPE_GET_SUBSCRIPTION", payload, None, None).unwrap();
    let answer = |r: &IoRequest| match r {
        IoRequest::Http { request, .. } => { eprintln!("REQ {} {}", request.method, request.url);
            IoAnswer::Http(Ok(orbital_core::runtime::http_service::HttpResponse { status: 200, headers: Default::default(), body: br#"{"id":"sub_1","customer":"cus_1","status":"active","cancel_at_period_end":false,"current_period_start":1767225600,"current_period_end":1769904000,"items":{"data":[{"id":"si_1","quantity":1,"price":{"id":"p"}}]}}"#.to_vec() })) }
        IoRequest::Secret { .. } => IoAnswer::Secret(Ok(Some("test-key".into()))),
        other => BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(other),
    };
    let out = drive_blocking(&kernel, progress, &answer).unwrap();
    for (n, o) in &out { eprintln!("OUTCOME {n}: failure={:?}", o.failure); for e in &o.effect_results { eprintln!("   {:?}", e.effect); } }
}
