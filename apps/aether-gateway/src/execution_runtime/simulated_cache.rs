use aether_contracts::{ExecutionPlan, ExecutionStreamTerminalSummary, StandardizedUsage};
use serde_json::Value;
use tracing::warn;

use super::kiro_cache::{
    estimate_simulated_cache_input_tokens, seed_simulated_cache_mode_in_report_context,
    simulated_cache_mode_from_provider_config, SimulatedCacheMode,
    SIMULATED_CACHE_MODULE_ENABLED_KEY,
};
use crate::ai_serving::api::{
    standardized_gross_input_tokens, supports_simulated_cache, SimulatedCachePolicy,
};
use crate::AppState;

pub(crate) fn request_supports_simulated_cache(
    plan: &ExecutionPlan,
    context: Option<&Value>,
) -> bool {
    if !supports_simulated_cache(&plan.client_api_format)
        || !supports_simulated_cache(&plan.provider_api_format)
    {
        return false;
    }
    if context
        .and_then(|c| c.get("image_request"))
        .is_some_and(|v| !v.is_null())
    {
        return false;
    }
    let requests = [
        context.and_then(|c| c.get("original_request_body")),
        context.and_then(|c| c.get("provider_request_body")),
        plan.body.json_body.as_ref(),
    ];
    !requests.into_iter().flatten().any(request_generates_media)
}

