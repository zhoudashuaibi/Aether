use super::{
    any, build_router_with_state, start_server, to_bytes, AppState, Body, Request, Response,
    Router, StatusCode,
};
use std::sync::{Arc, Mutex};

use aether_crypto::{encrypt_python_fernet_plaintext, DEVELOPMENT_ENCRYPTION_KEY};
use aether_data::repository::{
    auth::{InMemoryAuthApiKeySnapshotRepository, StoredAuthApiKeySnapshot},
    candidate_selection::InMemoryMinimalCandidateSelectionReadRepository,
    candidates::InMemoryRequestCandidateRepository,
    provider_catalog::InMemoryProviderCatalogReadRepository,
};
use aether_data_contracts::repository::{
    candidate_selection::StoredMinimalCandidateSelectionRow,
    provider_catalog::{
        StoredProviderCatalogEndpoint, StoredProviderCatalogKey, StoredProviderCatalogProvider,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn candidate() -> StoredMinimalCandidateSelectionRow {
    StoredMinimalCandidateSelectionRow {
        provider_id: "command-provider".to_string(),
        provider_name: "Command Code".to_string(),
        provider_type: "command_code".to_string(),
        provider_priority: 1,
        provider_is_active: true,
        endpoint_id: "command-endpoint".to_string(),
        endpoint_api_format: "openai:chat".to_string(),
        endpoint_api_family: Some("openai".to_string()),
        endpoint_kind: Some("chat".to_string()),
        endpoint_is_active: true,
        key_id: "command-key".to_string(),
        key_name: "test".to_string(),
        key_auth_type: "bearer".to_string(),
        key_is_active: true,
        key_api_formats: Some(vec!["openai:chat".to_string()]),
        key_allowed_models: None,
        key_capabilities: None,
        key_internal_priority: 1,
        key_global_priority_by_format: None,
        model_id: "command-model".to_string(),
        global_model_id: "command-global".to_string(),
        global_model_name: "command-test".to_string(),
        global_model_mappings: None,
        global_model_supports_streaming: Some(true),
        model_provider_model_name: "command-upstream".to_string(),
        model_provider_model_mappings: None,
        model_supports_streaming: Some(true),
        model_is_active: true,
        model_is_available: true,
    }
}

fn state(provider_url: String) -> AppState {
    let auth = StoredAuthApiKeySnapshot::new(
        "command-user".to_string(),
        "tester".to_string(),
        None,
        "user".to_string(),
        "local".to_string(),
        true,
        false,
        None,
        None,
        None,
        "command-client-key".to_string(),
        Some("test".to_string()),
        true,
        false,
        false,
        None,
        None,
        Some(4_102_444_800),
        None,
        None,
        None,
    )
    .unwrap();
    let provider = StoredProviderCatalogProvider::new(
        "command-provider".to_string(),
        "Command Code".to_string(),
        None,
        "command_code".to_string(),
    )
    .unwrap()
    .with_transport_fields(
        true,
        false,
        true,
        None,
        Some(0),
        None,
        Some(10.0),
        Some(5.0),
        None,
    );
    let endpoint = StoredProviderCatalogEndpoint::new(
        "command-endpoint".to_string(),
        "command-provider".to_string(),
        "openai:chat".to_string(),
        Some("openai".to_string()),
        Some("chat".to_string()),
        true,
    )
    .unwrap()
    .with_transport_fields(
        provider_url,
        None,
        None,
        Some(0),
        Some("/alpha/generate".to_string()),
        // The private wire protocol must remain streaming even with this override.
        Some(json!({"upstream_stream_policy":"force_non_stream"})),
        None,
        None,
    )
    .unwrap();
    let key = StoredProviderCatalogKey::new(
        "command-key".to_string(),
        "command-provider".to_string(),
        "test".to_string(),
        "bearer".to_string(),
        None,
        true,
    )
    .unwrap()
    .with_transport_fields(
        Some(json!(["openai:chat"])),
        encrypt_python_fernet_plaintext(DEVELOPMENT_ENCRYPTION_KEY, "user_command_fixture")
            .unwrap(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let data = crate::data::GatewayDataState::with_auth_candidate_selection_provider_catalog_and_request_candidate_repository_for_tests(
        Arc::new(InMemoryAuthApiKeySnapshotRepository::seed(vec![(Some(format!("{:x}", Sha256::digest(b"sk-command-client"))), auth)])),
        Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![candidate()])),
        Arc::new(InMemoryProviderCatalogReadRepository::seed(vec![provider], vec![endpoint], vec![key])),
        Arc::new(InMemoryRequestCandidateRepository::default()), DEVELOPMENT_ENCRYPTION_KEY,
    );
    AppState::new().unwrap().with_data_state_for_tests(data)
}

// Use the real planner, transport and response finalizers. No external services
// or credentials are needed; the only upstream is an ephemeral local fixture.
#[test]
fn command_code_serves_three_public_apis_in_sync_and_stream_modes() {
    let thread = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_contract());
        })
        .unwrap();
    if let Err(error) = thread.join() {
        std::panic::resume_unwind(error);
    }
}

