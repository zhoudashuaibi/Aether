use serde_json::{Map, Value};

use crate::ai_serving::{
    maybe_build_ai_surface_stream_rewriter, AiSurfaceFinalizeError, AiSurfaceStreamRewriter,
    ResponseHistoryRecord,
};
use crate::GatewayError;

pub(crate) struct LocalStreamRewriter<'a> {
    inner: Option<AiSurfaceStreamRewriter<'a>>,
    simulated_cache_usage: Option<SimulatedCacheUsageStreamRewriter>,
}

pub(crate) fn maybe_build_local_stream_rewriter<'a>(
    report_context: Option<&'a Value>,
) -> Option<LocalStreamRewriter<'a>> {
    let inner = maybe_build_ai_surface_stream_rewriter(report_context);
    let simulated_cache_usage =
        SimulatedCacheUsageStreamRewriter::from_report_context(report_context);
    (inner.is_some() || simulated_cache_usage.is_some()).then_some(LocalStreamRewriter {
        inner,
        simulated_cache_usage,
    })
}

impl LocalStreamRewriter<'_> {
    pub(crate) fn push_chunk(&mut self, chunk: &[u8]) -> Result<Vec<u8>, GatewayError> {
        let chunk = if let Some(inner) = self.inner.as_mut() {
            inner.push_chunk(chunk).map_err(map_surface_error)?
        } else {
            chunk.to_vec()
        };
        Ok(self
            .simulated_cache_usage
            .as_mut()
            .map(|rewriter| rewriter.push_chunk(&chunk))
            .unwrap_or(chunk))
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<u8>, GatewayError> {
        let chunk = if let Some(inner) = self.inner.as_mut() {
            inner.finish().map_err(map_surface_error)?
        } else {
            Vec::new()
        };
        let mut output = self
            .simulated_cache_usage
            .as_mut()
            .map(|rewriter| rewriter.push_chunk(&chunk))
            .unwrap_or(chunk);
        if let Some(rewriter) = self.simulated_cache_usage.as_mut() {
            output.extend(rewriter.finish());
        }
        Ok(output)
    }

    pub(crate) fn take_response_history_record(&mut self) -> Option<ResponseHistoryRecord> {
        self.inner
            .as_mut()
            .and_then(AiSurfaceStreamRewriter::take_response_history_record)
    }
}

pub(crate) fn apply_simulated_cache_usage_to_openai_responses_body(
    body: &mut Value,
    client_api_format: &str,
    report_context: Option<&Value>,
) -> bool {
    if !is_openai_responses_api_format(client_api_format) {
        return false;
    }
    let Some(cache_read_tokens) = simulated_cache_read_tokens(report_context) else {
        return false;
    };
    apply_cached_tokens_to_openai_responses_usage(body, cache_read_tokens)
}

const MAX_SIMULATED_CACHE_SSE_RECORD_BYTES: usize = 2 * 1024 * 1024;

struct SimulatedCacheUsageStreamRewriter {
    cache_read_tokens: u64,
    buffered: Vec<u8>,
    passthrough_after_oversize_record: bool,
}

impl SimulatedCacheUsageStreamRewriter {
    fn from_report_context(report_context: Option<&Value>) -> Option<Self> {
        let context = report_context?.as_object()?;
        let client_api_format = context.get("client_api_format").and_then(Value::as_str)?;
        let cache_read_tokens = simulated_cache_read_tokens(report_context)?;
        is_openai_responses_api_format(client_api_format).then_some(Self {
            cache_read_tokens,
            buffered: Vec::new(),
            passthrough_after_oversize_record: false,
        })
    }

    fn push_chunk(&mut self, chunk: &[u8]) -> Vec<u8> {
        if chunk.is_empty() {
            return Vec::new();
        }
        if self.passthrough_after_oversize_record {
            return chunk.to_vec();
        }

        self.buffered.extend_from_slice(chunk);
        let mut output = Vec::new();
        while let Some((record_end, separator_len)) = find_sse_record_boundary(&self.buffered) {
            let record = self
                .buffered
                .drain(..record_end + separator_len)
                .collect::<Vec<_>>();
            output.extend(rewrite_sse_record_cached_tokens(
                &record,
                self.cache_read_tokens,
            ));
        }

        if self.buffered.len() > MAX_SIMULATED_CACHE_SSE_RECORD_BYTES {
            output.extend(std::mem::take(&mut self.buffered));
            self.passthrough_after_oversize_record = true;
        }
        output
    }

