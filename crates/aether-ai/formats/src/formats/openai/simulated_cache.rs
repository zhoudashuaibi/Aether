use serde_json::{Map, Value};

/// 将模拟缓存读 tokens 写入 OpenAI Chat 或 Responses 同步响应 usage。
pub fn apply_simulated_cache_usage_to_openai_body(
    body: &mut Value,
    client_api_format: &str,
    report_context: Option<&Value>,
) -> bool {
    let Some(format) = OpenAiUsageFormat::from_api_format(client_api_format) else {
        return false;
    };
    let Some(cache_read_tokens) = simulated_cache_read_tokens(report_context) else {
        return false;
    };
    apply_cached_tokens_to_openai_usage(body, format, cache_read_tokens)
}

/// Backward-compatible name retained for callers that only handled Responses before Chat support.
pub fn apply_simulated_cache_usage_to_openai_responses_body(
    body: &mut Value,
    client_api_format: &str,
    report_context: Option<&Value>,
) -> bool {
    apply_simulated_cache_usage_to_openai_body(body, client_api_format, report_context)
}

const MAX_SIMULATED_CACHE_SSE_RECORD_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy)]
enum OpenAiUsageFormat {
    Chat,
    Responses,
}

impl OpenAiUsageFormat {
    fn from_api_format(api_format: &str) -> Option<Self> {
        let api_format = api_format.trim().to_ascii_lowercase();
        if api_format.starts_with("openai:responses") {
            Some(Self::Responses)
        } else if api_format.starts_with("openai:chat") {
            Some(Self::Chat)
        } else {
            None
        }
    }

    fn input_tokens_key(self) -> &'static str {
        match self {
            Self::Chat => "prompt_tokens",
            Self::Responses => "input_tokens",
        }
    }

    fn details_key(self) -> &'static str {
        match self {
            Self::Chat => "prompt_tokens_details",
            Self::Responses => "input_tokens_details",
        }
    }
}

pub struct SimulatedCacheUsageStreamRewriter {
    format: OpenAiUsageFormat,
    cache_read_tokens: u64,
    buffered: Vec<u8>,
    passthrough_after_oversize_record: bool,
}

impl SimulatedCacheUsageStreamRewriter {
    pub fn from_report_context(report_context: Option<&Value>) -> Option<Self> {
        let context = report_context?.as_object()?;
        let client_api_format = context.get("client_api_format").and_then(Value::as_str)?;
        let format = OpenAiUsageFormat::from_api_format(client_api_format)?;
        let cache_read_tokens = simulated_cache_read_tokens(report_context)?;
        Some(Self {
            format,
            cache_read_tokens,
            buffered: Vec::new(),
            passthrough_after_oversize_record: false,
        })
    }

    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<u8> {
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
                self.format,
            ));
        }

        if self.buffered.len() > MAX_SIMULATED_CACHE_SSE_RECORD_BYTES {
            output.extend(std::mem::take(&mut self.buffered));
            self.passthrough_after_oversize_record = true;
        }
        output
    }

    pub fn finish(&mut self) -> Vec<u8> {
        if self.buffered.is_empty() {
            return Vec::new();
        }
        let buffered = std::mem::take(&mut self.buffered);
        if self.passthrough_after_oversize_record {
            buffered
        } else {
            rewrite_sse_record_cached_tokens(&buffered, self.cache_read_tokens, self.format)
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

fn apply_cached_tokens_to_openai_usage(
    body: &mut Value,
    format: OpenAiUsageFormat,
    cache_read_tokens: u64,
) -> bool {
    let Some(body) = body.as_object_mut() else {
        return false;
    };
    let Some(usage) = body.get_mut("usage").and_then(Value::as_object_mut) else {
        return false;
    };
    apply_cached_tokens_to_openai_usage_object(usage, format, cache_read_tokens)
}

fn apply_cached_tokens_to_openai_usage_object(
    usage: &mut Map<String, Value>,
    format: OpenAiUsageFormat,
    cache_read_tokens: u64,
) -> bool {
    if usage
        .get(format.input_tokens_key())
        .and_then(Value::as_u64)
        .is_none()
    {
        return false;
    }

    let details = usage
        .entry(format.details_key().to_string())
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

fn rewrite_sse_record_cached_tokens(
    record: &[u8],
    cache_read_tokens: u64,
    format: OpenAiUsageFormat,
) -> Vec<u8> {
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
        let Some(usage_body) = (match format {
            OpenAiUsageFormat::Responses => {
                if event.get("type").and_then(Value::as_str) != Some("response.completed") {
                    None
                } else {
                    event.get_mut("response")
                }
            }
            OpenAiUsageFormat::Chat => event.get_mut("usage"),
        }) else {
            output.push_str(line);
            continue;
        };
        let did_rewrite = match format {
            OpenAiUsageFormat::Responses => {
                apply_cached_tokens_to_openai_usage(usage_body, format, cache_read_tokens)
            }
            OpenAiUsageFormat::Chat => usage_body.as_object_mut().is_some_and(|usage| {
                apply_cached_tokens_to_openai_usage_object(usage, format, cache_read_tokens)
            }),
        };
        if !did_rewrite {
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
