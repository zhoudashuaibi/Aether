//! Native Command Code transport. Protocol reference: CommandCodeGo-manager
//! v0.2.10 (MIT); attribution is retained in THIRD_PARTY_NOTICES.

use std::collections::{BTreeMap, HashMap};

use http::HeaderMap;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::snapshot::GatewayProviderTransportSnapshot;

mod fingerprint;
pub use fingerprint::fingerprint;

pub const PROVIDER_TYPE: &str = "command_code";
pub const ENVELOPE_NAME: &str =
    aether_ai_formats::provider_compat::surfaces::COMMAND_CODE_ENVELOPE_NAME;
pub const BASE_URL: &str = "https://api.commandcode.ai";
pub const GENERATE_PATH: &str = "/alpha/generate";
pub const CLI_VERSION: &str = "1.53.1";
pub const PROJECT_DIR: &str = r"C:\Users\dev\projects\app";
pub const INTERNAL_HEADER: &str = "x-aether-command-code-runtime";

pub fn is_command_code(transport: &GatewayProviderTransportSnapshot) -> bool {
    transport
        .provider
        .provider_type
        .trim()
        .eq_ignore_ascii_case(PROVIDER_TYPE)
}

pub fn envelope_name(transport: &GatewayProviderTransportSnapshot) -> Option<&'static str> {
    is_command_code(transport).then_some(ENVELOPE_NAME)
}

/// Rebuild the runtime marker from trusted provider metadata after auth cleanup.
pub fn mark_execution_headers(provider_type: Option<&str>, headers: &mut BTreeMap<String, String>) {
    if provider_type.is_some_and(|value| value.trim().eq_ignore_ascii_case(PROVIDER_TYPE)) {
        headers.insert(INTERNAL_HEADER.to_string(), "1".to_string());
    }
}

/// Called after standard conversion, model mapping and operator body rules.
pub fn adapt_request(
    transport: &GatewayProviderTransportSnapshot,
    headers: &HeaderMap,
    downstream_key_id: &str,
    original: &Value,
    body: &mut Value,
) -> Result<(), &'static str> {
    if !is_command_code(transport) {
        return Ok(());
    }
    if transport.endpoint.api_format != "openai:chat" {
        return Err("Command Code supports only the chat endpoint anchor");
    }
    validate_credential(&transport.key.decrypted_api_key)?;
    if original.get("store").and_then(Value::as_bool) == Some(true)
        || original
            .get("previous_response_id")
            .is_some_and(|value| !value.is_null())
    {
        return Err(
            "Command Code requires full conversation history and does not support stored responses",
        );
    }
    let session = session_id(headers, original, &transport.key.id, downstream_key_id);
    *body = build_request(body, session)?;
    Ok(())
}

pub fn validate_credential(key: &str) -> Result<(), &'static str> {
    let key = key.trim();
    if !key.starts_with("user_")
        || key.len() <= 5
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("Command Code requires a user_* upstream credential");
    }
    Ok(())
}

fn session_id(
    headers: &HeaderMap,
    original: &Value,
    upstream_key_id: &str,
    downstream_key_id: &str,
) -> Uuid {
    let requested = ["x-session-id", "x-claude-code-session-id", "session_id"]
        .iter()
        .find_map(|name| {
            headers
                .get(*name)
                .and_then(|value| value.to_str().ok())
                .filter(|value| (8..=256).contains(&value.len()))
        })
        .or_else(|| {
            original
                .get("prompt_cache_key")
                .and_then(Value::as_str)
                .filter(|value| (8..=256).contains(&value.len()))
        });
    // Use the authenticated principal from the planner, never a caller-supplied
    // identity header or an ambiguous choice between authentication channels.
    let caller = (!downstream_key_id.is_empty()).then_some(downstream_key_id);
    match requested.zip(caller) {
        Some((session, caller)) => Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("aether-command-code\0{upstream_key_id}\0{caller}\0{session}").as_bytes(),
        ),
        None => Uuid::new_v4(),
    }
}

