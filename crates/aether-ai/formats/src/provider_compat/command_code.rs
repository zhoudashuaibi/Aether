//! Command Code's NDJSON wire protocol, normalized to the OpenAI Chat anchor.
//! Protocol reference: CommandCodeGo-manager v0.2.10 (MIT); see THIRD_PARTY_NOTICES.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::formats::shared::AiSurfaceFinalizeError;

const MAX_LINE_BYTES: usize = 1024 * 1024;

pub struct CommandCodeStreamNormalizer {
    buffered: Vec<u8>,
    id: String,
    model: String,
    started: bool,
    finished: bool,
    failed: bool,
    terminal: Option<(String, Option<Value>)>,
    tool_ids: HashSet<String>,
    pending_tools: HashSet<String>,
}

impl CommandCodeStreamNormalizer {
    pub fn new(context: &Value) -> Self {
        Self {
            buffered: Vec::new(),
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
            model: context
                .get("mapped_model")
                .or_else(|| context.get("model"))
                .and_then(Value::as_str)
                .unwrap_or("command-code")
                .to_string(),
            started: false,
            finished: false,
            failed: false,
            terminal: None,
            tool_ids: HashSet::new(),
            pending_tools: HashSet::new(),
        }
    }

    pub fn push_chunk(&mut self, chunk: &[u8]) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        if self.failed {
            return Err(error("stream already failed"));
        }
        let result = self.push_chunk_inner(chunk);
        self.failed = result.is_err();
        result
    }

    fn push_chunk_inner(&mut self, chunk: &[u8]) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        if self.finished {
            return Err(error("data received after stream finalization"));
        }
        let mut output = Vec::new();
        // Scan each incoming byte once; do not repeatedly scan the partial line.
        for part in chunk.split_inclusive(|byte| *byte == b'\n') {
            if self.buffered.len().saturating_add(part.len()) > MAX_LINE_BYTES {
                return Err(error("NDJSON line exceeds 1 MiB"));
            }
            self.buffered.extend_from_slice(part);
            if part.last() == Some(&b'\n') {
                let line = std::mem::take(&mut self.buffered);
                output.extend(self.line(&line)?);
            }
        }
        Ok(output)
    }

    pub fn finish(&mut self) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        if self.failed {
            return Err(error("stream already failed"));
        }
        let result = self.finish_inner();
        self.failed = result.is_err();
        result
    }

    fn finish_inner(&mut self) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        if self.finished {
            return Ok(Vec::new());
        }
        let mut output = Vec::new();
        if !self.buffered.is_empty() {
            let line = std::mem::take(&mut self.buffered);
            output.extend(self.line(&line)?);
        }
        let (reason, usage) = self
            .terminal
            .take()
            .ok_or_else(|| error("stream ended without a valid finish event"))?;
        self.start(&mut output)?;
        output.extend(self.chunk(json!({}), Some(&reason), usage)?);
        output.extend_from_slice(b"data: [DONE]\n\n");
        self.finished = true;
        Ok(output)
    }

    fn line(&mut self, line: &[u8]) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        if line.iter().all(u8::is_ascii_whitespace) {
            return Ok(Vec::new());
        }
        let event: Value = serde_json::from_slice(line)
            .map_err(|_| error("invalid NDJSON or UTF-8 from upstream"))?;
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| error("upstream event has no type"))?;
        if kind == "error" || kind == "abort" {
            // Never relay arbitrary upstream error text: it can contain credentials.
            return Err(error("upstream reported a generation failure"));
        }
        match kind {
            "start" | "start-step" | "finish-step" | "text-start" | "text-end"
            | "reasoning-start" | "reasoning-end" | "raw" | "source" => {
                return Ok(Vec::new());
            }
            _ => {}
        }
        if self.terminal.is_some() {
            return Err(error("content received after the finish event"));
        }
        let mut output = Vec::new();
        match kind {
            "tool-input-start" | "tool-input-delta" | "tool-input-end" => {
                let id = required_string(&event, "id")?;
                if !self.tool_ids.contains(id) {
                    self.pending_tools.insert(id.to_string());
                }
            }
            "text-delta" | "reasoning-delta" => {
                let text = event
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| error("invalid text delta"))?;
                self.start(&mut output)?;
                let field = if kind == "text-delta" {
                    "content"
                } else {
                    "reasoning_content"
                };
                output.extend(self.chunk(json!({field: text}), None, None)?);
            }
            "tool-call" => {
                let id = required_string(&event, "toolCallId")?;
                let name = required_string(&event, "toolName")?;
                let input = event
                    .get("input")
                    .filter(|input| input.is_object())
                    .ok_or_else(|| error("tool call input must be an object"))?;
                let index = self.tool_ids.len();
                if !self.tool_ids.insert(id.to_string()) {
                    return Err(error("duplicate tool call id"));
                }
                self.pending_tools.remove(id);
                self.start(&mut output)?;
                output.extend(self.chunk(
                    json!({"tool_calls": [{
                        "index": index, "id": id, "type": "function",
                        "function": {"name": name, "arguments": input.to_string()}
                    }]}),
                    None,
                    None,
                )?);
            }
            "finish" => {
                let reason = event
                    .get("rawFinishReason")
                    .and_then(Value::as_str)
                    .filter(|reason| !reason.trim().is_empty())
                    .or_else(|| event.get("finishReason").and_then(Value::as_str))
                    .map(str::trim)
                    .ok_or_else(|| error("finish event has no finish reason"))?;
                let reason = match reason {
                    "stop" | "end_turn" => "stop",
                    "tool-calls" | "tool_calls" | "tool_use" => "tool_calls",
                    "length"
                    | "max_tokens"
                    | "max_output_tokens"
                    | "model_context_window_exceeded" => "length",
                    "pause_turn" => "length",
                    _ => return Err(error("unrecognized or unsuccessful finish reason")),
                };
                let usage = event
                    .get("totalUsage")
                    .or_else(|| event.get("usage"))
                    .filter(|usage| !usage.is_null())
                    .map(normalize_usage)
                    .transpose()?
                    .flatten();
                if !self.pending_tools.is_empty()
                    || reason == "tool_calls" && self.tool_ids.is_empty()
                {
                    return Err(error("tool completion has no complete tool call"));
                }
                self.terminal = Some((reason.to_string(), usage));
            }
            _ => return Err(error("unsupported upstream event type")),
        }
        Ok(output)
    }

    fn start(&mut self, output: &mut Vec<u8>) -> Result<(), AiSurfaceFinalizeError> {
        if !self.started {
            output.extend(self.chunk(json!({"role": "assistant"}), None, None)?);
            self.started = true;
        }
        Ok(())
    }

    fn chunk(
        &self,
        delta: Value,
        reason: Option<&str>,
        usage: Option<Value>,
    ) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        let mut value = json!({
            "id": self.id, "object": "chat.completion.chunk", "created": 0,
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": reason}]
        });
        if let Some(usage) = usage {
            value["usage"] = usage;
        }
        self.encode(value)
    }

    fn encode(&self, value: Value) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        Ok(format!(
            "data: {}\n\n",
            serde_json::to_string(&value).map_err(AiSurfaceFinalizeError::from)?
        )
        .into_bytes())
    }
}

