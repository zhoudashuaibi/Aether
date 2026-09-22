use aether_usage_runtime::decode_internal_report_body_base64;
use base64::Engine as _;
use serde_json::Value;

use crate::{usage::GatewaySyncReportRequest, GatewayError};

use super::{
    maybe_build_provider_private_stream_normalizer, normalize_provider_private_report_context,
    normalize_provider_private_response_value, provider_private_response_allows_sync_finalize,
    stream_body_contains_error_event, ProviderPrivateStreamNormalizer,
};

pub(crate) fn maybe_normalize_provider_private_sync_report_payload(
    payload: &GatewaySyncReportRequest,
) -> Result<Option<GatewaySyncReportRequest>, GatewayError> {
    let Some(report_context) = payload.report_context.as_ref() else {
        return Ok(Some(payload.clone()));
    };
    if !report_context
        .get("has_envelope")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(Some(payload.clone()));
    }
    if !provider_private_response_allows_sync_finalize(report_context) {
        return Ok(None);
    }

    let mut normalized = payload.clone();
    normalized.report_context = normalize_provider_private_report_context(Some(report_context));
    if !(200..300).contains(&payload.status_code)
        && report_context.get("envelope_name").and_then(Value::as_str)
            == Some(aether_provider_transport::command_code::ENVELOPE_NAME)
    {
        // HTTP error bodies are ordinary JSON/text, not the generation NDJSON.
        return Ok(Some(normalized));
    }
    if let (Some(body_json), Some(context)) = (
        payload.body_json.as_ref(),
        normalized.report_context.as_mut(),
    ) {
        maybe_attach_gemini_cli_v1internal_credits_context(report_context, body_json, context);
    }

    if let Some(body_json) = payload.body_json.clone() {
        normalized.body_json = normalize_provider_private_response_value(body_json, report_context);
        if normalized.body_json.is_none() {
            return Ok(None);
        }
    }

    if let Some(body_base64) = payload.body_base64.as_deref() {
        let body_bytes =
            decode_internal_report_body_base64(body_base64).map_err(GatewayError::Internal)?;
        let Some(normalized_bytes) =
            normalize_provider_private_stream_bytes(report_context, &body_bytes)?
        else {
            return Ok(None);
        };
        if stream_body_contains_error_event(&normalized_bytes) {
            return Ok(None);
        }
        normalized.body_base64 = (!normalized_bytes.is_empty())
            .then(|| base64::engine::general_purpose::STANDARD.encode(normalized_bytes));
    }

    Ok(Some(normalized))
}

fn maybe_attach_gemini_cli_v1internal_credits_context(
    original_report_context: &Value,
    body_json: &Value,
    normalized_report_context: &mut Value,
) {
    if !original_report_context
        .get("envelope_name")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("gemini_cli:v1internal"))
    {
        return;
    }

    let mut credits = serde_json::Map::new();
    for (source, target) in [
        ("remainingCredits", "remainingCredits"),
        ("consumedCredits", "consumedCredits"),
        ("traceId", "traceId"),
    ] {
        if let Some(value) = body_json
            .get(source)
            .cloned()
            .filter(|value| !value.is_null())
        {
            credits.insert(target.to_string(), value);
        }
    }
    if credits.is_empty() {
        return;
    }
    if let Some(object) = normalized_report_context.as_object_mut() {
        object.insert(
            "gemini_cli_v1internal_credits".to_string(),
            Value::Object(credits),
        );
    }
}

fn normalize_provider_private_stream_bytes(
    report_context: &Value,
    body: &[u8],
) -> Result<Option<Vec<u8>>, GatewayError> {
    let Some(mut normalizer): Option<ProviderPrivateStreamNormalizer<'_>> =
        maybe_build_provider_private_stream_normalizer(Some(report_context))
    else {
        return Ok(Some(body.to_vec()));
    };
    let mut normalized = normalizer.push_chunk(body).map_err(GatewayError::from)?;
    normalized.extend(normalizer.finish().map_err(GatewayError::from)?);
    Ok(Some(normalized))
}

#[cfg(test)]
mod command_code_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn http_failure_is_not_parsed_as_generation_events() {
        let report: GatewaySyncReportRequest = serde_json::from_value(json!({
            "trace_id":"test", "report_kind":"openai_chat_sync_finalize", "status_code":401, "headers":{},
            "report_context":{"has_envelope":true, "envelope_name":"command_code:generate", "provider_api_format":"openai:chat"},
            "body_base64":base64::engine::general_purpose::STANDARD.encode(b"{\"error\":\"unauthorized\"}")
        })).unwrap();
        let normalized = maybe_normalize_provider_private_sync_report_payload(&report)
            .unwrap()
            .unwrap();
        assert_eq!(normalized.status_code, 401);
        assert_eq!(normalized.body_base64, report.body_base64);
    }
}