fn build_request(body: &Value, session: Uuid) -> Result<Value, &'static str> {
    let object = body
        .as_object()
        .ok_or("Command Code request must be an object")?;
    for (field, value) in object {
        if value.is_null() {
            continue;
        }
        if !matches!(
            field.as_str(),
            "model"
                | "messages"
                | "max_tokens"
                | "max_completion_tokens"
                | "stream"
                | "stream_options"
                | "temperature"
                | "reasoning_effort"
                | "tools"
                | "tool_choice"
                | "parallel_tool_calls"
                | "prompt_cache_key"
                | "store"
        ) {
            return Err("Command Code request contains an unsupported parameter");
        }
    }
    let model = string(body, "model")?;
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .filter(|messages| !messages.is_empty())
        .ok_or("Command Code requires messages")?;
    let max_tokens = body
        .get("max_completion_tokens")
        .filter(|value| !value.is_null())
        .or_else(|| body.get("max_tokens").filter(|value| !value.is_null()))
        .map(|value| {
            value
                .as_u64()
                .filter(|n| (1..=200_000).contains(n))
                .ok_or("Command Code max_tokens must be between 1 and 200000")
        })
        .transpose()?
        .unwrap_or(64_000);
    if let (Some(a), Some(b)) = (body.get("max_tokens"), body.get("max_completion_tokens")) {
        if !a.is_null() && !b.is_null() && a != b {
            return Err("Command Code token limits conflict");
        }
    }
    if body
        .get("store")
        .is_some_and(|value| !value.is_null() && value != false)
    {
        return Err("Command Code does not support stored responses");
    }
    if let Some(options) = body.get("stream_options").filter(|value| !value.is_null()) {
        check_fields(options, &["include_usage"])?;
        if options
            .get("include_usage")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err("Command Code include_usage must be a boolean");
        }
    }
    let mut tool_names = HashMap::new();
    let mut system = Vec::new();
    let mut history = Vec::new();
    for message in messages {
        let role = string(message, "role")?;
        check_fields(
            message,
            match role {
                "assistant" => &["role", "content", "reasoning_content", "tool_calls"],
                "tool" => &["role", "content", "tool_call_id", "name"],
                _ => &["role", "content"],
            },
        )?;
        if matches!(role, "system" | "developer") {
            system.extend(text_parts(&message["content"])?);
            continue;
        }
        let mut content = Vec::new();
        match role {
            "user" => content.extend(text_parts(&message["content"])?),
            "assistant" => {
                if let Some(reasoning) = message
                    .get("reasoning_content")
                    .filter(|value| !value.is_null())
                {
                    content.push(json!({"type": "reasoning", "text": reasoning.as_str()
                        .ok_or("Command Code reasoning_content must be text")?}));
                }
                content.extend(text_parts(&message["content"])?);
                if let Some(calls) = message.get("tool_calls").filter(|value| !value.is_null()) {
                    for call in calls
                        .as_array()
                        .ok_or("Command Code tool_calls must be an array")?
                    {
                        check_fields(call, &["id", "type", "function"])?;
                        if call.get("type").and_then(Value::as_str) != Some("function") {
                            return Err("Command Code only supports function tools");
                        }
                        let function = &call["function"];
                        check_fields(function, &["name", "arguments"])?;
                        let id = string(call, "id")?;
                        let name = string(function, "name")?;
                        if tool_names.insert(id, name).is_some() {
                            return Err("Command Code tool call ids must be unique");
                        }
                        let args: Value = serde_json::from_str(string(function, "arguments")?)
                            .map_err(|_| "Command Code tool arguments must be valid JSON")?;
                        if !args.is_object() {
                            return Err("Command Code tool arguments must be an object");
                        }
                        content.push(json!({"type": "tool-call", "toolCallId": id,
                            "toolName": name, "input": args}));
                    }
                }
            }
            "tool" => {
                let id = string(message, "tool_call_id")?;
                let name = tool_names
                    .get(id)
                    .copied()
                    .ok_or("Command Code tool result has no matching call")?;
                if message
                    .get("name")
                    .is_some_and(|value| !value.is_null() && value != name)
                {
                    return Err("Command Code tool result name does not match its call");
                }
                let text = text_parts(&message["content"])?;
                let output = text
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                content.push(
                    json!({"type": "tool-result", "toolCallId": id, "toolName": name,
                    "output": {"type": "text", "value": output}}),
                );
            }
            _ => return Err("Command Code message role is unsupported"),
        }
        history.push(json!({"role": role, "content": content}));
    }
    if history.is_empty() {
        return Err("Command Code requires a conversation message");
    }
    let system_len = system.len();
    for part in system.iter_mut().take(system_len.saturating_sub(1)) {
        part["text"] = json!(format!("{}\n", part["text"].as_str().unwrap_or_default()));
    }
    if system.is_empty() {
        system.push(json!({"type": "text", "text": " "}));
    }
    if body
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .is_some()
        && !system
            .iter()
            .any(|part| part.get("cache_control").is_some())
    {
        if let Some(last) = system.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }
    }
    let mut tools = Vec::new();
    if let Some(definitions) = body.get("tools").filter(|value| !value.is_null()) {
        for tool in definitions
            .as_array()
            .ok_or("Command Code tools must be an array")?
        {
            check_fields(tool, &["type", "function"])?;
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                return Err("Command Code only supports function tools");
            }
            let function = &tool["function"];
            check_fields(function, &["name", "description", "parameters", "strict"])?;
            if function
                .get("strict")
                .is_some_and(|value| !value.is_null() && value != false)
            {
                return Err(
                    "Command Code does not support strict or extended function definitions",
                );
            }
            if function
                .get("description")
                .is_some_and(|value| !value.is_null() && !value.is_string())
            {
                return Err("Command Code tool description must be text");
            }
            let schema = function
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            if !schema.is_object() {
                return Err("Command Code tool schema must be an object");
            }
            tools.push(json!({"name": string(function, "name")?,
                "description": function.get("description").and_then(Value::as_str).unwrap_or(""),
                "input_schema": schema}));
        }
    }
    let mut params = json!({"model": model, "messages": history, "max_tokens": max_tokens,
        "stream": true, "system": system, "tools": tools});
    for field in ["temperature", "reasoning_effort", "parallel_tool_calls"] {
        if let Some(value) = body.get(field).filter(|value| !value.is_null()) {
            let valid = match field {
                "temperature" => value
                    .as_f64()
                    .is_some_and(|n| n.is_finite() && (0.0..=2.0).contains(&n)),
                "parallel_tool_calls" => value.is_boolean(),
                _ => value
                    .as_str()
                    .is_some_and(|s| matches!(s, "low" | "medium" | "high")),
            };
            if !valid {
                return Err("Command Code generation parameter is invalid or unsupported");
            }
            params[field] = value.clone();
        }
    }
    if let Some(choice) = body.get("tool_choice").filter(|value| !value.is_null()) {
        params["tool_choice"] = match choice.as_str() {
            Some("auto") => json!({"type": "auto"}),
            Some("none") => json!({"type": "none"}),
            Some("required") => json!({"type": "any"}),
            None if choice["type"] == "function" => {
                check_fields(choice, &["type", "function"])?;
                check_fields(&choice["function"], &["name"])?;
                let name = string(&choice["function"], "name")?;
                if !params["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name))
                {
                    return Err("Command Code tool_choice must name a declared tool");
                }
                json!({"type": "tool", "name": name})
            }
            _ => return Err("Command Code tool_choice is unsupported"),
        };
    }
    Ok(json!({
        "config": {"workingDir": PROJECT_DIR, "date": chrono::Utc::now().format("%Y-%m-%d").to_string(),
            "environment": "win32", "structure": [], "isGitRepo": false,
            "currentBranch": "", "mainBranch": "", "gitStatus": "", "recentCommits": []},
        "memory": null, "taste": null, "skills": null, "permissionMode": "standard",
        "threadId": session.to_string(), "mode": "agent", "params": params
    }))
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, &'static str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Command Code request is missing a required string")
}

