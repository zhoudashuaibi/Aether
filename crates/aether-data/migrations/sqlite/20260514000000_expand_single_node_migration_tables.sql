CREATE TABLE IF NOT EXISTS api_key_provider_mappings (
    id TEXT PRIMARY KEY NOT NULL,
    api_key_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    priority_adjustment INTEGER NOT NULL DEFAULT 0,
    weight_multiplier REAL NOT NULL DEFAULT 1,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (api_key_id, provider_id)
);
CREATE INDEX IF NOT EXISTS api_key_provider_mappings_api_key_id_idx
    ON api_key_provider_mappings (api_key_id);
CREATE INDEX IF NOT EXISTS api_key_provider_mappings_provider_id_idx
    ON api_key_provider_mappings (provider_id);
CREATE INDEX IF NOT EXISTS idx_apikey_provider_enabled
    ON api_key_provider_mappings (api_key_id, is_enabled);

CREATE TABLE IF NOT EXISTS provider_usage_tracking (
    id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL,
    window_start INTEGER NOT NULL,
    window_end INTEGER NOT NULL,
    total_requests INTEGER NOT NULL DEFAULT 0,
    successful_requests INTEGER NOT NULL DEFAULT 0,
    failed_requests INTEGER NOT NULL DEFAULT 0,
    avg_response_time_ms REAL NOT NULL DEFAULT 0,
    total_response_time_ms REAL NOT NULL DEFAULT 0,
    total_cost_usd REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS provider_usage_tracking_provider_id_idx
    ON provider_usage_tracking (provider_id);
CREATE INDEX IF NOT EXISTS provider_usage_tracking_window_start_idx
    ON provider_usage_tracking (window_start);
CREATE INDEX IF NOT EXISTS idx_provider_window
    ON provider_usage_tracking (provider_id, window_start);
CREATE INDEX IF NOT EXISTS idx_window_time
    ON provider_usage_tracking (window_start, window_end);

CREATE TABLE IF NOT EXISTS usage_routing_snapshots (
    request_id TEXT PRIMARY KEY NOT NULL,
    candidate_id TEXT,
    candidate_index INTEGER,
    key_name TEXT,
    planner_kind TEXT,
    route_family TEXT,
    route_kind TEXT,
    execution_path TEXT,
    local_execution_runtime_miss_reason TEXT,
    selected_provider_id TEXT,
    selected_endpoint_id TEXT,
    selected_provider_api_key_id TEXT,
    has_format_conversion INTEGER,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (request_id) REFERENCES "usage"(request_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_usage_routing_snapshots_route_family_kind
    ON usage_routing_snapshots (route_family, route_kind);
CREATE INDEX IF NOT EXISTS ix_usage_routing_snapshots_candidate_id
    ON usage_routing_snapshots (candidate_id);

CREATE TABLE IF NOT EXISTS stats_summary (
    id TEXT PRIMARY KEY NOT NULL,
    cutoff_date INTEGER NOT NULL,
    all_time_requests INTEGER NOT NULL DEFAULT 0,
    all_time_success_requests INTEGER NOT NULL DEFAULT 0,
    all_time_error_requests INTEGER NOT NULL DEFAULT 0,
    all_time_input_tokens INTEGER NOT NULL DEFAULT 0,
    all_time_output_tokens INTEGER NOT NULL DEFAULT 0,
    all_time_cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    all_time_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    all_time_cost REAL NOT NULL DEFAULT 0,
    all_time_actual_cost REAL NOT NULL DEFAULT 0,
    total_users INTEGER NOT NULL DEFAULT 0,
    active_users INTEGER NOT NULL DEFAULT 0,
    total_api_keys INTEGER NOT NULL DEFAULT 0,
    active_api_keys INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS user_model_usage_counts (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    model TEXT NOT NULL,
    usage_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (user_id, model)
);
CREATE INDEX IF NOT EXISTS idx_user_model_usage_user
    ON user_model_usage_counts (user_id);
CREATE INDEX IF NOT EXISTS idx_user_model_usage_model
    ON user_model_usage_counts (model);

ALTER TABLE stats_hourly
    ADD COLUMN response_time_sum_ms REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN response_time_samples INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN cache_hit_total_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN cache_hit_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN completed_total_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN completed_cache_hit_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN completed_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN completed_cache_creation_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN completed_cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN completed_total_input_context INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN completed_cache_creation_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN completed_cache_read_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN settled_total_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN settled_total_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN settled_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN settled_output_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN settled_cache_creation_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN settled_cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly
    ADD COLUMN settled_first_finalized_at_unix_secs INTEGER;
ALTER TABLE stats_hourly
    ADD COLUMN settled_last_finalized_at_unix_secs INTEGER;

ALTER TABLE stats_hourly_user
    ADD COLUMN cache_creation_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN actual_total_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN response_time_sum_ms REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN response_time_samples INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN settled_total_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN settled_total_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN settled_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN settled_output_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN settled_cache_creation_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN settled_cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user
    ADD COLUMN settled_first_finalized_at_unix_secs INTEGER;
ALTER TABLE stats_hourly_user
    ADD COLUMN settled_last_finalized_at_unix_secs INTEGER;

ALTER TABLE stats_hourly_user_model
    ADD COLUMN response_time_sum_ms REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_user_model
    ADD COLUMN response_time_samples INTEGER NOT NULL DEFAULT 0;

ALTER TABLE stats_hourly_model
    ADD COLUMN response_time_sum_ms REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_hourly_model
    ADD COLUMN response_time_samples INTEGER NOT NULL DEFAULT 0;

ALTER TABLE stats_daily
    ADD COLUMN effective_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN total_input_context INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN response_time_sum_ms REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN response_time_samples INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN cache_creation_ephemeral_5m_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN cache_creation_ephemeral_1h_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN cache_hit_total_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN cache_hit_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN completed_total_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN completed_cache_hit_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN completed_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN completed_cache_creation_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN completed_cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN completed_total_input_context INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN completed_cache_creation_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN completed_cache_read_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN settled_total_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN settled_total_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN settled_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN settled_output_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN settled_cache_creation_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN settled_cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily
    ADD COLUMN settled_first_finalized_at_unix_secs INTEGER;
ALTER TABLE stats_daily
    ADD COLUMN settled_last_finalized_at_unix_secs INTEGER;

ALTER TABLE stats_daily_model
    ADD COLUMN cache_creation_ephemeral_5m_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily_model
    ADD COLUMN cache_creation_ephemeral_1h_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_daily_model
    ADD COLUMN response_time_sum_ms REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_daily_model
    ADD COLUMN response_time_samples INTEGER NOT NULL DEFAULT 0;

ALTER TABLE stats_user_daily
    ADD COLUMN actual_total_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN response_time_sum_ms REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN response_time_samples INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN effective_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN total_input_context INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN cache_creation_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN cache_read_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN cache_creation_ephemeral_5m_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN cache_creation_ephemeral_1h_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN settled_total_cost REAL NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN settled_total_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN settled_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN settled_output_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN settled_cache_creation_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN settled_cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stats_user_daily
    ADD COLUMN settled_first_finalized_at_unix_secs INTEGER;
ALTER TABLE stats_user_daily
    ADD COLUMN settled_last_finalized_at_unix_secs INTEGER;

CREATE TABLE IF NOT EXISTS stats_user_summary (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    cutoff_date INTEGER NOT NULL,
    all_time_requests INTEGER NOT NULL DEFAULT 0,
    all_time_success_requests INTEGER NOT NULL DEFAULT 0,
    all_time_error_requests INTEGER NOT NULL DEFAULT 0,
    all_time_input_tokens INTEGER NOT NULL DEFAULT 0,
    all_time_output_tokens INTEGER NOT NULL DEFAULT 0,
    all_time_cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    all_time_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    all_time_cost REAL NOT NULL DEFAULT 0,
    all_time_actual_cost REAL NOT NULL DEFAULT 0,
    active_days INTEGER NOT NULL DEFAULT 0,
    first_active_date INTEGER,
    last_active_date INTEGER,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_summary_cutoff_date
    ON stats_user_summary (cutoff_date);

CREATE TABLE IF NOT EXISTS stats_user_daily_model (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    date INTEGER NOT NULL,
    model TEXT NOT NULL,
    total_requests INTEGER NOT NULL DEFAULT 0,
    success_requests INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    effective_input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    total_input_context INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_ephemeral_5m_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_ephemeral_1h_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost REAL NOT NULL DEFAULT 0,
    actual_total_cost REAL NOT NULL DEFAULT 0,
    response_time_sum_ms REAL NOT NULL DEFAULT 0,
    response_time_samples INTEGER NOT NULL DEFAULT 0,
    successful_response_time_sum_ms REAL NOT NULL DEFAULT 0,
    successful_response_time_samples INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, date, model)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_model_date
    ON stats_user_daily_model (date);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_model_user_id
    ON stats_user_daily_model (user_id);

CREATE TABLE IF NOT EXISTS stats_user_daily_provider (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    date INTEGER NOT NULL,
    provider_name TEXT NOT NULL,
    total_requests INTEGER NOT NULL DEFAULT 0,
    success_requests INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    effective_input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    total_input_context INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_ephemeral_5m_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_ephemeral_1h_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost REAL NOT NULL DEFAULT 0,
    actual_total_cost REAL NOT NULL DEFAULT 0,
    response_time_sum_ms REAL NOT NULL DEFAULT 0,
    response_time_samples INTEGER NOT NULL DEFAULT 0,
    successful_response_time_sum_ms REAL NOT NULL DEFAULT 0,
    successful_response_time_samples INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, date, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_provider_date
    ON stats_user_daily_provider (date);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_provider_user_id
    ON stats_user_daily_provider (user_id);

CREATE TABLE IF NOT EXISTS stats_user_daily_api_format (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    date INTEGER NOT NULL,
    api_format TEXT NOT NULL,
    total_requests INTEGER NOT NULL DEFAULT 0,
    success_requests INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    effective_input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    total_input_context INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_ephemeral_5m_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_ephemeral_1h_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost REAL NOT NULL DEFAULT 0,
    actual_total_cost REAL NOT NULL DEFAULT 0,
    response_time_sum_ms REAL NOT NULL DEFAULT 0,
    response_time_samples INTEGER NOT NULL DEFAULT 0,
    successful_response_time_sum_ms REAL NOT NULL DEFAULT 0,
    successful_response_time_samples INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, date, api_format)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_api_format_date
    ON stats_user_daily_api_format (date);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_api_format_user_id
    ON stats_user_daily_api_format (user_id);

CREATE TABLE IF NOT EXISTS stats_daily_model_provider (
    id TEXT PRIMARY KEY NOT NULL,
    date INTEGER NOT NULL,
    model TEXT NOT NULL,
    provider_name TEXT NOT NULL,
    total_requests INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost REAL NOT NULL DEFAULT 0,
    response_time_sum_ms REAL NOT NULL DEFAULT 0,
    response_time_samples INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (date, model, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_stats_daily_model_provider_date
    ON stats_daily_model_provider (date);
CREATE INDEX IF NOT EXISTS idx_stats_daily_model_provider_date_model_provider
    ON stats_daily_model_provider (date, model, provider_name);

CREATE TABLE IF NOT EXISTS stats_user_daily_model_provider (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    date INTEGER NOT NULL,
    model TEXT NOT NULL,
    provider_name TEXT NOT NULL,
    total_requests INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost REAL NOT NULL DEFAULT 0,
    response_time_sum_ms REAL NOT NULL DEFAULT 0,
    response_time_samples INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, date, model, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_model_provider_date
    ON stats_user_daily_model_provider (date);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_model_provider_user_date
    ON stats_user_daily_model_provider (user_id, date);

CREATE TABLE IF NOT EXISTS stats_daily_cost_savings (
    id TEXT PRIMARY KEY NOT NULL,
    date INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_cost REAL NOT NULL DEFAULT 0,
    cache_creation_cost REAL NOT NULL DEFAULT 0,
    estimated_full_cost REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (date)
);
CREATE INDEX IF NOT EXISTS idx_stats_daily_cost_savings_date
    ON stats_daily_cost_savings (date);

CREATE TABLE IF NOT EXISTS stats_daily_cost_savings_provider (
    id TEXT PRIMARY KEY NOT NULL,
    date INTEGER NOT NULL,
    provider_name TEXT NOT NULL,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_cost REAL NOT NULL DEFAULT 0,
    cache_creation_cost REAL NOT NULL DEFAULT 0,
    estimated_full_cost REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (date, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_stats_daily_cost_savings_provider_date
    ON stats_daily_cost_savings_provider (date);
CREATE INDEX IF NOT EXISTS idx_stats_daily_cost_savings_provider_date_provider
    ON stats_daily_cost_savings_provider (date, provider_name);

CREATE TABLE IF NOT EXISTS stats_daily_cost_savings_model (
    id TEXT PRIMARY KEY NOT NULL,
    date INTEGER NOT NULL,
    model TEXT NOT NULL,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_cost REAL NOT NULL DEFAULT 0,
    cache_creation_cost REAL NOT NULL DEFAULT 0,
    estimated_full_cost REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (date, model)
);
CREATE INDEX IF NOT EXISTS idx_stats_daily_cost_savings_model_date
    ON stats_daily_cost_savings_model (date);
CREATE INDEX IF NOT EXISTS idx_stats_daily_cost_savings_model_date_model
    ON stats_daily_cost_savings_model (date, model);

CREATE TABLE IF NOT EXISTS stats_daily_cost_savings_model_provider (
    id TEXT PRIMARY KEY NOT NULL,
    date INTEGER NOT NULL,
    model TEXT NOT NULL,
    provider_name TEXT NOT NULL,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_cost REAL NOT NULL DEFAULT 0,
    cache_creation_cost REAL NOT NULL DEFAULT 0,
    estimated_full_cost REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (date, model, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_stats_daily_cost_savings_model_provider_date
    ON stats_daily_cost_savings_model_provider (date);
CREATE INDEX IF NOT EXISTS idx_stats_daily_cost_savings_model_provider_date_dims
    ON stats_daily_cost_savings_model_provider (date, model, provider_name);

CREATE TABLE IF NOT EXISTS stats_user_daily_cost_savings (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    date INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_cost REAL NOT NULL DEFAULT 0,
    cache_creation_cost REAL NOT NULL DEFAULT 0,
    estimated_full_cost REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, date)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_cost_savings_date
    ON stats_user_daily_cost_savings (date);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_cost_savings_user_date
    ON stats_user_daily_cost_savings (user_id, date);

CREATE TABLE IF NOT EXISTS stats_user_daily_cost_savings_provider (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    date INTEGER NOT NULL,
    provider_name TEXT NOT NULL,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_cost REAL NOT NULL DEFAULT 0,
    cache_creation_cost REAL NOT NULL DEFAULT 0,
    estimated_full_cost REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, date, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_cost_savings_provider_date
    ON stats_user_daily_cost_savings_provider (date);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_cost_savings_provider_user_date
    ON stats_user_daily_cost_savings_provider (user_id, date);

CREATE TABLE IF NOT EXISTS stats_user_daily_cost_savings_model (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    date INTEGER NOT NULL,
    model TEXT NOT NULL,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_cost REAL NOT NULL DEFAULT 0,
    cache_creation_cost REAL NOT NULL DEFAULT 0,
    estimated_full_cost REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, date, model)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_cost_savings_model_date
    ON stats_user_daily_cost_savings_model (date);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_cost_savings_model_user_date
    ON stats_user_daily_cost_savings_model (user_id, date);

CREATE TABLE IF NOT EXISTS stats_user_daily_cost_savings_model_provider (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT,
    date INTEGER NOT NULL,
    model TEXT NOT NULL,
    provider_name TEXT NOT NULL,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_cost REAL NOT NULL DEFAULT 0,
    cache_creation_cost REAL NOT NULL DEFAULT 0,
    estimated_full_cost REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, date, model, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_cost_savings_model_provider_date
    ON stats_user_daily_cost_savings_model_provider (date);
CREATE INDEX IF NOT EXISTS idx_stats_user_daily_cost_savings_model_provider_user_date
    ON stats_user_daily_cost_savings_model_provider (user_id, date);
