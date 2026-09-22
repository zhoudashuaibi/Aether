use super::*;

#[tokio::test]
async fn gateway_creates_and_updates_command_code_provider_with_fixed_endpoint() {
    let repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ));
    let gateway = build_router_with_state(AppState::new().unwrap().with_data_state_for_tests(
        GatewayDataState::with_provider_catalog_repository_for_tests(Arc::clone(&repository)),
    ));
    let (gateway_url, gateway_handle) = start_server(gateway).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{gateway_url}/api/admin/providers/"))
        .headers(trusted_admin_headers())
        .json(&json!({
            "name": "command-code-provider",
            "provider_type": "command_code",
            "stream_first_byte_timeout": 30,
            "request_timeout": 300,
            "max_transfer_count": 0,
            "max_transfer_timeout_seconds": 0
        }))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
    let provider_id = payload["id"].as_str().unwrap().to_string();
    let providers = repository.list_providers(false).await.unwrap();
    let provider = providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .unwrap();
    assert_eq!(provider.provider_type, "command_code");
    assert!(provider.enable_format_conversion);
    let endpoints = repository
        .list_endpoints_by_provider_ids(std::slice::from_ref(&provider_id))
        .await
        .unwrap();
    assert_eq!(endpoints.len(), 1);
    let endpoint = &endpoints[0];
    assert_eq!(endpoint.api_format, "openai:chat");
    assert_eq!(endpoint.base_url, "https://api.commandcode.ai");
    assert_eq!(endpoint.custom_path.as_deref(), Some("/alpha/generate"));
    assert_eq!(
        endpoint.config.as_ref().unwrap()["upstream_stream_policy"],
        "force_stream"
    );

    let response = client
        .patch(format!("{gateway_url}/api/admin/providers/{provider_id}"))
        .headers(trusted_admin_headers())
        .json(&json!({"name": "command-code-updated", "provider_type": " Command_Code "}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let providers = repository.list_providers(false).await.unwrap();
    let provider = providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .unwrap();
    assert_eq!(provider.name, "command-code-updated");
    assert_eq!(provider.provider_type, "command_code");
    let updated_endpoints = repository
        .list_endpoints_by_provider_ids(std::slice::from_ref(&provider_id))
        .await
        .unwrap();
    assert_eq!(updated_endpoints.len(), 1);
    assert_eq!(updated_endpoints[0].id, endpoint.id);
    assert_eq!(updated_endpoints[0].custom_path, endpoint.custom_path);
    gateway_handle.abort();
}