fn request_generates_media(body: &Value) -> bool {
    if body
        .get("tool_choice")
        .and_then(|v| v.get("type"))
        .and_then(Value::as_str)
        == Some("image_generation")
    {
        return true;
    }
    [
        body.get("modalities"),
        body.get("response_modalities"),
        body.pointer("/generationConfig/responseModalities"),
        body.pointer("/generation_config/response_modalities"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_array)
    .flatten()
    .filter_map(Value::as_str)
    .any(|modality| {
        matches!(
            modality.to_ascii_lowercase().as_str(),
            "image" | "audio" | "video"
        )
    })
}

pub(crate) fn seed_report_context_input_tokens(
    _plan: &ExecutionPlan,
    report_context: &mut Option<Value>,
) {
    let Some(context) = report_context.as_mut().and_then(Value::as_object_mut) else {
        return;
    };
    if context
        .get("input_tokens")
        .and_then(Value::as_u64)
        .is_some_and(|tokens| tokens > 0)
    {
        return;
    }
    if let Some(body) = context.get("original_request_body") {
        let input = estimate_simulated_cache_input_tokens(body);
        context.insert("input_tokens".into(), Value::from(input));
    }
}

pub(crate) fn seed_report_context_simulated_cache_usage(report_context: &mut Option<Value>) {
    let Some(policy) = SimulatedCachePolicy::from_report_context(report_context.as_ref()) else {
        return;
    };
    let Some(context) = report_context.as_mut().and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(input) = context.get("input_tokens").and_then(Value::as_u64) {
        context.insert(
            "cache_read_input_tokens".into(),
            Value::from(policy.cache_read_tokens(input)),
        );
    }
}

pub(crate) fn apply_simulated_cache_to_summary(
    api_format: &str,
    context: Option<&Value>,
    summary: &mut ExecutionStreamTerminalSummary,
) {
    let Some(policy) = SimulatedCachePolicy::from_report_context(context) else {
        return;
    };
    let Some(gross) = summary
        .standardized_usage
        .as_ref()
        .map(|usage| standardized_gross_input_tokens(usage, api_format))
        .or_else(|| {
            context
                .and_then(|c| c.get("input_tokens"))
                .and_then(Value::as_u64)
                .filter(|tokens| *tokens > 0)
        })
    else {
        return;
    };
    policy.apply_to_usage(
        summary
            .standardized_usage
            .get_or_insert_with(StandardizedUsage::new),
        api_format,
        gross,
    );
}

pub(crate) async fn seed_simulated_cache_config(
    state: &AppState,
    plan: &ExecutionPlan,
    report_context: &mut Option<Value>,
) {
    if !request_supports_simulated_cache(plan, report_context.as_ref()) {
        seed_simulated_cache_mode_in_report_context(report_context, SimulatedCacheMode::Disabled);
        return;
    }
    let provider_scope = format!(
        "{}:{}:{}:{}",
        plan.request_id, plan.provider_id, plan.endpoint_id, plan.key_id
    );
    if report_context
        .as_ref()
        .and_then(|context| context.get("simulated_cache_provider_scope"))
        .and_then(Value::as_str)
        == Some(provider_scope.as_str())
    {
        return;
    }
    let module_enabled = match state
        .read_system_config_json_value(SIMULATED_CACHE_MODULE_ENABLED_KEY)
        .await
    {
        Ok(value) => value.as_ref().and_then(Value::as_bool).unwrap_or(false),
        Err(err) => {
            warn!(
                event_name = "simulated_cache_module_config_read_failed",
                log_type = "event",
                request_id = %plan.request_id,
                provider_id = %plan.provider_id,
                error = ?err,
                "failed to read simulated cache module config; defaulting disabled"
            );
            false
        }
    };
    let allow_legacy_kiro = plan
        .provider_name
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case("kiro"));
    let mode = if module_enabled || allow_legacy_kiro {
        match state
            .read_provider_catalog_providers_by_ids(std::slice::from_ref(&plan.provider_id))
            .await
        {
            Ok(providers) => providers
                .iter()
                .find(|provider| provider.id == plan.provider_id)
                .map(|provider| {
                    simulated_cache_mode_from_provider_config(
                        provider.provider_type.as_str(),
                        provider.config.as_ref(),
                        module_enabled,
                        allow_legacy_kiro,
                    )
                })
                .unwrap_or(SimulatedCacheMode::Disabled),
            Err(err) => {
                warn!(
                    event_name = "simulated_cache_provider_config_read_failed",
                    log_type = "event",
                    request_id = %plan.request_id,
                    provider_id = %plan.provider_id,
                    error = ?err,
                    "failed to read simulated cache provider config; defaulting disabled"
                );
                SimulatedCacheMode::Disabled
            }
        }
    } else {
        SimulatedCacheMode::Disabled
    };
    seed_simulated_cache_mode_in_report_context(report_context, mode);
    if let Some(context) = report_context.as_mut().and_then(Value::as_object_mut) {
        context.insert(
            "simulated_cache_provider_scope".into(),
            Value::from(provider_scope),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn media_outputs_are_excluded_but_media_inputs_remain_text_requests() {
        for body in [
            json!({"tool_choice":{"type":"image_generation"}}),
            json!({"modalities":["text","audio"]}),
            json!({"generationConfig":{"responseModalities":["TEXT","IMAGE"]}}),
            json!({"generation_config":{"response_modalities":["AUDIO"]}}),
            json!({"response_modalities":["video"]}),
        ] {
            assert!(request_generates_media(&body), "{body}");
        }
        for body in [
            json!({"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com/image.png"}},{"type":"text","text":"describe it"}]}]}),
            json!({"contents":[{"parts":[{"inlineData":{"mimeType":"audio/wav","data":"test"}}]}]}),
            json!({"tools":[{"type":"image_generation"}],"tool_choice":"auto"}),
        ] {
            assert!(!request_generates_media(&body), "{body}");
        }
    }

    #[test]
    fn summary_uses_real_input_even_when_estimate_is_larger_or_actual_is_zero() {
        let context = json!({"simulated_cache_enabled":true,"simulated_cache_hit_basis_points":10000,"input_tokens":1000});
        for input in [0, 300] {
            let mut summary = ExecutionStreamTerminalSummary {
                standardized_usage: Some(StandardizedUsage {
                    input_tokens: input,
                    output_tokens: 7,
                    ..StandardizedUsage::new()
                }),
                ..ExecutionStreamTerminalSummary::default()
            };
            apply_simulated_cache_to_summary("openai:responses", Some(&context), &mut summary);
            let usage = summary.standardized_usage.unwrap();
            assert_eq!(usage.input_tokens, input);
            assert_eq!(usage.cache_read_tokens, input);
            assert_eq!(usage.output_tokens, 7);
        }
        let mut summary = ExecutionStreamTerminalSummary::default();
        apply_simulated_cache_to_summary("openai:responses", Some(&context), &mut summary);
        assert_eq!(summary.standardized_usage.unwrap().cache_read_tokens, 1000);
        let mut summary = ExecutionStreamTerminalSummary::default();
        apply_simulated_cache_to_summary(
            "openai:responses",
            Some(&json!({"simulated_cache_enabled":true,"simulated_cache_hit_basis_points":5000})),
            &mut summary,
        );
        assert!(summary.standardized_usage.is_none());
    }
}
