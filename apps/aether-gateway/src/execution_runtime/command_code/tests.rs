use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::{body::Body, extract::Request, response::Response, routing::any, Router};

fn plan(base: &str) -> ExecutionPlan {
    ExecutionPlan {
        request_id: "command-init-test".to_string(),
        candidate_id: None,
        provider_name: None,
        provider_id: "provider".to_string(),
        endpoint_id: "endpoint".to_string(),
        key_id: "key".to_string(),
        method: "POST".to_string(),
        url: format!("{base}{GENERATE_PATH}"),
        headers: std::collections::BTreeMap::from([
            (INTERNAL_HEADER.to_string(), "1".to_string()),
            (
                "authorization".to_string(),
                "Bearer user_init_fixture".to_string(),
            ),
            ("content-type".to_string(), "application/json".to_string()),
            (
                "x-command-code-version".to_string(),
                CLI_VERSION.to_string(),
            ),
        ]),
        content_type: Some("application/json".to_string()),
        content_encoding: None,
        body: RequestBody::from_json(json!({})),
        stream: true,
        client_api_format: "openai:chat".to_string(),
        provider_api_format: "openai:chat".to_string(),
        model_name: None,
        proxy: None,
        transport_profile: None,
        timeouts: None,
    }
}

#[test]
fn shared_transport_initialization_future_has_a_bounded_stack_footprint() {
    let plan = plan("https://example.invalid");
    let initialization = ensure_initialized(&plan, None);
    assert!(std::mem::size_of_val(&initialization) <= 1024);
}

#[tokio::test]
async fn initialization_is_single_flight_and_success_is_cached() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = Arc::clone(&calls);
    let app = Router::new().fallback(any(move |request: Request| {
        let captured = Arc::clone(&captured);
        async move {
            captured.fetch_add(1, Ordering::SeqCst);
            assert!(matches!(
                request.uri().path(),
                "/alpha/fingerprint/record" | "/alpha/lifecycle-events"
            ));
            assert_eq!(
                request.headers()["authorization"],
                "Bearer user_init_fixture"
            );
            assert!(!request.headers().contains_key(INTERNAL_HEADER));
            let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(body.is_object());
            tokio::time::sleep(Duration::from_millis(20)).await;
            Response::builder().status(200).body(Body::empty()).unwrap()
        }
    }));
    let (base, server) = crate::tests::start_server(app).await;
    let plan = plan(&base);
    tokio::join!(
        ensure_initialized(&plan, None),
        ensure_initialized(&plan, None),
        ensure_initialized(&plan, None)
    );
    ensure_initialized(&plan, None).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
async fn cancelled_initialization_does_not_hold_the_key_lock() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = Arc::clone(&calls);
    let app = Router::new().fallback(any(move || {
        let captured = Arc::clone(&captured);
        async move {
            if captured.fetch_add(1, Ordering::SeqCst) < 2 {
                std::future::pending::<()>().await;
            }
            Response::builder().status(200).body(Body::empty()).unwrap()
        }
    }));
    let (base, server) = crate::tests::start_server(app).await;
    let plan = plan(&base);
    {
        let initialization = ensure_initialized(&plan, None);
        tokio::pin!(initialization);
        tokio::select! {
            () = &mut initialization => panic!("fixture should block initialization"),
            () = crate::tests::wait_until(2000, || calls.load(Ordering::SeqCst) == 2) => {}
        }
    }
    tokio::time::timeout(Duration::from_secs(2), ensure_initialized(&plan, None))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    server.abort();
}