    fn finish(&mut self) -> Vec<u8> {
        if self.buffered.is_empty() {
            return Vec::new();
        }
        let buffered = std::mem::take(&mut self.buffered);
        if self.passthrough_after_oversize_record {
            buffered
        } else {
            rewrite_sse_record_cached_tokens(&buffered, self.cache_read_tokens)
        }
    }
}

fn simulated_cache_read_tokens(report_context: Option<&Value>) -> Option<u64> {
    let context = report_context?.as_object()?;
    context
        .get("simulated_cache_enabled")
        .and_then(Value::as_bool)
        .filter(|enabled| *enabled)
        .and_then(|_| context.get("cache_read_input_tokens"))
        .and_then(Value::as_u64)
        .filter(|tokens| *tokens > 0)
}

fn is_openai_responses_api_format(api_format: &str) -> bool {
    api_format
        .trim()
        .to_ascii_lowercase()
        .starts_with("openai:responses")
}

fn apply_cached_tokens_to_openai_responses_usage(body: &mut Value, cache_read_tokens: u64) -> bool {
    let Some(body) = body.as_object_mut() else {
        return false;
    };
    let Some(usage) = body.get_mut("usage").and_then(Value::as_object_mut) else {
        return false;
    };
    if usage.get("input_tokens").and_then(Value::as_u64).is_none() {
        return false;
    }

    let details = usage
        .entry("input_tokens_details".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(details) = details.as_object_mut() else {
        return false;
    };
    details.insert("cached_tokens".to_string(), Value::from(cache_read_tokens));
    true
}

fn find_sse_record_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    for index in 0..buffer.len().saturating_sub(1) {
        if buffer[index] == b'\n' && buffer[index + 1] == b'\n' {
            return Some((index, 2));
        }
        if index + 3 < buffer.len() && buffer[index..index + 4] == [b'\r', b'\n', b'\r', b'\n'] {
            return Some((index, 4));
        }
    }
    None
}

fn rewrite_sse_record_cached_tokens(record: &[u8], cache_read_tokens: u64) -> Vec<u8> {
    let Ok(record) = std::str::from_utf8(record) else {
        return record.to_vec();
    };
    let mut output = String::with_capacity(record.len());
    let mut rewritten = false;

    for line in record.split_inclusive('\n') {
        if rewritten {
            output.push_str(line);
            continue;
        }
        let content_end = line.trim_end_matches(['\r', '\n']).len();
        let content = &line[..content_end];
        let leading_len = content.len() - content.trim_start().len();
        let Some(data) = content[leading_len..].strip_prefix("data:") else {
            output.push_str(line);
            continue;
        };
        let data_leading_len = data.len() - data.trim_start().len();
        let payload = data.trim();
        let Ok(mut event) = serde_json::from_str::<Value>(payload) else {
            output.push_str(line);
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("response.completed") {
            output.push_str(line);
            continue;
        }
        let Some(response) = event.get_mut("response") else {
            output.push_str(line);
            continue;
        };
        if !apply_cached_tokens_to_openai_responses_usage(response, cache_read_tokens) {
            output.push_str(line);
            continue;
        }

        output.push_str(&content[..leading_len + "data:".len() + data_leading_len]);
        output.push_str(
            &serde_json::to_string(&event).expect("serializing a JSON value should not fail"),
        );
        output.push_str(&line[content_end..]);
        rewritten = true;
    }

    output.into_bytes()
}

fn map_surface_error(error: AiSurfaceFinalizeError) -> GatewayError {
    error.into()
}

#[cfg(test)]
#[path = "../tests_stream.rs"]
mod tests;