async fn run_contract() {
    let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let captured = Arc::clone(&seen);
    let upstream = Router::new().fallback(any(move |request: Request| {
        let captured = Arc::clone(&captured);
        async move {
            let (parts, body) = request.into_parts();
            assert_eq!(parts.headers["authorization"], "Bearer user_command_fixture");
            assert!(!parts.headers.contains_key("x-aether-command-code-runtime"));
            let payload: Value = serde_json::from_slice(&to_bytes(body, 1024 * 1024).await.unwrap()).unwrap();
            let path = parts.uri.path().to_string();
            if path == "/alpha/generate" {
                assert_eq!(payload["params"]["stream"], true);
                assert_eq!(payload["params"]["model"], "command-upstream");
                assert_eq!(parts.headers["x-session-id"].to_str().unwrap(), payload["threadId"].as_str().unwrap());
            }
            captured.lock().unwrap().push((path.clone(), payload));
            if path != "/alpha/generate" {
                // Optional initialization failure must not prevent generation,
                // and its cooldown must prevent a request storm.
                return Response::builder().status(StatusCode::SERVICE_UNAVAILABLE).body(Body::empty()).unwrap();
            }
            Response::builder().header("content-type", "application/x-ndjson").body(Body::from(concat!(
                "{\"type\":\"text-delta\",\"text\":\"command fixture reply\"}\n",
                "{\"type\":\"finish\",\"finishReason\":\"stop\",\"totalUsage\":{\"inputTokens\":10,\"outputTokens\":3,\"cachedInputTokens\":4}}\n"
            ))).unwrap()
        }
    }));
    let (provider_url, provider_handle) = start_server(upstream).await;
    let (gateway_url, gateway_handle) =
        start_server(build_router_with_state(state(provider_url))).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap();
    for path in ["/v1/chat/completions", "/v1/responses", "/v1/messages"] {
        for stream in [false, true] {
            let mut body = json!({"model":"command-test", "stream":stream});
            if path == "/v1/responses" {
                body["input"] = json!("hello");
                body["max_output_tokens"] = json!(100);
            } else {
                body["messages"] = json!([{"role":"user", "content":"hello"}]);
                body["max_tokens"] = json!(100);
            }
            let response = client
                .post(format!("{gateway_url}{path}"))
                .bearer_auth("sk-command-client")
                .header("x-session-id", "command-fixture-session")
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let output = response.text().await.unwrap();
            assert_eq!(status, StatusCode::OK, "{path} stream={stream}: {output}");
            assert!(
                output.contains("command fixture reply"),
                "{path} stream={stream}: {output}"
            );
            if stream {
                let terminal = match path {
                    "/v1/responses" => "response.completed",
                    "/v1/messages" => "message_stop",
                    _ => "[DONE]",
                };
                assert!(output.contains(terminal), "{output}");
            } else {
                let output: Value = serde_json::from_str(&output).unwrap();
                let usage = &output["usage"];
                match path {
                    "/v1/chat/completions" => assert_eq!(usage["prompt_tokens"], 10),
                    "/v1/responses" => assert_eq!(usage["input_tokens"], 10),
                    _ => {
                        assert_eq!(usage["input_tokens"], 6);
                        assert_eq!(usage["cache_read_input_tokens"], 4);
                    }
                }
            }
        }
    }
    let rejected = client.post(format!("{gateway_url}/v1/chat/completions")).bearer_auth("sk-command-client")
        .json(&json!({"model":"command-test", "messages":[{"role":"user","content":"hi"}], "top_p":0.8}))
        .send().await.unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.iter()
                .filter(|(path, _)| path == "/alpha/generate")
                .count(),
            6
        );
        assert_eq!(
            seen.iter()
                .filter(|(path, _)| path == "/alpha/fingerprint/record")
                .count(),
            1
        );
        assert_eq!(
            seen.iter()
                .filter(|(path, _)| path == "/alpha/lifecycle-events")
                .count(),
            1
        );
        let sessions = seen
            .iter()
            .filter(|(path, _)| path == "/alpha/generate")
            .map(|(_, body)| body["threadId"].as_str().unwrap())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(sessions.len(), 1);
    }
    gateway_handle.abort();
    provider_handle.abort();
}