fn error(message: &str) -> AiSurfaceFinalizeError {
    AiSurfaceFinalizeError::new(format!("Command Code: {message}"))
}

fn required_string<'a>(event: &'a Value, field: &str) -> Result<&'a str, AiSurfaceFinalizeError> {
    event
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error("missing tool call identifier or name"))
}

fn normalize_usage(usage: &Value) -> Result<Option<Value>, AiSurfaceFinalizeError> {
    if !usage.is_object() {
        return Err(error("invalid token usage"));
    }
    // A partial report is not exact usage. Let the gateway's existing missing
    // usage policy handle it rather than inventing zero token counts.
    let read_count = |value: Option<&Value>| -> Result<Option<u64>, AiSurfaceFinalizeError> {
        value
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| error("invalid token usage count"))
            })
            .transpose()
    };
    let (Some(input), Some(output)) = (
        read_count(usage.get("inputTokens"))?,
        read_count(usage.get("outputTokens"))?,
    ) else {
        return Ok(None);
    };
    let details = &usage["inputTokenDetails"];
    let cached = read_count(usage.get("cachedInputTokens"))?
        .or(read_count(details.get("cacheReadTokens"))?)
        .unwrap_or(0);
    let written = read_count(details.get("cacheWriteTokens"))?.unwrap_or(0);
    if cached.saturating_add(written) > input {
        return Err(error("cache usage exceeds total input tokens"));
    }
    Ok(Some(json!({
        "prompt_tokens": input, "completion_tokens": output,
        "total_tokens": input.saturating_add(output),
        "prompt_tokens_details": {"cached_tokens": cached, "cache_write_tokens": written}
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_split_utf8_tools_usage_and_unterminated_last_line() {
        let wire = concat!(
            "{\"type\":\"text-delta\",\"text\":\"你好\"}\n",
            "{\"type\":\"reasoning-delta\",\"text\":\"thinking\"}\n",
            "{\"type\":\"tool-call\",\"toolCallId\":\"call_1\",\"toolName\":\"lookup\",\"input\":{\"q\":1}}\n",
            "{\"type\":\"finish\",\"finishReason\":\"tool-calls\",\"totalUsage\":{\"inputTokens\":10,\"outputTokens\":3,\"cachedInputTokens\":4}}"
        );
        let mut state = CommandCodeStreamNormalizer::new(&json!({"mapped_model": "test"}));
        let mut output = Vec::new();
        for byte in wire.as_bytes() {
            output.extend(state.push_chunk(&[*byte]).unwrap());
        }
        assert!(!String::from_utf8_lossy(&output).contains("[DONE]"));
        output.extend(state.finish().unwrap());
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("你好"));
        assert!(text.contains("reasoning_content"));
        assert!(text.contains("call_1"));
        assert!(text.contains("\"cached_tokens\":4"));
        assert!(text.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn never_fabricates_success_for_truncation_or_step_finish() {
        for wire in [
            b"".as_slice(),
            b"{\"type\":\"finish-step\",\"finishReason\":\"stop\"}\n",
        ] {
            let mut state = CommandCodeStreamNormalizer::new(&json!({}));
            state.push_chunk(wire).unwrap();
            assert!(state.finish().is_err());
        }
    }

    #[test]
    fn rejects_malformed_oversized_and_post_finish_error() {
        for wire in [b"{bad}\n".to_vec(), vec![b'x'; MAX_LINE_BYTES + 1],
            b"{\"type\":\"finish\",\"finishReason\":\"stop\"}\n{\"type\":\"error\",\"error\":\"user_secret\"}\n".to_vec()] {
            let mut state = CommandCodeStreamNormalizer::new(&json!({}));
            let err = state.push_chunk(&wire).unwrap_err();
            assert!(!err.to_string().contains("user_secret"));
            assert!(state.finish().is_err());
            assert!(state.push_chunk(b"{\"type\":\"finish\",\"finishReason\":\"stop\"}\n").is_err());
        }
    }

    #[test]
    fn preserves_cache_write_as_a_subset_of_input() {
        let usage = normalize_usage(&json!({"inputTokens": 20, "outputTokens": 2,
            "inputTokenDetails": {"cacheReadTokens": 6, "cacheWriteTokens": 4, "noCacheTokens": 10}})).unwrap().unwrap();
        assert_eq!(usage["prompt_tokens"], 20);
        assert_eq!(usage["total_tokens"], 22);
        assert_eq!(usage["prompt_tokens_details"]["cache_write_tokens"], 4);
    }

    #[test]
    fn normalizes_pause_and_partial_usage_without_claiming_exact_counts() {
        let mut state = CommandCodeStreamNormalizer::new(&json!({}));
        state.push_chunk(b"{\"type\":\"finish\",\"rawFinishReason\":\"pause_turn\",\"usage\":{\"outputTokens\":2}}\n").unwrap();
        let output = String::from_utf8(state.finish().unwrap()).unwrap();
        assert!(output.contains("\"finish_reason\":\"length\""));
        assert!(!output.contains("\"usage\""));
    }

    #[test]
    fn rejects_incomplete_tools_and_raw_failure_reasons() {
        for wire in [
            concat!("{\"type\":\"tool-input-start\",\"id\":\"pending\"}\n", "{\"type\":\"finish\",\"finishReason\":\"stop\"}\n"),
            "{\"type\":\"finish\",\"rawFinishReason\":\"network_error\",\"finishReason\":\"stop\"}\n",
            "{\"type\":\"finish\",\"rawFinishReason\":\"other\",\"finishReason\":\"stop\"}\n",
        ] {
            let mut state = CommandCodeStreamNormalizer::new(&json!({}));
            assert!(state.push_chunk(wire.as_bytes()).is_err());
            assert!(state.finish().is_err());
        }
    }

    #[test]
    fn private_stream_bridge_preserves_tools_and_usage_in_all_client_formats() {
        use crate::formats::shared::{
            maybe_build_ai_surface_stream_rewriter,
            sync_products::{
                aggregate_openai_chat_stream_sync_response,
                aggregate_standard_chat_stream_sync_response, convert_standard_chat_response,
            },
        };
        use crate::provider_compat::private_envelope::{
            maybe_build_provider_private_stream_normalizer,
            normalize_provider_private_report_context,
        };

        let wire = concat!(
            "{\"type\":\"text-delta\",\"text\":\"hello\"}\n",
            "{\"type\":\"tool-call\",\"toolCallId\":\"call_lookup\",\"toolName\":\"lookup\",\"input\":{\"q\":1}}\n",
            "{\"type\":\"finish\",\"finishReason\":\"tool-calls\",\"totalUsage\":{\"inputTokens\":20,\"outputTokens\":2,\"inputTokenDetails\":{\"cacheReadTokens\":6,\"cacheWriteTokens\":4}}}\n"
        );
        for client in ["openai:chat", "openai:responses", "claude:messages"] {
            let context = json!({"has_envelope":true, "envelope_name":"command_code:generate",
                "provider_api_format":"openai:chat", "client_api_format":client,
                "needs_conversion":client != "openai:chat", "mapped_model":"test"});
            let normalized = normalize_provider_private_report_context(Some(&context)).unwrap();
            let mut normalizer =
                maybe_build_provider_private_stream_normalizer(Some(&context)).unwrap();
            let mut anchor = normalizer.push_chunk(wire.as_bytes()).unwrap();
            anchor.extend(normalizer.finish().unwrap());
            let sync_anchor = aggregate_openai_chat_stream_sync_response(&anchor).unwrap();
            let sync =
                convert_standard_chat_response(&sync_anchor, "openai:chat", client, &normalized)
                    .unwrap();
            let stream = if let Some(mut rewriter) =
                maybe_build_ai_surface_stream_rewriter(Some(&normalized))
            {
                let mut output = Vec::new();
                for chunk in anchor.chunks(7) {
                    output.extend(rewriter.push_chunk(chunk).unwrap());
                }
                output.extend(rewriter.finish().unwrap());
                output
            } else {
                anchor
            };
            let streamed = aggregate_standard_chat_stream_sync_response(&stream, client).unwrap();
            for response in [sync, streamed] {
                assert!(
                    response.to_string().contains("lookup"),
                    "{client}: {response}"
                );
                assert!(
                    response.to_string().contains("call_lookup"),
                    "{client}: {response}"
                );
                let usage = &response["usage"];
                match client {
                    "openai:chat" => {
                        assert_eq!(usage["prompt_tokens"], 20);
                        assert_eq!(usage["completion_tokens"], 2);
                        assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 6);
                        assert_eq!(usage["prompt_tokens_details"]["cache_write_tokens"], 4);
                    }
                    "openai:responses" => {
                        assert_eq!(usage["input_tokens"], 20);
                        assert_eq!(usage["output_tokens"], 2);
                        assert_eq!(usage["input_tokens_details"]["cached_tokens"], 6);
                    }
                    _ => {
                        assert_eq!(usage["input_tokens"], 10);
                        assert_eq!(usage["output_tokens"], 2);
                        assert_eq!(usage["cache_read_input_tokens"], 6);
                        assert_eq!(usage["cache_creation_input_tokens"], 4);
                    }
                }
            }
        }
    }
}
