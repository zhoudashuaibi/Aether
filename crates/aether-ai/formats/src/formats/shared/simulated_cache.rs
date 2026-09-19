use aether_contracts::StandardizedUsage;
use serde_json::{Map, Value};

/// The gateway selects a ratio once per attempt; format code only maps usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatedCachePolicy {
    Percentage(u32),
    Fixed(u64),
}

impl SimulatedCachePolicy {
    pub fn from_report_context(context: Option<&Value>) -> Option<Self> {
        let context = context?.as_object()?;
        if context
            .get("simulated_cache_enabled")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return None;
        }
        if let Some(bps) = context
            .get("simulated_cache_hit_basis_points")
            .and_then(Value::as_u64)
        {
            return (bps <= 10_000).then_some(Self::Percentage(bps as u32));
        }
        // Compatibility with reports produced before ratios were persisted.
        context
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .map(Self::Fixed)
    }

    pub fn cache_read_tokens(self, gross_input: u64) -> u64 {
        match self {
            Self::Percentage(bps) => {
                ((u128::from(gross_input) * u128::from(bps.min(10_000))) / 10_000) as u64
            }
            Self::Fixed(tokens) => tokens.min(gross_input),
        }
    }

    pub fn apply_to_usage(self, usage: &mut StandardizedUsage, api_format: &str, gross_input: u64) {
        let gross_input = gross_input.min(i64::MAX as u64);
        let read = self.cache_read_tokens(gross_input);
        usage.input_tokens = if reports_gross_input_tokens(api_format) {
            gross_input
        } else {
            gross_input - read
        } as i64;
        usage.cache_read_tokens = read as i64;
        usage.cache_creation_tokens = 0;
        usage.cache_creation_ephemeral_5m_tokens = 0;
        usage.cache_creation_ephemeral_1h_tokens = 0;
    }
}

pub fn reports_gross_input_tokens(api_format: &str) -> bool {
    matches!(
        api_format
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "openai" | "gemini" | "google"
    )
}

pub fn standardized_gross_input_tokens(usage: &StandardizedUsage, api_format: &str) -> u64 {
    let input = usage.input_tokens.max(0) as u64;
    if reports_gross_input_tokens(api_format) {
        input
    } else {
        input
            .saturating_add(usage.cache_creation_tokens.max(0) as u64)
            .saturating_add(usage.cache_read_tokens.max(0) as u64)
    }
}

#[derive(Clone, Copy)]
enum UsageFormat {
    Chat,
    Responses,
    Claude,
    Gemini,
    Interactions,
}

impl UsageFormat {
    fn parse(api_format: &str) -> Option<Self> {
        match api_format.trim().to_ascii_lowercase().as_str() {
            "openai:chat" => Some(Self::Chat),
            "openai:responses" | "openai:responses:compact" | "openai:responses_compact" => {
                Some(Self::Responses)
            }
            "claude:messages" => Some(Self::Claude),
            "gemini:generate_content" | "google:generate_content" => Some(Self::Gemini),
            "gemini:interactions" | "google:interactions" => Some(Self::Interactions),
            _ => None,
        }
    }

