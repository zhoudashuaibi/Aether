use super::*;
use serde_json::json;

fn context(format: &str, bps: u32) -> Value {
    json!({"client_api_format":format, "simulated_cache_enabled":true,"simulated_cache_hit_basis_points":bps,"input_tokens":1000,"cache_read_input_tokens":1000})
}

#[test]
fn native_usage_uses_actual_input_and_preserves_unknown_fields() {
    for (format, mut body, path, input_path) in [
        (
            "openai:chat",
            json!({"usage":{"prompt_tokens":300,"prompt_tokens_details":{"audio_tokens":12}},"extra":true}),
            "/usage/prompt_tokens_details/cached_tokens",
            "/usage/prompt_tokens",
        ),
        (
            "openai:responses",
            json!({"usage":{"input_tokens":300},"extra":true}),
            "/usage/input_tokens_details/cached_tokens",
            "/usage/input_tokens",
        ),
        (
            "claude:messages",
            json!({"usage":{"input_tokens":100,"cache_read_input_tokens":100,"cache_creation_input_tokens":100},"extra":true}),
            "/usage/cache_read_input_tokens",
            "/usage/input_tokens",
        ),
        (
            "gemini:generate_content",
            json!({"usageMetadata":{"promptTokenCount":300},"extra":true}),
            "/usageMetadata/cachedContentTokenCount",
            "/usageMetadata/promptTokenCount",
        ),
        (
            "gemini:interactions",
            json!({"usage":{"total_input_tokens":300},"extra":true}),
            "/usage/total_cached_tokens",
            "/usage/total_input_tokens",
        ),
    ] {
        assert!(apply_simulated_cache_usage_to_body(
            &mut body,
            format,
            Some(&context(format, 10000))
        ));
        assert_eq!(body.pointer(path), Some(&json!(300)), "{format}");
        assert_eq!(body["extra"], true);
        assert!(apply_simulated_cache_usage_to_body(
            &mut body,
            format,
            Some(&context(format, 0))
        ));
        assert_eq!(body.pointer(path), Some(&json!(0)));
        assert_eq!(body.pointer(input_path), Some(&json!(300)));
    }
}

#[test]
fn claude_stream_retains_input_across_output_only_delta_and_chunk_boundaries() {
    let input = concat!("event: message_start\r\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":600,\"cache_creation_input_tokens\":100,\"cache_read_input_tokens\":300}}}\r\n\r\n",
        "event: message_delta\r\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":8}}\r\n\r\n");
    let mut rewriter = SimulatedCacheUsageStreamRewriter::from_report_context(Some(&context(
        "claude:messages",
        5000,
    )))
    .unwrap();
    let mut output = Vec::new();
    for chunk in input.as_bytes().chunks(7) {
        output.extend(rewriter.push_chunk(chunk));
    }
    output.extend(rewriter.finish());
    let output = String::from_utf8(output).unwrap();
    let events: Vec<Value> = output
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();
    for usage in [&events[0]["message"]["usage"], &events[1]["usage"]] {
        assert_eq!(usage["input_tokens"], 500);
        assert_eq!(usage["cache_read_input_tokens"], 500);
        assert_eq!(usage["cache_creation_input_tokens"], 0);
    }
    assert_eq!(events[1]["usage"]["output_tokens"], 8);
}

#[test]
fn response_websocket_batches_rewrite_only_existing_terminal_usage() {
    let mut event = json!({"chunks":[
        {"type":"response.done","response":{"usage":{"input_tokens":1000}}},
        {"type":"response.incomplete","response":{"usage":{"input_tokens":300}}},
        {"type":"response.failed","response":{"error":{"message":"failed"}}}
    ]});
    assert!(apply_simulated_cache_usage_to_event(
        &mut event,
        "openai:responses",
        Some(&context("openai:responses", 5000))
    ));
    assert_eq!(
        event["chunks"][0]["response"]["usage"]["input_tokens_details"]["cached_tokens"],
        500
    );
    assert_eq!(
        event["chunks"][1]["response"]["usage"]["input_tokens_details"]["cached_tokens"],
        150
    );
    assert!(event["chunks"][2]["response"].get("usage").is_none());
}

#[test]
fn percentage_and_fixed_policies_cannot_exceed_input() {
    assert_eq!(
        SimulatedCachePolicy::Percentage(10000).cache_read_tokens(u64::MAX),
        u64::MAX
    );
    assert_eq!(
        SimulatedCachePolicy::Fixed(1000).cache_read_tokens(300),
        300
    );
    for format in ["openai:chat", "claude:messages", "gemini:generate_content"] {
        let mut usage = StandardizedUsage::new();
        SimulatedCachePolicy::Percentage(5000).apply_to_usage(&mut usage, format, 1000);
        assert_eq!(standardized_gross_input_tokens(&usage, format), 1000);
        assert_eq!(usage.cache_read_tokens, 500);
    }
}

#[test]
fn gemini_native_streams_expose_simulated_cache_usage() {
    for (format, event, path) in [
        (
            "gemini:generate_content",
            json!({"usageMetadata":{"promptTokenCount":300,"candidatesTokenCount":7},"unknown":true}),
            "/usageMetadata/cachedContentTokenCount",
        ),
        (
            "gemini:interactions",
            json!({"event_type":"interaction.completed","interaction":{"usage":{"total_input_tokens":300,"total_output_tokens":7}},"unknown":true}),
            "/interaction/usage/total_cached_tokens",
        ),
    ] {
        let mut rewriter =
            SimulatedCacheUsageStreamRewriter::from_report_context(Some(&context(format, 5000)))
                .unwrap();
        let record = format!("data: {event}\n\n");
        let output = rewriter.push_chunk(record.as_bytes());
        let output = String::from_utf8(output).unwrap();
        let event: Value =
            serde_json::from_str(output.trim().strip_prefix("data: ").unwrap()).unwrap();
        assert_eq!(event.pointer(path), Some(&json!(150)));
        assert_eq!(event["unknown"], true);
    }
}

#[test]
fn disabled_and_nontext_formats_preserve_upstream_usage() {
    for format in [
        "openai:image",
        "openai:audio",
        "openai:embedding",
        "gemini:embedding",
        "openai:video",
    ] {
        let mut body =
            json!({"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":2}}});
        let original = body.clone();
        assert!(!apply_simulated_cache_usage_to_body(
            &mut body,
            format,
            Some(&context(format, 5000))
        ));
        assert_eq!(body, original);
    }
    let mut body =
        json!({"usage":{"prompt_tokens":100,"prompt_tokens_details":{"cached_tokens":2}}});
    let original = body.clone();
    assert!(!apply_simulated_cache_usage_to_body(
        &mut body,
        "openai:chat",
        Some(&json!({"simulated_cache_enabled":false}))
    ));
    assert_eq!(body, original);
}

#[test]
fn multiline_sse_usage_preserves_event_metadata() {
    let mut rewriter =
        SimulatedCacheUsageStreamRewriter::from_report_context(Some(&context("openai:chat", 5000)))
            .unwrap();
    let record=b"id: 7\r\ndata: {\"usage\":{\r\ndata: \"prompt_tokens\":1000,\"completion_tokens\":10}}\r\n\r\n";
    let output = String::from_utf8(rewriter.push_chunk(record)).unwrap();
    assert!(output.starts_with("id: 7\r\n"));
    let data = output
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    let event: Value = serde_json::from_str(data).unwrap();
    assert_eq!(
        event["usage"]["prompt_tokens_details"]["cached_tokens"],
        500
    );
}