fn check_fields(value: &Value, allowed: &[&str]) -> Result<(), &'static str> {
    if value
        .as_object()
        .is_none_or(|object| object.keys().any(|key| !allowed.contains(&key.as_str())))
    {
        return Err("Command Code request contains unsupported fields");
    }
    Ok(())
}

fn text_parts(value: &Value) -> Result<Vec<Value>, &'static str> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(text) => Ok(vec![json!({"type": "text", "text": text})]),
        Value::Array(parts) => parts
            .iter()
            .map(|part| {
                if part["type"] != "text"
                    || !part["text"].is_string()
                    || part.as_object().is_none_or(|part| {
                        part.keys()
                            .any(|key| !matches!(key.as_str(), "type" | "text" | "cache_control"))
                    })
                {
                    return Err("Command Code currently supports text content only");
                }
                Ok(part.clone())
            })
            .collect(),
        _ => Err("Command Code content must be text or text blocks"),
    }
}

pub fn build_headers(
    input: crate::StandardProviderRequestHeadersInput<'_>,
) -> Option<crate::StandardProviderRequestHeaders> {
    validate_credential(&input.transport.key.decrypted_api_key).ok()?;
    let auth = format!("Bearer {}", input.transport.key.decrypted_api_key.trim());
    let mut headers = BTreeMap::from([
        (INTERNAL_HEADER.to_string(), "1".to_string()),
        ("authorization".to_string(), auth.clone()),
        ("content-type".to_string(), "application/json".to_string()),
        ("accept".to_string(), "application/x-ndjson".to_string()),
        ("accept-encoding".to_string(), "identity".to_string()),
        ("user-agent".to_string(), "cli".to_string()),
        (
            "x-command-code-version".to_string(),
            CLI_VERSION.to_string(),
        ),
        ("x-cli-environment".to_string(), "production".to_string()),
        (
            "x-project-slug".to_string(),
            "c-users-dev-projects-app".to_string(),
        ),
        ("x-taste-learning".to_string(), "false".to_string()),
        (
            "x-session-id".to_string(),
            input
                .provider_request_body
                .get("threadId")?
                .as_str()?
                .to_string(),
        ),
        (
            "traceparent".to_string(),
            format!(
                "00-{}-{}-01",
                Uuid::new_v4().simple(),
                &Uuid::new_v4().simple().to_string()[..16]
            ),
        ),
    ]);
    if input
        .headers
        .get("x-cmd-zdr")
        .is_some_and(|value| value == "1")
    {
        headers.insert("x-cmd-zdr".to_string(), "1".to_string());
    }
    if !crate::apply_local_header_rules_with_request_headers(
        &mut headers,
        input.header_rules,
        &[
            "authorization",
            "content-type",
            "x-session-id",
            INTERNAL_HEADER,
            "x-command-code-version",
        ],
        input.provider_request_body,
        Some(input.original_request_body),
        Some(input.headers),
    ) {
        return None;
    }
    Some(crate::StandardProviderRequestHeaders {
        headers,
        auth_header: "authorization".to_string(),
        auth_value: auth,
    })
}