    fn usage_key(self) -> &'static str {
        match self {
            Self::Gemini => "usageMetadata",
            _ => "usage",
        }
    }

    fn input_key(self) -> &'static str {
        match self {
            Self::Chat => "prompt_tokens",
            Self::Responses | Self::Claude => "input_tokens",
            Self::Gemini => "promptTokenCount",
            Self::Interactions => "total_input_tokens",
        }
    }

    fn gross_input(self, usage: &Map<String, Value>) -> Option<u64> {
        let input = usage.get(self.input_key())?.as_u64()?;
        Some(if matches!(self, Self::Claude) {
            input
                .saturating_add(
                    usage
                        .get("cache_creation_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
                .saturating_add(
                    usage
                        .get("cache_read_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
        } else {
            input
        })
    }
}

pub fn supports_simulated_cache(api_format: &str) -> bool {
    UsageFormat::parse(api_format).is_some()
}

/// None distinguishes missing usage from a real zero-token response.
pub fn response_gross_input_tokens(body: &Value, api_format: &str) -> Option<u64> {
    let format = UsageFormat::parse(api_format)?;
    if let Some(usage) = body.get(format.usage_key()) {
        // A present null usage blocks lookup into nested or stale response envelopes.
        return usage
            .as_object()
            .and_then(|usage| format.gross_input(usage));
    }
    ["response", "message", "interaction"]
        .into_iter()
        .find_map(|key| {
            body.get(key)
                .and_then(|body| response_gross_input_tokens(body, api_format))
        })
}

pub fn apply_simulated_cache_usage_to_body(
    body: &mut Value,
    api_format: &str,
    context: Option<&Value>,
) -> bool {
    let Some(format) = UsageFormat::parse(api_format) else {
        return false;
    };
    let Some(policy) = SimulatedCachePolicy::from_report_context(context) else {
        return false;
    };
    rewrite_body(body, format, policy, None)
}

fn rewrite_body(
    body: &mut Value,
    format: UsageFormat,
    policy: SimulatedCachePolicy,
    gross: Option<u64>,
) -> bool {
    let Some(usage) = body
        .get_mut(format.usage_key())
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    rewrite_usage(usage, format, policy, gross)
}

fn rewrite_usage(
    usage: &mut Map<String, Value>,
    format: UsageFormat,
    policy: SimulatedCachePolicy,
    gross: Option<u64>,
) -> bool {
    let Some(input) = format.gross_input(usage).or(gross) else {
        return false;
    };
    let read = policy.cache_read_tokens(input);
    match format {
        UsageFormat::Chat | UsageFormat::Responses => {
            let key = if matches!(format, UsageFormat::Chat) {
                "prompt_tokens_details"
            } else {
                "input_tokens_details"
            };
            let details = usage
                .entry(key)
                .or_insert_with(|| Value::Object(Map::new()));
            if details.is_null() {
                *details = Value::Object(Map::new());
            }
            let Some(details) = details.as_object_mut() else {
                return false;
            };
            details.insert("cached_tokens".into(), Value::from(read));
            // Simulated reads replace native cache accounting, including writes.
            for key in ["cache_write_tokens", "cached_creation_tokens"] {
                if let Some(value) = details.get_mut(key) {
                    *value = Value::from(0);
                }
            }
            for key in ["cache_creation_input_tokens", "cache_read_input_tokens"] {
                if let Some(value) = usage.get_mut(key) {
                    *value = Value::from(if key == "cache_read_input_tokens" {
                        read
                    } else {
                        0
                    });
                }
            }
        }
        UsageFormat::Claude => {
            usage.insert("input_tokens".into(), Value::from(input - read));
            usage.insert("cache_read_input_tokens".into(), Value::from(read));
            usage.insert("cache_creation_input_tokens".into(), Value::from(0));
            if let Some(creation) = usage
                .get_mut("cache_creation")
                .and_then(Value::as_object_mut)
            {
                for key in ["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"] {
                    creation.insert(key.into(), Value::from(0));
                }
            }
        }
        UsageFormat::Gemini => {
            usage.insert("cachedContentTokenCount".into(), Value::from(read));
        }
        UsageFormat::Interactions => {
            usage.insert("total_cached_tokens".into(), Value::from(read));
        }
    }
    true
}

/// Rewrites native JSON events, including Responses WebSocket batch frames.
pub fn apply_simulated_cache_usage_to_event(
    event: &mut Value,
    api_format: &str,
    context: Option<&Value>,
) -> bool {
    let Some(format) = UsageFormat::parse(api_format) else {
        return false;
    };
    let Some(policy) = SimulatedCachePolicy::from_report_context(context) else {
        return false;
    };
    rewrite_event(event, format, policy, &mut None)
}

fn rewrite_event(
    event: &mut Value,
    format: UsageFormat,
    policy: SimulatedCachePolicy,
    claude_input: &mut Option<u64>,
) -> bool {
    if let Some(chunks) = event.get_mut("chunks").and_then(Value::as_array_mut) {
        let mut changed = false;
        for chunk in chunks {
            changed |= rewrite_event(chunk, format, policy, claude_input);
        }
        return changed;
    }
    match format {
        UsageFormat::Responses => {
            if !matches!(
                event.get("type").and_then(Value::as_str),
                Some("response.completed" | "response.done" | "response.incomplete")
            ) {
                return false;
            }
            event
                .get_mut("response")
                .is_some_and(|body| rewrite_body(body, format, policy, None))
        }
        UsageFormat::Claude => match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                let Some(message) = event.get_mut("message") else {
                    return false;
                };
                *claude_input = message
                    .get("usage")
                    .and_then(Value::as_object)
                    .and_then(|usage| format.gross_input(usage));
                rewrite_body(message, format, policy, *claude_input)
            }
            Some("message_delta") => {
                if let Some(input) = event
                    .get("usage")
                    .and_then(Value::as_object)
                    .and_then(|usage| format.gross_input(usage))
                {
                    *claude_input = Some(input);
                }
                rewrite_body(event, format, policy, *claude_input)
            }
            _ => false,
        },
        UsageFormat::Interactions => event
            .get_mut("interaction")
            .is_some_and(|body| rewrite_body(body, format, policy, None)),
        UsageFormat::Chat | UsageFormat::Gemini => rewrite_body(event, format, policy, None),
    }
}

const MAX_SSE_RECORD_BYTES: usize = 2 * 1024 * 1024;

pub struct SimulatedCacheUsageStreamRewriter {
    format: UsageFormat,
    policy: SimulatedCachePolicy,
    claude_input: Option<u64>,
    buffered: Vec<u8>,
    passthrough_after_oversize_record: bool,
}

impl SimulatedCacheUsageStreamRewriter {
    pub fn from_report_context(context: Option<&Value>) -> Option<Self> {
        Some(Self {
            format: UsageFormat::parse(context?.get("client_api_format")?.as_str()?)?,
            policy: SimulatedCachePolicy::from_report_context(context)?,
            claude_input: None,
            buffered: Vec::new(),
            passthrough_after_oversize_record: false,
        })
    }

    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<u8> {
        if self.passthrough_after_oversize_record {
            return chunk.to_vec();
        }
        self.buffered.extend_from_slice(chunk);
        let mut output = Vec::new();
        while let Some((end, separator)) = find_record_boundary(&self.buffered) {
            let record = self.buffered.drain(..end + separator).collect::<Vec<_>>();
            output.extend(self.rewrite_record(&record));
        }
        if self.buffered.len() > MAX_SSE_RECORD_BYTES {
            output.extend(std::mem::take(&mut self.buffered));
            self.passthrough_after_oversize_record = true;
        }
        output
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let buffered = std::mem::take(&mut self.buffered);
        self.rewrite_record(&buffered)
    }

    fn rewrite_record(&mut self, record: &[u8]) -> Vec<u8> {
        if record.len() > MAX_SSE_RECORD_BYTES {
            return record.to_vec();
        }
        let Ok(record) = std::str::from_utf8(record) else {
            return record.to_vec();
        };
        // SSE joins multiple data lines before parsing a single JSON event.
        let payload = record
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("data:"))
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(mut event) = serde_json::from_str::<Value>(&payload) else {
            return record.as_bytes().to_vec();
        };
        if !rewrite_event(&mut event, self.format, self.policy, &mut self.claude_input) {
            return record.as_bytes().to_vec();
        }
        let encoded = serde_json::to_string(&event).expect("JSON value serialization");
        let mut output = String::with_capacity(record.len());
        let mut emitted = false;
        for line in record.split_inclusive('\n') {
            let content = line.trim_end_matches(['\r', '\n']);
            let trimmed = content.trim_start();
            let Some(data) = trimmed.strip_prefix("data:") else {
                output.push_str(line);
                continue;
            };
            if emitted {
                continue;
            }
            let prefix_len =
                content.len() - trimmed.len() + 5 + data.len() - data.trim_start().len();
            output.push_str(&line[..prefix_len]);
            output.push_str(&encoded);
            output.push_str(&line[content.len()..]);
            emitted = true;
        }
        output.into_bytes()
    }
}

fn find_record_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    for index in 0..buffer.len().saturating_sub(1) {
        if buffer[index..].starts_with(b"\n\n") {
            return Some((index, 2));
        }
        if buffer[index..].starts_with(b"\r\n\r\n") {
            return Some((index, 4));
        }
    }
    None
}

#[cfg(test)]
mod tests;
