//! Optional CLI initialization shares the selected generation transport.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use aether_contracts::{ExecutionPlan, ExecutionTimeouts, RequestBody};
use aether_provider_transport::command_code::{
    credential_digest, fingerprint, CLI_VERSION, GENERATE_PATH, INTERNAL_HEADER,
};
use serde_json::json;
use tokio::sync::Mutex as AsyncMutex;

use super::transport::send_command_code_initialization;

type InitEntry = Arc<AsyncMutex<Instant>>;
static INIT_CACHE: OnceLock<Mutex<HashMap<String, InitEntry>>> = OnceLock::new();
const MAX_INIT_KEYS: usize = 4096;

pub(super) fn is_generation(plan: &ExecutionPlan) -> bool {
    plan.headers
        .get(INTERNAL_HEADER)
        .is_some_and(|value| value == "1")
        && plan.method == "POST"
        && plan.url.ends_with(GENERATE_PATH)
}

pub(super) async fn ensure_initialized(plan: &ExecutionPlan, state: Option<&crate::AppState>) {
    if !is_generation(plan) {
        return;
    }
    let Some(secret) = plan
        .headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return;
    };
    let base = plan.url.strip_suffix(GENERATE_PATH).unwrap_or_default();
    let cache_key = credential_digest(&format!(
        "{base}\0{secret}\0{}\0{}\0{}",
        serde_json::to_string(&plan.proxy).unwrap_or_default(),
        serde_json::to_string(&plan.transport_profile).unwrap_or_default(),
        plan.headers
            .get("x-cmd-zdr")
            .map(String::as_str)
            .unwrap_or_default()
    ));
    let entry = {
        let cache = INIT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let Ok(mut cache) = cache.lock() else {
            return;
        };
        if cache.len() >= MAX_INIT_KEYS && !cache.contains_key(&cache_key) {
            cache.retain(|_, entry| {
                Arc::strong_count(entry) > 1
                    || entry.try_lock().map_or(true, |next| *next > Instant::now())
            });
            if cache.len() >= MAX_INIT_KEYS {
                return;
            }
        }
        Arc::clone(
            cache
                .entry(cache_key)
                .or_insert_with(|| Arc::new(AsyncMutex::new(Instant::now()))),
        )
    };
    // A cancelled initializer releases the mutex; another request can retry.
    let Ok(mut next_init) = tokio::time::timeout(Duration::from_secs(6), entry.lock()).await else {
        return;
    };
    if *next_init > Instant::now() {
        return;
    }
    let fingerprint_plan =
        initialization_plan(plan, base, "/alpha/fingerprint/record", fingerprint(secret));
    let lifecycle_plan = initialization_plan(
        plan,
        base,
        "/alpha/lifecycle-events",
        json!({
            "eventType": "cli_session_exists", "metadata": {
                "sessionId": format!("sess_{}", uuid::Uuid::new_v4().simple()),
                "cliVersion": CLI_VERSION, "mode": "interactive", "os": "win32-x64"
            }
        }),
    );
    let initialized = tokio::time::timeout(Duration::from_secs(5), async {
        let (fingerprint_ok, lifecycle_ok) = tokio::join!(
            send_initialization(&fingerprint_plan, state),
            send_initialization(&lifecycle_plan, state)
        );
        fingerprint_ok && lifecycle_ok
    })
    .await
    .unwrap_or(false);
    let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]);
    *next_init = Instant::now()
        + if initialized {
            Duration::from_secs(8 * 3600 + jitter * 7200 / 256)
        } else {
            Duration::from_secs(60 + jitter * 240 / 256)
        };
    if !initialized {
        tracing::debug!(provider_id = %plan.provider_id, key_id = %plan.key_id,
            "Command Code initialization did not complete; continuing generation with retry cooldown");
    }
}

fn initialization_plan(
    plan: &ExecutionPlan,
    base: &str,
    path: &str,
    body: serde_json::Value,
) -> ExecutionPlan {
    let mut init = plan.clone();
    init.url = format!("{base}{path}");
    init.stream = false;
    init.content_encoding = None;
    init.body = RequestBody::from_json(body);
    // Keep operator transport controls (e.g. TLS/HTTP version) on the same route.
    // Drop the generation marker so an initialization request cannot recurse.
    init.headers.retain(|key, _| {
        key.starts_with("x-aether-") && key != INTERNAL_HEADER
            || matches!(
                key.as_str(),
                "authorization"
                    | "content-type"
                    | "user-agent"
                    | "accept-encoding"
                    | "x-command-code-version"
                    | "x-cli-environment"
                    | "x-cmd-zdr"
            )
    });
    init.timeouts = Some(ExecutionTimeouts {
        connect_ms: Some(5000),
        first_byte_ms: Some(5000),
        total_ms: Some(5000),
        ..Default::default()
    });
    init
}

async fn send_initialization(plan: &ExecutionPlan, state: Option<&crate::AppState>) -> bool {
    let Ok(body) = serde_json::to_vec(&plan.body.json_body) else {
        return false;
    };
    send_command_code_initialization(state, plan, body).await
}

#[cfg(test)]
mod tests;