pub fn credential_digest(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_tools_history_and_keeps_original_tool_names() {
        let body = build_request(&json!({"model": "test", "messages": [
            {"role": "system", "content": "system"},
            {"role": "assistant", "content": null, "reasoning_content": "why",
                "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "task_output", "arguments": "{\"id\":1}"}}]},
            {"role": "tool", "tool_call_id": "call_1", "content": "result"}],
            "tools": [{"type": "function", "function": {"name": "task_output", "parameters": {"type": "object"}}}],
            "max_completion_tokens": 12}), Uuid::nil()).unwrap();
        assert_eq!(body["params"]["stream"], true);
        assert_eq!(body["params"]["max_tokens"], 12);
        assert_eq!(
            body["params"]["messages"][0]["content"][0]["type"],
            "reasoning"
        );
        assert_eq!(
            body["params"]["messages"][1]["content"][0]["toolName"],
            "task_output"
        );
        assert_eq!(body["params"]["tools"][0]["name"], "task_output");
    }

    #[test]
    fn rejects_unsupported_fields_images_and_invalid_limits() {
        for extra in [
            json!({"stop": ["end"]}),
            json!({"top_p": 0.9}),
            json!({"store": true}),
            json!({"max_tokens": 0}),
            json!({"max_tokens": 200001}),
            json!({"stream_options": {"include_usage": true, "unknown": true}}),
            json!({"tools": [{"type":"function", "unexpected": true, "function":{"name":"lookup"}}]}),
            json!({"messages": [{"role":"user", "content":"hi", "name":"unmapped"}]}),
            json!({"messages": [{"role":"tool", "tool_call_id":"missing", "name":"lookup", "content":"result"}]}),
            json!({"messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.test/image"}}]}]}),
        ] {
            let mut body =
                json!({"model": "test", "messages": [{"role": "user", "content": "hi"}]});
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(build_request(&body, Uuid::nil()).is_err());
        }
    }

    #[test]
    fn scopes_sessions_to_callers_and_never_reuses_an_implicit_session() {
        let mut headers = HeaderMap::new();
        headers.insert("x-session-id", "same-session".parse().unwrap());
        headers.insert("authorization", "Bearer downstream-a".parse().unwrap());
        let a = session_id(&headers, &json!({}), "upstream", "caller-a");
        assert_eq!(a, session_id(&headers, &json!({}), "upstream", "caller-a"));
        assert_ne!(a, session_id(&headers, &json!({}), "upstream", "caller-b"));
        assert_ne!(
            a,
            session_id(&headers, &json!({}), "other-upstream", "caller-a")
        );
        headers.insert("authorization", "Bearer downstream-b".parse().unwrap());
        assert_eq!(a, session_id(&headers, &json!({}), "upstream", "caller-a"));
        assert_ne!(
            session_id(&headers, &json!({}), "upstream", ""),
            session_id(&headers, &json!({}), "upstream", "")
        );
        headers.remove("x-session-id");
        assert_ne!(
            session_id(&headers, &json!({}), "upstream", "caller-a"),
            session_id(&headers, &json!({}), "upstream", "caller-a")
        );
    }
}
