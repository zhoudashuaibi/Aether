use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use sqlx::{Column, Row, TypeInfo, ValueRef};

use crate::error::SqlResultExt;
use crate::{DataLayerError, DatabaseDriver, SqlDatabaseConfig};

pub const EXPORT_FORMAT_VERSION: u32 = 1;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ExportDomain {
    Users,
    ApiKeys,
    Providers,
    ProviderKeys,
    Endpoints,
    GlobalModels,
    Models,
    AuthModules,
    OAuthProviders,
    UserOAuthLinks,
    UserGroups,
    UserGroupMembers,
    ProxyNodes,
    SystemConfigs,
    Wallets,
    Usage,
    Billing,
    Stats,
    Auxiliary,
}

impl ExportDomain {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Users => "users",
            Self::ApiKeys => "api_keys",
            Self::Providers => "providers",
            Self::ProviderKeys => "provider_keys",
            Self::Endpoints => "endpoints",
            Self::Models => "models",
            Self::GlobalModels => "global_models",
            Self::AuthModules => "auth_modules",
            Self::OAuthProviders => "oauth_providers",
            Self::UserOAuthLinks => "user_oauth_links",
            Self::UserGroups => "user_groups",
            Self::UserGroupMembers => "user_group_members",
            Self::ProxyNodes => "proxy_nodes",
            Self::SystemConfigs => "system_configs",
            Self::Wallets => "wallets",
            Self::Usage => "usage",
            Self::Billing => "billing",
            Self::Stats => "stats",
            Self::Auxiliary => "auxiliary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DataExportManifest {
    pub format_version: u32,
    pub created_at_unix_secs: u64,
    pub source_driver: Option<DatabaseDriver>,
    pub domains: Vec<ExportDomain>,
}

impl DataExportManifest {
    pub fn new(
        created_at_unix_secs: u64,
        source_driver: Option<DatabaseDriver>,
        domains: Vec<ExportDomain>,
    ) -> Self {
        let mut domains = domains;
        domains.sort();
        domains.dedup();
        Self {
            format_version: EXPORT_FORMAT_VERSION,
            created_at_unix_secs,
            source_driver,
            domains,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "record_type", rename_all = "snake_case")]
pub enum DataExportRecord {
    Manifest {
        manifest: DataExportManifest,
    },
    Row {
        domain: ExportDomain,
        id: String,
        payload: Value,
    },
}

impl DataExportRecord {
    pub fn manifest(manifest: DataExportManifest) -> Self {
        Self::Manifest { manifest }
    }

    pub fn row(domain: ExportDomain, id: impl Into<String>, payload: Value) -> Self {
        Self::Row {
            domain,
            id: id.into(),
            payload,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataImportPlan {
    pub manifest: DataExportManifest,
    pub rows_by_domain: BTreeMap<ExportDomain, Vec<ExportRow>>,
}

impl DataImportPlan {
    pub fn rows(&self, domain: ExportDomain) -> &[ExportRow] {
        self.rows_by_domain
            .get(&domain)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportRow {
    pub id: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataCopyOptions {
    pub omit_request_body_details: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PostgresImportColumn {
    data_type: String,
    udt_name: String,
    is_nullable: bool,
    has_default: bool,
}

type PostgresImportColumns = BTreeMap<String, PostgresImportColumn>;
type ImportColumnNames = BTreeSet<String>;

#[derive(Debug, Clone, Copy)]
struct StatsTableSpec {
    table_name: &'static str,
    id_column: &'static str,
    timestamp_columns: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct AuxiliaryTableSpec {
    table_name: &'static str,
    id_columns: &'static [&'static str],
    timestamp_columns: &'static [&'static str],
}

const USAGE_REQUEST_BODY_DETAIL_COLUMNS: &[&str] = &[
    "request_body",
    "response_body",
    "provider_request_body",
    "client_response_body",
    "request_body_compressed",
    "response_body_compressed",
    "provider_request_body_compressed",
    "client_response_body_compressed",
];

const STATS_DATE_TIMESTAMPS: &[&str] = &["date", "created_at", "updated_at"];
const STATS_HOUR_TIMESTAMPS: &[&str] = &["hour_utc", "created_at", "updated_at"];
const STATS_ROLLUP_TIMESTAMPS: &[&str] = &["date", "aggregated_at", "created_at", "updated_at"];
const STATS_HOURLY_ROLLUP_TIMESTAMPS: &[&str] =
    &["hour_utc", "aggregated_at", "created_at", "updated_at"];
const STATS_SUMMARY_TIMESTAMPS: &[&str] = &["cutoff_date", "created_at", "updated_at"];
const STATS_USER_SUMMARY_TIMESTAMPS: &[&str] = &[
    "cutoff_date",
    "first_active_date",
    "last_active_date",
    "created_at",
    "updated_at",
];
const ID_COLUMNS: &[&str] = &["id"];
const REQUEST_ID_COLUMNS: &[&str] = &["request_id"];
const PROXY_NODE_METRIC_COLUMNS: &[&str] = &["node_id", "bucket_start_unix_secs"];
const AUDIT_TIMESTAMPS: &[&str] = &["created_at"];
const ANNOUNCEMENT_TIMESTAMPS: &[&str] = &["start_time", "end_time", "created_at", "updated_at"];
const ANNOUNCEMENT_READ_TIMESTAMPS: &[&str] = &["read_at"];
const MANAGEMENT_TOKEN_TIMESTAMPS: &[&str] =
    &["expires_at", "last_used_at", "created_at", "updated_at"];
const USER_SESSION_TIMESTAMPS: &[&str] = &[
    "rotated_at",
    "last_seen_at",
    "expires_at",
    "revoked_at",
    "created_at",
    "updated_at",
];
const CONFIG_TIMESTAMPS: &[&str] = &["created_at", "updated_at"];
const GEMINI_FILE_TIMESTAMPS: &[&str] = &["created_at", "expires_at"];
const PROVIDER_USAGE_TIMESTAMPS: &[&str] =
    &["window_start", "window_end", "created_at", "updated_at"];
const REQUEST_CANDIDATE_TIMESTAMPS: &[&str] = &["created_at", "started_at", "finished_at"];
const VIDEO_TASK_TIMESTAMPS: &[&str] = &[
    "next_poll_at",
    "created_at",
    "submitted_at",
    "completed_at",
    "updated_at",
];
const ROUTING_SNAPSHOT_TIMESTAMPS: &[&str] = &["created_at", "updated_at"];

const SQLITE_STATS_TABLES: &[StatsTableSpec] = &[
    stats_table("stats_hourly", STATS_HOURLY_ROLLUP_TIMESTAMPS),
    stats_table("stats_summary", STATS_SUMMARY_TIMESTAMPS),
    stats_table("stats_hourly_user", STATS_HOUR_TIMESTAMPS),
    stats_table("stats_hourly_user_model", STATS_HOUR_TIMESTAMPS),
    stats_table("user_model_usage_counts", &["created_at", "updated_at"]),
    stats_table("stats_hourly_model", STATS_HOUR_TIMESTAMPS),
    stats_table("stats_hourly_provider", STATS_HOUR_TIMESTAMPS),
    stats_table("stats_daily", STATS_ROLLUP_TIMESTAMPS),
    stats_table("stats_daily_model", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_provider", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_api_key", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_error", STATS_DATE_TIMESTAMPS),
    stats_table("stats_user_daily", STATS_DATE_TIMESTAMPS),
    stats_table("stats_user_summary", STATS_USER_SUMMARY_TIMESTAMPS),
    stats_table("stats_user_daily_model", STATS_DATE_TIMESTAMPS),
    stats_table("stats_user_daily_provider", STATS_DATE_TIMESTAMPS),
    stats_table("stats_user_daily_api_format", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_model_provider", STATS_DATE_TIMESTAMPS),
    stats_table("stats_user_daily_model_provider", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_cost_savings", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_cost_savings_provider", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_cost_savings_model", STATS_DATE_TIMESTAMPS),
    stats_table(
        "stats_daily_cost_savings_model_provider",
        STATS_DATE_TIMESTAMPS,
    ),
    stats_table("stats_user_daily_cost_savings", STATS_DATE_TIMESTAMPS),
    stats_table(
        "stats_user_daily_cost_savings_provider",
        STATS_DATE_TIMESTAMPS,
    ),
    stats_table("stats_user_daily_cost_savings_model", STATS_DATE_TIMESTAMPS),
    stats_table(
        "stats_user_daily_cost_savings_model_provider",
        STATS_DATE_TIMESTAMPS,
    ),
];

const MYSQL_STATS_TABLES: &[StatsTableSpec] = &[
    stats_table("stats_hourly", STATS_HOURLY_ROLLUP_TIMESTAMPS),
    stats_table("stats_summary", STATS_SUMMARY_TIMESTAMPS),
    stats_table("stats_hourly_user", STATS_HOUR_TIMESTAMPS),
    stats_table("stats_hourly_user_model", STATS_HOUR_TIMESTAMPS),
    stats_table("user_model_usage_counts", &["created_at", "updated_at"]),
    stats_table("stats_hourly_model", STATS_HOUR_TIMESTAMPS),
    stats_table("stats_hourly_provider", STATS_HOUR_TIMESTAMPS),
    stats_table("stats_daily", STATS_ROLLUP_TIMESTAMPS),
    stats_table("stats_daily_model", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_provider", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_api_key", STATS_DATE_TIMESTAMPS),
    stats_table("stats_daily_error", STATS_DATE_TIMESTAMPS),
    stats_table("stats_user_daily", STATS_DATE_TIMESTAMPS),
];

const AUXILIARY_TABLES: &[AuxiliaryTableSpec] = &[
    aux_table("audit_logs", ID_COLUMNS, AUDIT_TIMESTAMPS),
    aux_table("announcements", ID_COLUMNS, ANNOUNCEMENT_TIMESTAMPS),
    aux_table(
        "announcement_reads",
        ID_COLUMNS,
        ANNOUNCEMENT_READ_TIMESTAMPS,
    ),
    aux_table("management_tokens", ID_COLUMNS, MANAGEMENT_TOKEN_TIMESTAMPS),
    aux_table("user_preferences", ID_COLUMNS, CONFIG_TIMESTAMPS),
    aux_table("user_sessions", ID_COLUMNS, USER_SESSION_TIMESTAMPS),
    aux_table("ldap_configs", ID_COLUMNS, CONFIG_TIMESTAMPS),
    aux_table("api_key_provider_mappings", ID_COLUMNS, CONFIG_TIMESTAMPS),
    aux_table("gemini_file_mappings", ID_COLUMNS, GEMINI_FILE_TIMESTAMPS),
    aux_table(
        "provider_usage_tracking",
        ID_COLUMNS,
        PROVIDER_USAGE_TIMESTAMPS,
    ),
    aux_table("pool_member_scores", ID_COLUMNS, &[]),
    aux_table("proxy_node_events", ID_COLUMNS, AUDIT_TIMESTAMPS),
    aux_table(
        "request_candidates",
        ID_COLUMNS,
        REQUEST_CANDIDATE_TIMESTAMPS,
    ),
    aux_table("video_tasks", ID_COLUMNS, VIDEO_TASK_TIMESTAMPS),
    aux_table(
        "usage_routing_snapshots",
        REQUEST_ID_COLUMNS,
        ROUTING_SNAPSHOT_TIMESTAMPS,
    ),
    aux_table("background_task_runs", ID_COLUMNS, &[]),
    aux_table("background_task_events", ID_COLUMNS, &[]),
    aux_table("proxy_node_metrics_1m", PROXY_NODE_METRIC_COLUMNS, &[]),
    aux_table("proxy_node_metrics_1h", PROXY_NODE_METRIC_COLUMNS, &[]),
];

const fn stats_table(
    table_name: &'static str,
    timestamp_columns: &'static [&'static str],
) -> StatsTableSpec {
    StatsTableSpec {
        table_name,
        id_column: "id",
        timestamp_columns,
    }
}

const fn aux_table(
    table_name: &'static str,
    id_columns: &'static [&'static str],
    timestamp_columns: &'static [&'static str],
) -> AuxiliaryTableSpec {
    AuxiliaryTableSpec {
        table_name,
        id_columns,
        timestamp_columns,
    }
}

pub fn encode_jsonl(records: &[DataExportRecord]) -> Result<String, DataLayerError> {
    validate_export_records(records)?;

    let mut output = String::new();
    for record in records {
        let line = serde_json::to_string(record)
            .map_err(|err| DataLayerError::UnexpectedValue(err.to_string()))?;
        output.push_str(&line);
        output.push('\n');
    }
    Ok(output)
}

pub fn decode_jsonl(input: &str) -> Result<Vec<DataExportRecord>, DataLayerError> {
    let mut records = Vec::new();
    for (line_index, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<DataExportRecord>(line).map_err(|err| {
            DataLayerError::InvalidInput(format!(
                "invalid export JSONL record on line {}: {err}",
                line_index + 1
            ))
        })?;
        records.push(record);
    }
    validate_export_records(&records)?;
    Ok(records)
}

pub fn build_import_plan(input: &str) -> Result<DataImportPlan, DataLayerError> {
    let records = decode_jsonl(input)?;
    build_import_plan_from_records(records)
}

pub fn build_import_plan_from_records(
    records: Vec<DataExportRecord>,
) -> Result<DataImportPlan, DataLayerError> {
    validate_export_records(&records)?;
    let manifest = match records.first() {
        Some(DataExportRecord::Manifest { manifest }) => manifest.clone(),
        _ => unreachable!("validate_export_records validates the manifest record"),
    };
    let mut rows_by_domain = BTreeMap::<ExportDomain, Vec<ExportRow>>::new();
    for record in records.into_iter().skip(1) {
        let DataExportRecord::Row {
            domain,
            id,
            payload,
        } = record
        else {
            return Err(DataLayerError::InvalidInput(
                "export manifest must appear only as the first record".to_string(),
            ));
        };
        rows_by_domain
            .entry(domain)
            .or_default()
            .push(ExportRow { id, payload });
    }
    Ok(DataImportPlan {
        manifest,
        rows_by_domain,
    })
}

pub fn validate_export_records(records: &[DataExportRecord]) -> Result<(), DataLayerError> {
    let Some(DataExportRecord::Manifest { manifest }) = records.first() else {
        return Err(DataLayerError::InvalidInput(
            "export JSONL must start with a manifest record".to_string(),
        ));
    };
    if manifest.format_version != EXPORT_FORMAT_VERSION {
        return Err(DataLayerError::InvalidInput(format!(
            "unsupported export format version {}; expected {}",
            manifest.format_version, EXPORT_FORMAT_VERSION
        )));
    }

    let allowed_domains = manifest.domains.iter().copied().collect::<BTreeSet<_>>();
    let mut seen_ids = BTreeSet::<(ExportDomain, String)>::new();
    for (index, record) in records.iter().enumerate().skip(1) {
        match record {
            DataExportRecord::Manifest { .. } => {
                return Err(DataLayerError::InvalidInput(format!(
                    "export manifest appears more than once at record {}",
                    index + 1
                )));
            }
            DataExportRecord::Row {
                domain,
                id,
                payload: _,
            } => {
                if !allowed_domains.contains(domain) {
                    return Err(DataLayerError::InvalidInput(format!(
                        "record {} uses domain '{}' not declared in manifest",
                        index + 1,
                        domain.as_str()
                    )));
                }
                if id.trim().is_empty() {
                    return Err(DataLayerError::InvalidInput(format!(
                        "record {} has an empty id",
                        index + 1
                    )));
                }
                let key = (*domain, id.clone());
                if !seen_ids.insert(key) {
                    return Err(DataLayerError::InvalidInput(format!(
                        "duplicate '{}' export id '{}' at record {}",
                        domain.as_str(),
                        id,
                        index + 1
                    )));
                }
            }
        }
    }
    Ok(())
}

pub fn sqlite_core_export_domains() -> Vec<ExportDomain> {
    vec![
        ExportDomain::Users,
        ExportDomain::ApiKeys,
        ExportDomain::Providers,
        ExportDomain::ProviderKeys,
        ExportDomain::Endpoints,
        ExportDomain::GlobalModels,
        ExportDomain::Models,
        ExportDomain::AuthModules,
        ExportDomain::OAuthProviders,
        ExportDomain::UserOAuthLinks,
        ExportDomain::UserGroups,
        ExportDomain::UserGroupMembers,
        ExportDomain::ProxyNodes,
        ExportDomain::SystemConfigs,
        ExportDomain::Wallets,
        ExportDomain::Usage,
        ExportDomain::Billing,
        ExportDomain::Stats,
        ExportDomain::Auxiliary,
    ]
}

pub fn mysql_core_export_domains() -> Vec<ExportDomain> {
    sqlite_core_export_domains()
}

pub fn postgres_core_export_domains() -> Vec<ExportDomain> {
    sqlite_core_export_domains()
}

pub async fn export_database_records(
    database: SqlDatabaseConfig,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
) -> Result<Vec<DataExportRecord>, DataLayerError> {
    match database.driver {
        DatabaseDriver::Sqlite => {
            let pool = crate::driver::sqlite::SqlitePoolFactory::new(database)?.connect_lazy()?;
            if domains.is_empty() {
                export_sqlite_core_records(&pool, created_at_unix_secs).await
            } else {
                export_sqlite_records(&pool, domains, created_at_unix_secs).await
            }
        }
        DatabaseDriver::Mysql => {
            let pool = crate::driver::mysql::MysqlPoolFactory::new(database)?.connect_lazy()?;
            if domains.is_empty() {
                export_mysql_core_records(&pool, created_at_unix_secs).await
            } else {
                export_mysql_records(&pool, domains, created_at_unix_secs).await
            }
        }
        DatabaseDriver::Postgres => {
            let pool =
                crate::driver::postgres::PostgresPoolFactory::new(database.to_postgres_config()?)?
                    .connect_lazy()?;
            if domains.is_empty() {
                export_postgres_core_records(&pool, created_at_unix_secs).await
            } else {
                export_postgres_records(&pool, domains, created_at_unix_secs).await
            }
        }
    }
}

pub async fn export_database_jsonl(
    database: SqlDatabaseConfig,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
) -> Result<String, DataLayerError> {
    let records = export_database_records(database, domains, created_at_unix_secs).await?;
    encode_jsonl(&records)
}

pub async fn import_database_jsonl(
    database: SqlDatabaseConfig,
    input: &str,
) -> Result<usize, DataLayerError> {
    match database.driver {
        DatabaseDriver::Sqlite => {
            let pool = crate::driver::sqlite::SqlitePoolFactory::new(database)?.connect_lazy()?;
            import_sqlite_jsonl(&pool, input).await
        }
        DatabaseDriver::Mysql => {
            let pool = crate::driver::mysql::MysqlPoolFactory::new(database)?.connect_lazy()?;
            import_mysql_jsonl(&pool, input).await
        }
        DatabaseDriver::Postgres => {
            let pool =
                crate::driver::postgres::PostgresPoolFactory::new(database.to_postgres_config()?)?
                    .connect_lazy()?;
            import_postgres_jsonl(&pool, input).await
        }
    }
}

pub async fn import_database_plan(
    database: SqlDatabaseConfig,
    plan: &DataImportPlan,
) -> Result<usize, DataLayerError> {
    match database.driver {
        DatabaseDriver::Sqlite => {
            let pool = crate::driver::sqlite::SqlitePoolFactory::new(database)?.connect_lazy()?;
            import_sqlite_plan(&pool, plan).await
        }
        DatabaseDriver::Mysql => {
            let pool = crate::driver::mysql::MysqlPoolFactory::new(database)?.connect_lazy()?;
            import_mysql_plan(&pool, plan).await
        }
        DatabaseDriver::Postgres => {
            let pool =
                crate::driver::postgres::PostgresPoolFactory::new(database.to_postgres_config()?)?
                    .connect_lazy()?;
            import_postgres_plan(&pool, plan).await
        }
    }
}

pub async fn copy_database_records(
    source: SqlDatabaseConfig,
    target: SqlDatabaseConfig,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
    options: DataCopyOptions,
) -> Result<usize, DataLayerError> {
    let source_driver = source.driver;
    let target_driver = target.driver;
    let mut records =
        export_database_records(source.clone(), domains, created_at_unix_secs).await?;
    if options.omit_request_body_details {
        omit_request_body_details_from_records(&mut records);
    }
    let plan = build_import_plan_from_records(records)?;
    let mut imported = import_database_plan(target.clone(), &plan).await?;

    if !options.omit_request_body_details
        && source_driver == DatabaseDriver::Postgres
        && target_driver == DatabaseDriver::Sqlite
    {
        imported =
            imported.saturating_add(copy_postgres_sqlite_usage_body_details(source, target).await?);
    }

    Ok(imported)
}

fn omit_request_body_details_from_records(records: &mut [DataExportRecord]) {
    for record in records {
        let DataExportRecord::Row {
            domain: ExportDomain::Usage,
            payload,
            ..
        } = record
        else {
            continue;
        };

        if let Some(object) = payload.as_object_mut() {
            for column_name in USAGE_REQUEST_BODY_DETAIL_COLUMNS {
                object.remove(*column_name);
            }
        }
    }
}

async fn copy_postgres_sqlite_usage_body_details(
    source: SqlDatabaseConfig,
    target: SqlDatabaseConfig,
) -> Result<usize, DataLayerError> {
    let postgres_pool =
        crate::driver::postgres::PostgresPoolFactory::new(source.to_postgres_config()?)?
            .connect_lazy()?;
    let sqlite_pool = crate::driver::sqlite::SqlitePoolFactory::new(target)?.connect_lazy()?;

    ensure_sqlite_usage_body_detail_tables(&sqlite_pool).await?;

    let mut imported = 0usize;
    if postgres_table_exists(&postgres_pool, "public.usage_body_blobs").await? {
        imported = imported.saturating_add(
            copy_postgres_usage_body_blobs_to_sqlite(&postgres_pool, &sqlite_pool).await?,
        );
    }
    if postgres_table_exists(&postgres_pool, "public.usage_http_audits").await? {
        imported = imported.saturating_add(
            copy_postgres_usage_http_audits_to_sqlite(&postgres_pool, &sqlite_pool).await?,
        );
    }
    Ok(imported)
}

async fn postgres_table_exists(
    pool: &crate::driver::postgres::PostgresPool,
    table_name: &str,
) -> Result<bool, DataLayerError> {
    sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
        .bind(table_name)
        .fetch_one(pool)
        .await
        .map_sql_err()
}

async fn ensure_sqlite_usage_body_detail_tables(
    pool: &crate::driver::sqlite::SqlitePool,
) -> Result<(), DataLayerError> {
    sqlx::raw_sql(
        r#"
CREATE TABLE IF NOT EXISTS usage_body_blobs (
    body_ref TEXT PRIMARY KEY NOT NULL,
    request_id TEXT NOT NULL,
    body_field TEXT NOT NULL,
    payload_gzip BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (request_id, body_field),
    FOREIGN KEY (request_id) REFERENCES "usage"(request_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS usage_body_blobs_request_id_idx
    ON usage_body_blobs (request_id);

CREATE TABLE IF NOT EXISTS usage_http_audits (
    request_id TEXT PRIMARY KEY NOT NULL,
    request_headers TEXT,
    provider_request_headers TEXT,
    response_headers TEXT,
    client_response_headers TEXT,
    request_body_ref TEXT,
    provider_request_body_ref TEXT,
    response_body_ref TEXT,
    client_response_body_ref TEXT,
    request_body_state TEXT,
    provider_request_body_state TEXT,
    response_body_state TEXT,
    client_response_body_state TEXT,
    body_capture_mode TEXT NOT NULL DEFAULT 'none',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (request_id) REFERENCES "usage"(request_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS usage_http_audits_updated_at_idx
    ON usage_http_audits (updated_at);
"#,
    )
    .execute(pool)
    .await
    .map_sql_err()?;
    Ok(())
}

async fn copy_postgres_usage_body_blobs_to_sqlite(
    postgres_pool: &crate::driver::postgres::PostgresPool,
    sqlite_pool: &crate::driver::sqlite::SqlitePool,
) -> Result<usize, DataLayerError> {
    let rows = sqlx::query(
        r#"
SELECT
  body_ref,
  request_id,
  body_field,
  payload_gzip,
  created_at::text AS created_at,
  updated_at::text AS updated_at
FROM public.usage_body_blobs
ORDER BY body_ref ASC
"#,
    )
    .fetch_all(postgres_pool)
    .await
    .map_sql_err()?;

    let mut imported = 0usize;
    for row in rows {
        sqlx::query(
            r#"
INSERT OR REPLACE INTO usage_body_blobs (
  body_ref,
  request_id,
  body_field,
  payload_gzip,
  created_at,
  updated_at
) VALUES (?, ?, ?, ?, ?, ?)
"#,
        )
        .bind(row.try_get::<String, _>("body_ref").map_sql_err()?)
        .bind(row.try_get::<String, _>("request_id").map_sql_err()?)
        .bind(row.try_get::<String, _>("body_field").map_sql_err()?)
        .bind(row.try_get::<Vec<u8>, _>("payload_gzip").map_sql_err()?)
        .bind(row.try_get::<String, _>("created_at").map_sql_err()?)
        .bind(row.try_get::<String, _>("updated_at").map_sql_err()?)
        .execute(sqlite_pool)
        .await
        .map_sql_err()?;
        imported = imported.saturating_add(1);
    }
    Ok(imported)
}

async fn copy_postgres_usage_http_audits_to_sqlite(
    postgres_pool: &crate::driver::postgres::PostgresPool,
    sqlite_pool: &crate::driver::sqlite::SqlitePool,
) -> Result<usize, DataLayerError> {
    let rows = sqlx::query(
        r#"
SELECT
  request_id,
  request_headers::text AS request_headers,
  provider_request_headers::text AS provider_request_headers,
  response_headers::text AS response_headers,
  client_response_headers::text AS client_response_headers,
  request_body_ref,
  provider_request_body_ref,
  response_body_ref,
  client_response_body_ref,
  request_body_state,
  provider_request_body_state,
  response_body_state,
  client_response_body_state,
  body_capture_mode,
  created_at::text AS created_at,
  updated_at::text AS updated_at
FROM public.usage_http_audits
ORDER BY request_id ASC
"#,
    )
    .fetch_all(postgres_pool)
    .await
    .map_sql_err()?;

    let mut imported = 0usize;
    for row in rows {
        sqlx::query(
            r#"
INSERT OR REPLACE INTO usage_http_audits (
  request_id,
  request_headers,
  provider_request_headers,
  response_headers,
  client_response_headers,
  request_body_ref,
  provider_request_body_ref,
  response_body_ref,
  client_response_body_ref,
  request_body_state,
  provider_request_body_state,
  response_body_state,
  client_response_body_state,
  body_capture_mode,
  created_at,
  updated_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
"#,
        )
        .bind(row.try_get::<String, _>("request_id").map_sql_err()?)
        .bind(
            row.try_get::<Option<String>, _>("request_headers")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("provider_request_headers")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("response_headers")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("client_response_headers")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("request_body_ref")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("provider_request_body_ref")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("response_body_ref")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("client_response_body_ref")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("request_body_state")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("provider_request_body_state")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("response_body_state")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<Option<String>, _>("client_response_body_state")
                .map_sql_err()?,
        )
        .bind(
            row.try_get::<String, _>("body_capture_mode")
                .map_sql_err()?,
        )
        .bind(row.try_get::<String, _>("created_at").map_sql_err()?)
        .bind(row.try_get::<String, _>("updated_at").map_sql_err()?)
        .execute(sqlite_pool)
        .await
        .map_sql_err()?;
        imported = imported.saturating_add(1);
    }
    Ok(imported)
}

pub async fn export_sqlite_core_jsonl(
    pool: &crate::driver::sqlite::SqlitePool,
    created_at_unix_secs: u64,
) -> Result<String, DataLayerError> {
    let records = export_sqlite_core_records(pool, created_at_unix_secs).await?;
    encode_jsonl(&records)
}

pub async fn export_sqlite_jsonl(
    pool: &crate::driver::sqlite::SqlitePool,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
) -> Result<String, DataLayerError> {
    let records = export_sqlite_records(pool, domains, created_at_unix_secs).await?;
    encode_jsonl(&records)
}

pub async fn export_sqlite_core_records(
    pool: &crate::driver::sqlite::SqlitePool,
    created_at_unix_secs: u64,
) -> Result<Vec<DataExportRecord>, DataLayerError> {
    export_sqlite_records(pool, sqlite_core_export_domains(), created_at_unix_secs).await
}

pub async fn export_sqlite_records(
    pool: &crate::driver::sqlite::SqlitePool,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
) -> Result<Vec<DataExportRecord>, DataLayerError> {
    let manifest = DataExportManifest::new(
        created_at_unix_secs,
        Some(DatabaseDriver::Sqlite),
        domains.clone(),
    );
    let mut records = vec![DataExportRecord::manifest(manifest)];

    for domain in domains {
        if domain == ExportDomain::Billing {
            export_sqlite_billing_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Wallets {
            export_sqlite_wallet_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Stats {
            export_sqlite_stats_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Auxiliary {
            export_sqlite_auxiliary_records(pool, &mut records).await?;
            continue;
        }
        let (table_name, id_column) = sqlite_domain_table(domain)?;
        let order_by = export_order_by(domain, id_column);
        let sql = format!("SELECT * FROM {table_name} ORDER BY {order_by}");
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = sqlite_export_row_id(domain, &row, id_column)?;
            records.push(DataExportRecord::row(domain, id, sqlite_row_payload(&row)?));
        }
    }

    Ok(records)
}

pub async fn import_sqlite_jsonl(
    pool: &crate::driver::sqlite::SqlitePool,
    input: &str,
) -> Result<usize, DataLayerError> {
    let plan = build_import_plan(input)?;
    import_sqlite_plan(pool, &plan).await
}

pub async fn import_sqlite_plan(
    pool: &crate::driver::sqlite::SqlitePool,
    plan: &DataImportPlan,
) -> Result<usize, DataLayerError> {
    let mut imported = 0usize;
    let mut column_cache = BTreeMap::<String, ImportColumnNames>::new();
    for domain in &plan.manifest.domains {
        if *domain == ExportDomain::Billing {
            for row in plan.rows(*domain) {
                import_sqlite_billing_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Wallets {
            for row in plan.rows(*domain) {
                import_sqlite_wallet_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Stats {
            for row in plan.rows(*domain) {
                import_sqlite_stats_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Auxiliary {
            for row in plan.rows(*domain) {
                import_sqlite_auxiliary_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        let (table_name, _id_column) = sqlite_domain_table(*domain)?;
        let target_columns =
            sqlite_import_columns_cached(pool, &mut column_cache, table_name).await?;
        for row in plan.rows(*domain) {
            import_sqlite_row(pool, table_name, *domain, row, &target_columns).await?;
            imported = imported.saturating_add(1);
        }
    }
    Ok(imported)
}

pub async fn export_postgres_core_jsonl(
    pool: &crate::driver::postgres::PostgresPool,
    created_at_unix_secs: u64,
) -> Result<String, DataLayerError> {
    let records = export_postgres_core_records(pool, created_at_unix_secs).await?;
    encode_jsonl(&records)
}

pub async fn export_postgres_jsonl(
    pool: &crate::driver::postgres::PostgresPool,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
) -> Result<String, DataLayerError> {
    let records = export_postgres_records(pool, domains, created_at_unix_secs).await?;
    encode_jsonl(&records)
}

pub async fn export_postgres_core_records(
    pool: &crate::driver::postgres::PostgresPool,
    created_at_unix_secs: u64,
) -> Result<Vec<DataExportRecord>, DataLayerError> {
    export_postgres_records(pool, postgres_core_export_domains(), created_at_unix_secs).await
}

pub async fn export_postgres_records(
    pool: &crate::driver::postgres::PostgresPool,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
) -> Result<Vec<DataExportRecord>, DataLayerError> {
    let manifest = DataExportManifest::new(
        created_at_unix_secs,
        Some(DatabaseDriver::Postgres),
        domains.clone(),
    );
    let mut records = vec![DataExportRecord::manifest(manifest)];

    for domain in domains {
        if domain == ExportDomain::Billing {
            export_postgres_billing_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Wallets {
            export_postgres_wallet_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Stats {
            export_postgres_stats_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Auxiliary {
            export_postgres_auxiliary_records(pool, &mut records).await?;
            continue;
        }
        let (table_name, id_column) = postgres_domain_table(domain)?;
        let export_id_sql = postgres_export_id_sql(domain, id_column);
        let order_by = export_order_by(domain, id_column);
        let sql = format!(
            "SELECT {export_id_sql} AS export_id, to_jsonb(t) AS payload FROM {table_name} AS t ORDER BY {order_by}"
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row.try_get::<String, _>("export_id").map_sql_err()?;
            let payload = row.try_get::<Value, _>("payload").map_sql_err()?;
            records.push(DataExportRecord::row(domain, id, payload));
        }
    }

    Ok(records)
}

pub async fn import_postgres_jsonl(
    pool: &crate::driver::postgres::PostgresPool,
    input: &str,
) -> Result<usize, DataLayerError> {
    let plan = build_import_plan(input)?;
    import_postgres_plan(pool, &plan).await
}

pub async fn import_postgres_plan(
    pool: &crate::driver::postgres::PostgresPool,
    plan: &DataImportPlan,
) -> Result<usize, DataLayerError> {
    let mut imported = 0usize;
    let mut column_cache = BTreeMap::<String, PostgresImportColumns>::new();
    for domain in &plan.manifest.domains {
        if *domain == ExportDomain::Billing {
            for row in plan.rows(*domain) {
                import_postgres_billing_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Wallets {
            for row in plan.rows(*domain) {
                import_postgres_wallet_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Stats {
            for row in plan.rows(*domain) {
                import_postgres_stats_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Auxiliary {
            for row in plan.rows(*domain) {
                import_postgres_auxiliary_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        let (table_name, id_column) = postgres_domain_table(*domain)?;
        let conflict_columns = postgres_conflict_columns(*domain, id_column);
        let rows = plan.rows(*domain);
        if rows.is_empty() {
            continue;
        }
        let target_columns =
            postgres_import_columns_cached(pool, &mut column_cache, table_name).await?;
        for row in rows {
            import_postgres_row(
                pool,
                table_name,
                &conflict_columns,
                *domain,
                row,
                &target_columns,
            )
            .await?;
            imported = imported.saturating_add(1);
        }
    }
    Ok(imported)
}

pub async fn export_mysql_core_jsonl(
    pool: &crate::driver::mysql::MysqlPool,
    created_at_unix_secs: u64,
) -> Result<String, DataLayerError> {
    let records = export_mysql_core_records(pool, created_at_unix_secs).await?;
    encode_jsonl(&records)
}

pub async fn export_mysql_jsonl(
    pool: &crate::driver::mysql::MysqlPool,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
) -> Result<String, DataLayerError> {
    let records = export_mysql_records(pool, domains, created_at_unix_secs).await?;
    encode_jsonl(&records)
}

pub async fn export_mysql_core_records(
    pool: &crate::driver::mysql::MysqlPool,
    created_at_unix_secs: u64,
) -> Result<Vec<DataExportRecord>, DataLayerError> {
    export_mysql_records(pool, mysql_core_export_domains(), created_at_unix_secs).await
}

pub async fn export_mysql_records(
    pool: &crate::driver::mysql::MysqlPool,
    domains: Vec<ExportDomain>,
    created_at_unix_secs: u64,
) -> Result<Vec<DataExportRecord>, DataLayerError> {
    let manifest = DataExportManifest::new(
        created_at_unix_secs,
        Some(DatabaseDriver::Mysql),
        domains.clone(),
    );
    let mut records = vec![DataExportRecord::manifest(manifest)];

    for domain in domains {
        if domain == ExportDomain::Billing {
            export_mysql_billing_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Wallets {
            export_mysql_wallet_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Stats {
            export_mysql_stats_records(pool, &mut records).await?;
            continue;
        }
        if domain == ExportDomain::Auxiliary {
            export_mysql_auxiliary_records(pool, &mut records).await?;
            continue;
        }
        let (table_name, id_column) = mysql_domain_table(domain)?;
        let order_by = export_order_by(domain, id_column);
        let sql = format!("SELECT * FROM {table_name} ORDER BY {order_by}");
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = mysql_export_row_id(domain, &row, id_column)?;
            records.push(DataExportRecord::row(domain, id, mysql_row_payload(&row)?));
        }
    }

    Ok(records)
}

pub async fn import_mysql_jsonl(
    pool: &crate::driver::mysql::MysqlPool,
    input: &str,
) -> Result<usize, DataLayerError> {
    let plan = build_import_plan(input)?;
    import_mysql_plan(pool, &plan).await
}

pub async fn import_mysql_plan(
    pool: &crate::driver::mysql::MysqlPool,
    plan: &DataImportPlan,
) -> Result<usize, DataLayerError> {
    let mut imported = 0usize;
    let mut column_cache = BTreeMap::<String, ImportColumnNames>::new();
    for domain in &plan.manifest.domains {
        if *domain == ExportDomain::Billing {
            for row in plan.rows(*domain) {
                import_mysql_billing_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Wallets {
            for row in plan.rows(*domain) {
                import_mysql_wallet_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Stats {
            for row in plan.rows(*domain) {
                import_mysql_stats_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        if *domain == ExportDomain::Auxiliary {
            for row in plan.rows(*domain) {
                import_mysql_auxiliary_row(pool, row, &mut column_cache).await?;
                imported = imported.saturating_add(1);
            }
            continue;
        }
        let (table_name, _id_column) = mysql_domain_table(*domain)?;
        let target_columns =
            mysql_import_columns_cached(pool, &mut column_cache, table_name).await?;
        for row in plan.rows(*domain) {
            import_mysql_row(pool, table_name, *domain, row, &target_columns).await?;
            imported = imported.saturating_add(1);
        }
    }
    Ok(imported)
}

fn sqlite_domain_table(
    domain: ExportDomain,
) -> Result<(&'static str, &'static str), DataLayerError> {
    match domain {
        ExportDomain::Users => Ok(("users", "id")),
        ExportDomain::ApiKeys => Ok(("api_keys", "id")),
        ExportDomain::Providers => Ok(("providers", "id")),
        ExportDomain::ProviderKeys => Ok(("provider_api_keys", "id")),
        ExportDomain::Endpoints => Ok(("provider_endpoints", "id")),
        ExportDomain::Models => Ok(("models", "id")),
        ExportDomain::GlobalModels => Ok(("global_models", "id")),
        ExportDomain::AuthModules => Ok(("auth_modules", "id")),
        ExportDomain::OAuthProviders => Ok(("oauth_providers", "provider_type")),
        ExportDomain::UserOAuthLinks => Ok(("user_oauth_links", "id")),
        ExportDomain::UserGroups => Ok(("user_groups", "id")),
        ExportDomain::UserGroupMembers => Ok(("user_group_members", "group_id")),
        ExportDomain::ProxyNodes => Ok(("proxy_nodes", "id")),
        ExportDomain::SystemConfigs => Ok(("system_configs", "id")),
        ExportDomain::Wallets => Err(DataLayerError::InvalidInput(
            "sqlite wallet export uses multiple tables and must be handled as a domain".to_string(),
        )),
        ExportDomain::Usage => Ok((r#""usage""#, "request_id")),
        ExportDomain::Billing => Err(DataLayerError::InvalidInput(
            "sqlite billing export uses multiple tables and must be handled as a domain"
                .to_string(),
        )),
        ExportDomain::Stats => Err(DataLayerError::InvalidInput(
            "sqlite stats export uses multiple tables and must be handled as a domain".to_string(),
        )),
        ExportDomain::Auxiliary => Err(DataLayerError::InvalidInput(
            "sqlite auxiliary export uses multiple tables and must be handled as a domain"
                .to_string(),
        )),
    }
}

fn export_order_by(domain: ExportDomain, id_column: &str) -> String {
    if domain == ExportDomain::UserGroupMembers {
        "group_id ASC, user_id ASC".to_string()
    } else {
        format!("{id_column} ASC")
    }
}

fn sqlite_export_row_id(
    domain: ExportDomain,
    row: &sqlx::sqlite::SqliteRow,
    id_column: &str,
) -> Result<String, DataLayerError> {
    if domain == ExportDomain::UserGroupMembers {
        let group_id = sqlite_required_export_text(row, "group_id", domain)?;
        let user_id = sqlite_required_export_text(row, "user_id", domain)?;
        return Ok(format!("{group_id}:{user_id}"));
    }
    sqlite_required_export_text(row, id_column, domain)
}

fn sqlite_required_export_text(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
    domain: ExportDomain,
) -> Result<String, DataLayerError> {
    row.try_get::<Option<String>, _>(column)
        .map_sql_err()?
        .ok_or_else(|| {
            DataLayerError::UnexpectedValue(format!(
                "{} export row has null id column '{}'",
                domain.as_str(),
                column
            ))
        })
}

async fn export_sqlite_billing_records(
    pool: &crate::driver::sqlite::SqlitePool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for table_name in [
        "billing_rules",
        "dimension_collectors",
        "usage_settlement_snapshots",
    ] {
        let id_column = if table_name == "usage_settlement_snapshots" {
            "request_id"
        } else {
            "id"
        };
        let sql = format!("SELECT * FROM {table_name} ORDER BY {id_column} ASC");
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row
                .try_get::<Option<String>, _>(id_column)
                .map_sql_err()?
                .ok_or_else(|| {
                    DataLayerError::UnexpectedValue(format!(
                        "billing export row in table '{table_name}' has null id"
                    ))
                })?;
            records.push(DataExportRecord::row(
                ExportDomain::Billing,
                format!("{table_name}:{id}"),
                payload_with_table(sqlite_row_payload(&row)?, table_name)?,
            ));
        }
    }
    Ok(())
}

async fn export_sqlite_wallet_records(
    pool: &crate::driver::sqlite::SqlitePool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for (table_name, id_column) in sqlite_wallet_tables() {
        let sql = format!("SELECT * FROM {table_name} ORDER BY {id_column} ASC");
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row
                .try_get::<Option<String>, _>(id_column)
                .map_sql_err()?
                .ok_or_else(|| {
                    DataLayerError::UnexpectedValue(format!(
                        "wallet export row in table '{table_name}' has null id"
                    ))
                })?;
            records.push(DataExportRecord::row(
                ExportDomain::Wallets,
                format!("{table_name}:{id}"),
                payload_with_table(sqlite_row_payload(&row)?, table_name)?,
            ));
        }
    }
    Ok(())
}

async fn import_sqlite_row(
    pool: &crate::driver::sqlite::SqlitePool,
    table_name: &str,
    domain: ExportDomain,
    row: &ExportRow,
    target_columns: &ImportColumnNames,
) -> Result<(), DataLayerError> {
    let object = filter_import_payload("sqlite", table_name, domain, row, target_columns)?;

    let columns = object.keys().map(String::as_str).collect::<Vec<_>>();
    let column_sql = columns
        .iter()
        .map(|column| sqlite_quote_identifier(column))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let placeholder_sql = vec!["?"; columns.len()].join(", ");
    let sql =
        format!("INSERT OR REPLACE INTO {table_name} ({column_sql}) VALUES ({placeholder_sql})");
    let mut query = sqlx::query(&sql);
    for column in columns {
        let value = object
            .get(column)
            .expect("column name came from payload object keys");
        query = bind_sqlite_json_value(query, value)?;
    }
    query.execute(pool).await.map_sql_err()?;
    Ok(())
}

async fn import_sqlite_billing_row(
    pool: &crate::driver::sqlite::SqlitePool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, ImportColumnNames>,
) -> Result<(), DataLayerError> {
    let (table_name, payload) = billing_payload_table(row)?;
    let table_name = sqlite_billing_table_name(&table_name)?;
    let target_columns = sqlite_import_columns_cached(pool, column_cache, table_name).await?;
    import_sqlite_row(
        pool,
        table_name,
        ExportDomain::Billing,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn sqlite_billing_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    match table_name {
        "billing_rules" => Ok("billing_rules"),
        "dimension_collectors" => Ok("dimension_collectors"),
        "usage_settlement_snapshots" => Ok("usage_settlement_snapshots"),
        other => Err(DataLayerError::InvalidInput(format!(
            "unsupported sqlite billing export table '{other}'"
        ))),
    }
}

async fn import_sqlite_wallet_row(
    pool: &crate::driver::sqlite::SqlitePool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, ImportColumnNames>,
) -> Result<(), DataLayerError> {
    let (table_name, payload) = domain_payload_table(row, "wallet", Some("wallets"))?;
    let table_name = sqlite_wallet_table_name(&table_name)?;
    let target_columns = sqlite_import_columns_cached(pool, column_cache, table_name).await?;
    import_sqlite_row(
        pool,
        table_name,
        ExportDomain::Wallets,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn sqlite_wallet_tables() -> &'static [(&'static str, &'static str)] {
    &[
        ("wallets", "id"),
        ("wallet_transactions", "id"),
        ("wallet_daily_usage_ledgers", "id"),
        ("payment_orders", "id"),
        ("payment_callbacks", "id"),
        ("refund_requests", "id"),
        ("redeem_code_batches", "id"),
        ("redeem_codes", "id"),
    ]
}

fn sqlite_wallet_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    sqlite_wallet_tables()
        .iter()
        .find(|(candidate, _)| *candidate == table_name)
        .map(|(table, _)| *table)
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported sqlite wallet export table '{table_name}'"
            ))
        })
}

async fn export_sqlite_stats_records(
    pool: &crate::driver::sqlite::SqlitePool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for spec in SQLITE_STATS_TABLES {
        if !sqlite_table_exists(pool, spec.table_name).await? {
            continue;
        }
        let sql = format!(
            "SELECT * FROM {} ORDER BY {} ASC",
            spec.table_name, spec.id_column
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row
                .try_get::<Option<String>, _>(spec.id_column)
                .map_sql_err()?
                .ok_or_else(|| {
                    DataLayerError::UnexpectedValue(format!(
                        "stats export row in table '{}' has null id",
                        spec.table_name
                    ))
                })?;
            records.push(DataExportRecord::row(
                ExportDomain::Stats,
                format!("{}:{id}", spec.table_name),
                payload_with_table(sqlite_row_payload(&row)?, spec.table_name)?,
            ));
        }
    }
    Ok(())
}

async fn import_sqlite_stats_row(
    pool: &crate::driver::sqlite::SqlitePool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, ImportColumnNames>,
) -> Result<(), DataLayerError> {
    let (table_name, payload) = domain_payload_table(row, "stats", None)?;
    let table_name = sqlite_stats_table_name(&table_name)?;
    let target_columns = sqlite_import_columns_cached(pool, column_cache, table_name).await?;
    import_sqlite_row(
        pool,
        table_name,
        ExportDomain::Stats,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn sqlite_stats_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    SQLITE_STATS_TABLES
        .iter()
        .find(|spec| spec.table_name == table_name)
        .map(|spec| spec.table_name)
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported sqlite stats export table '{table_name}'"
            ))
        })
}

async fn export_sqlite_auxiliary_records(
    pool: &crate::driver::sqlite::SqlitePool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for spec in AUXILIARY_TABLES {
        if !sqlite_table_exists(pool, spec.table_name).await? {
            continue;
        }
        let export_id_sql = sqlite_export_id_sql(spec.id_columns)?;
        let order_by = spec
            .id_columns
            .iter()
            .map(|column| sqlite_quote_identifier(column))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");
        let sql = format!(
            "SELECT *, {export_id_sql} AS __export_id FROM {} ORDER BY {order_by}",
            spec.table_name
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row
                .try_get::<Option<String>, _>("__export_id")
                .map_sql_err()?
                .ok_or_else(|| {
                    DataLayerError::UnexpectedValue(format!(
                        "auxiliary export row in table '{}' has null id",
                        spec.table_name
                    ))
                })?;
            let payload = payload_without_export_id(sqlite_row_payload(&row)?)?;
            records.push(DataExportRecord::row(
                ExportDomain::Auxiliary,
                format!("{}:{id}", spec.table_name),
                payload_with_table(payload, spec.table_name)?,
            ));
        }
    }
    Ok(())
}

async fn import_sqlite_auxiliary_row(
    pool: &crate::driver::sqlite::SqlitePool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, ImportColumnNames>,
) -> Result<(), DataLayerError> {
    let (table_name, payload) = domain_payload_table(row, "auxiliary", None)?;
    let table_name = sqlite_auxiliary_table_name(&table_name)?;
    let target_columns = sqlite_import_columns_cached(pool, column_cache, table_name).await?;
    import_sqlite_row(
        pool,
        table_name,
        ExportDomain::Auxiliary,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn sqlite_auxiliary_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    AUXILIARY_TABLES
        .iter()
        .find(|spec| spec.table_name == table_name)
        .map(|spec| spec.table_name)
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported sqlite auxiliary export table '{table_name}'"
            ))
        })
}

fn sqlite_export_id_sql(id_columns: &[&str]) -> Result<String, DataLayerError> {
    id_columns
        .iter()
        .map(|column| {
            let quoted = sqlite_quote_identifier(column)?;
            Ok(format!("CAST({quoted} AS TEXT)"))
        })
        .collect::<Result<Vec<_>, DataLayerError>>()
        .map(|parts| parts.join(" || ':' || "))
}

async fn sqlite_table_exists(
    pool: &crate::driver::sqlite::SqlitePool,
    table_name: &str,
) -> Result<bool, DataLayerError> {
    sqlx::query_scalar::<_, bool>(
        r#"
SELECT EXISTS (
  SELECT 1
  FROM sqlite_master
  WHERE type = 'table'
    AND name = ?
)
"#,
    )
    .bind(table_name)
    .fetch_one(pool)
    .await
    .map_sql_err()
}

async fn sqlite_import_columns_cached(
    pool: &crate::driver::sqlite::SqlitePool,
    cache: &mut BTreeMap<String, ImportColumnNames>,
    table_name: &str,
) -> Result<ImportColumnNames, DataLayerError> {
    if let Some(columns) = cache.get(table_name) {
        return Ok(columns.clone());
    }

    let columns = load_sqlite_import_columns(pool, table_name).await?;
    cache.insert(table_name.to_string(), columns.clone());
    Ok(columns)
}

async fn load_sqlite_import_columns(
    pool: &crate::driver::sqlite::SqlitePool,
    table_name: &str,
) -> Result<ImportColumnNames, DataLayerError> {
    let sql = format!("PRAGMA table_info({table_name})");
    let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
    let mut columns = ImportColumnNames::new();
    for row in rows {
        columns.insert(row.try_get::<String, _>("name").map_sql_err()?);
    }

    if columns.is_empty() {
        return Err(DataLayerError::UnexpectedValue(format!(
            "sqlite import target table '{table_name}' has no visible columns"
        )));
    }

    Ok(columns)
}

fn sqlite_quote_identifier(identifier: &str) -> Result<String, DataLayerError> {
    if identifier.trim().is_empty() {
        return Err(DataLayerError::InvalidInput(
            "sqlite import column name cannot be empty".to_string(),
        ));
    }
    if !identifier
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(DataLayerError::InvalidInput(format!(
            "sqlite import column name '{identifier}' contains unsupported characters"
        )));
    }
    Ok(format!(r#""{identifier}""#))
}

fn bind_sqlite_json_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    value: &'q Value,
) -> Result<sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>, DataLayerError>
{
    Ok(match value {
        Value::Null => query.bind(Option::<String>::None),
        Value::Bool(value) => query.bind(i64::from(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                query.bind(value)
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value).map_err(|_| {
                    DataLayerError::InvalidInput(format!(
                        "sqlite import integer value {value} exceeds i64"
                    ))
                })?;
                query.bind(value)
            } else if let Some(value) = value.as_f64() {
                query.bind(value)
            } else {
                return Err(DataLayerError::InvalidInput(
                    "sqlite import number is not representable".to_string(),
                ));
            }
        }
        Value::String(value) => query.bind(value),
        Value::Array(_) | Value::Object(_) => {
            let value = serde_json::to_string(value)
                .map_err(|err| DataLayerError::UnexpectedValue(err.to_string()))?;
            query.bind(value)
        }
    })
}

fn postgres_domain_table(
    domain: ExportDomain,
) -> Result<(&'static str, &'static str), DataLayerError> {
    match domain {
        ExportDomain::Users => Ok(("public.users", "id")),
        ExportDomain::ApiKeys => Ok(("public.api_keys", "id")),
        ExportDomain::Providers => Ok(("public.providers", "id")),
        ExportDomain::ProviderKeys => Ok(("public.provider_api_keys", "id")),
        ExportDomain::Endpoints => Ok(("public.provider_endpoints", "id")),
        ExportDomain::Models => Ok(("public.models", "id")),
        ExportDomain::GlobalModels => Ok(("public.global_models", "id")),
        ExportDomain::AuthModules => Ok(("public.auth_modules", "id")),
        ExportDomain::OAuthProviders => Ok(("public.oauth_providers", "provider_type")),
        ExportDomain::UserOAuthLinks => Ok(("public.user_oauth_links", "id")),
        ExportDomain::UserGroups => Ok(("public.user_groups", "id")),
        ExportDomain::UserGroupMembers => Ok(("public.user_group_members", "group_id")),
        ExportDomain::ProxyNodes => Ok(("public.proxy_nodes", "id")),
        ExportDomain::SystemConfigs => Ok(("public.system_configs", "id")),
        ExportDomain::Wallets => Err(DataLayerError::InvalidInput(
            "postgres wallet export uses multiple tables and must be handled as a domain"
                .to_string(),
        )),
        ExportDomain::Usage => Ok(("public.usage", "request_id")),
        ExportDomain::Billing => Err(DataLayerError::InvalidInput(
            "postgres billing export uses multiple tables and must be handled as a domain"
                .to_string(),
        )),
        ExportDomain::Stats => Err(DataLayerError::InvalidInput(
            "postgres stats export uses multiple tables and must be handled as a domain"
                .to_string(),
        )),
        ExportDomain::Auxiliary => Err(DataLayerError::InvalidInput(
            "postgres auxiliary export uses multiple tables and must be handled as a domain"
                .to_string(),
        )),
    }
}

fn postgres_export_id_sql(domain: ExportDomain, id_column: &str) -> String {
    if domain == ExportDomain::UserGroupMembers {
        "group_id::text || ':' || user_id::text".to_string()
    } else {
        format!("{id_column}::text")
    }
}

fn postgres_conflict_columns(domain: ExportDomain, id_column: &str) -> Vec<&str> {
    if domain == ExportDomain::UserGroupMembers {
        vec!["group_id", "user_id"]
    } else {
        vec![id_column]
    }
}

async fn postgres_import_columns_cached(
    pool: &crate::driver::postgres::PostgresPool,
    cache: &mut BTreeMap<String, PostgresImportColumns>,
    table_name: &str,
) -> Result<PostgresImportColumns, DataLayerError> {
    if let Some(columns) = cache.get(table_name) {
        return Ok(columns.clone());
    }

    let columns = load_postgres_import_columns(pool, table_name).await?;
    cache.insert(table_name.to_string(), columns.clone());
    Ok(columns)
}

async fn load_postgres_import_columns(
    pool: &crate::driver::postgres::PostgresPool,
    table_name: &str,
) -> Result<PostgresImportColumns, DataLayerError> {
    let (schema_name, relation_name) = postgres_table_parts(table_name)?;
    let rows = sqlx::query(
        r#"
SELECT column_name, data_type, udt_name, is_nullable, column_default IS NOT NULL AS has_default
FROM information_schema.columns
WHERE table_schema = $1
  AND table_name = $2
"#,
    )
    .bind(schema_name)
    .bind(relation_name)
    .fetch_all(pool)
    .await
    .map_sql_err()?;

    let mut columns = PostgresImportColumns::new();
    for row in rows {
        let column_name = row.try_get::<String, _>("column_name").map_sql_err()?;
        let data_type = row
            .try_get::<String, _>("data_type")
            .map_sql_err()?
            .to_ascii_lowercase();
        let udt_name = row
            .try_get::<String, _>("udt_name")
            .map_sql_err()?
            .to_ascii_lowercase();
        let is_nullable = row.try_get::<String, _>("is_nullable").map_sql_err()? == "YES";
        let has_default = row.try_get::<bool, _>("has_default").map_sql_err()?;
        columns.insert(
            column_name,
            PostgresImportColumn {
                data_type,
                udt_name,
                is_nullable,
                has_default,
            },
        );
    }

    if columns.is_empty() {
        return Err(DataLayerError::UnexpectedValue(format!(
            "postgres import target table '{table_name}' has no visible columns"
        )));
    }

    Ok(columns)
}

fn postgres_table_parts(table_name: &str) -> Result<(&str, &str), DataLayerError> {
    let Some((schema_name, relation_name)) = table_name.split_once('.') else {
        return Err(DataLayerError::InvalidInput(format!(
            "postgres import target table '{table_name}' must include a schema"
        )));
    };
    Ok((
        schema_name.trim_matches('"'),
        relation_name.trim_matches('"'),
    ))
}

async fn export_postgres_billing_records(
    pool: &crate::driver::postgres::PostgresPool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for (table_name, export_table, id_column) in [
        ("public.billing_rules", "billing_rules", "id"),
        ("public.dimension_collectors", "dimension_collectors", "id"),
        (
            "public.usage_settlement_snapshots",
            "usage_settlement_snapshots",
            "request_id",
        ),
    ] {
        let sql = format!(
            "SELECT {id_column}::text AS export_id, to_jsonb(t) || jsonb_build_object('__table', '{export_table}') AS payload FROM {table_name} AS t ORDER BY {id_column} ASC"
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row.try_get::<String, _>("export_id").map_sql_err()?;
            let payload = row.try_get::<Value, _>("payload").map_sql_err()?;
            records.push(DataExportRecord::row(
                ExportDomain::Billing,
                format!("{export_table}:{id}"),
                payload,
            ));
        }
    }
    Ok(())
}

async fn export_postgres_wallet_records(
    pool: &crate::driver::postgres::PostgresPool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for (table_name, export_table, id_column) in postgres_wallet_tables() {
        let sql = format!(
            "SELECT {id_column}::text AS export_id, to_jsonb(t) || jsonb_build_object('__table', '{export_table}') AS payload FROM {table_name} AS t ORDER BY {id_column} ASC"
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row.try_get::<String, _>("export_id").map_sql_err()?;
            let payload = row.try_get::<Value, _>("payload").map_sql_err()?;
            records.push(DataExportRecord::row(
                ExportDomain::Wallets,
                format!("{export_table}:{id}"),
                payload,
            ));
        }
    }
    Ok(())
}

async fn import_postgres_row(
    pool: &crate::driver::postgres::PostgresPool,
    table_name: &str,
    conflict_columns: &[&str],
    domain: ExportDomain,
    row: &ExportRow,
    target_columns: &PostgresImportColumns,
) -> Result<(), DataLayerError> {
    let object = normalize_postgres_import_payload(table_name, domain, row, target_columns)?;

    let columns = object.keys().map(String::as_str).collect::<Vec<_>>();
    let column_sql = columns
        .iter()
        .map(|column| postgres_quote_identifier(column))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let update_sql = columns
        .iter()
        .filter(|column| !conflict_columns.contains(column))
        .map(|column| {
            let quoted = postgres_quote_identifier(column)?;
            Ok(format!("{quoted} = EXCLUDED.{quoted}"))
        })
        .collect::<Result<Vec<_>, DataLayerError>>()?
        .join(", ");
    let conflict_target_sql = conflict_columns
        .iter()
        .map(|column| postgres_quote_identifier(column))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let conflict_sql = if update_sql.is_empty() {
        format!("ON CONFLICT ({conflict_target_sql}) DO NOTHING")
    } else {
        format!("ON CONFLICT ({conflict_target_sql}) DO UPDATE SET {update_sql}")
    };
    let sql = format!(
        "INSERT INTO {table_name} ({column_sql}) SELECT {column_sql} FROM jsonb_populate_record(NULL::{table_name}, $1::jsonb) {conflict_sql}"
    );
    let payload = Value::Object(object);

    sqlx::query(&sql)
        .bind(&payload)
        .execute(pool)
        .await
        .map_sql_err()?;
    Ok(())
}

fn normalize_postgres_import_payload(
    table_name: &str,
    domain: ExportDomain,
    row: &ExportRow,
    target_columns: &PostgresImportColumns,
) -> Result<serde_json::Map<String, Value>, DataLayerError> {
    let object = row.payload.as_object().ok_or_else(|| {
        DataLayerError::InvalidInput(format!(
            "{} export row '{}' payload must be a JSON object",
            domain.as_str(),
            row.id
        ))
    })?;
    if object.is_empty() {
        return Err(DataLayerError::InvalidInput(format!(
            "{} export row '{}' payload cannot be empty",
            domain.as_str(),
            row.id
        )));
    }

    let mut normalized = serde_json::Map::new();
    for (column_name, value) in object {
        if let Some(target_column) = target_columns.get(column_name) {
            if value.is_null() && !target_column.is_nullable && target_column.has_default {
                continue;
            }
            normalized.insert(
                column_name.clone(),
                normalize_postgres_import_value(column_name, target_column, value)?,
            );
            continue;
        }
        if value.is_null() {
            continue;
        }
        return Err(DataLayerError::InvalidInput(format!(
            "{} export row '{}' contains column '{}' that does not exist in postgres table '{}'",
            domain.as_str(),
            row.id,
            column_name,
            table_name
        )));
    }

    if normalized.is_empty() {
        return Err(DataLayerError::InvalidInput(format!(
            "{} export row '{}' has no columns supported by postgres table '{}'",
            domain.as_str(),
            row.id,
            table_name
        )));
    }

    Ok(normalized)
}

fn normalize_postgres_import_value(
    column_name: &str,
    target_column: &PostgresImportColumn,
    value: &Value,
) -> Result<Value, DataLayerError> {
    if value.is_null() {
        return Ok(Value::Null);
    }

    if is_postgres_boolean_column(target_column) {
        return normalize_postgres_boolean_value(column_name, value);
    }
    if is_postgres_timestamp_column(target_column) {
        return normalize_postgres_timestamp_value(column_name, value);
    }
    if is_postgres_json_column(target_column) {
        return normalize_postgres_json_value(value);
    }

    Ok(value.clone())
}

fn is_postgres_boolean_column(target_column: &PostgresImportColumn) -> bool {
    target_column.data_type == "boolean" || target_column.udt_name == "bool"
}

fn is_postgres_timestamp_column(target_column: &PostgresImportColumn) -> bool {
    matches!(
        target_column.data_type.as_str(),
        "timestamp with time zone" | "timestamp without time zone"
    ) || matches!(target_column.udt_name.as_str(), "timestamptz" | "timestamp")
}

fn is_postgres_json_column(target_column: &PostgresImportColumn) -> bool {
    matches!(target_column.data_type.as_str(), "json" | "jsonb")
        || matches!(target_column.udt_name.as_str(), "json" | "jsonb")
}

fn normalize_postgres_boolean_value(
    column_name: &str,
    value: &Value,
) -> Result<Value, DataLayerError> {
    match value {
        Value::Bool(_) => Ok(value.clone()),
        Value::Number(number) => {
            let Some(value) = number
                .as_i64()
                .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            else {
                return Err(DataLayerError::InvalidInput(format!(
                    "postgres boolean import column '{column_name}' has non-integer value {number}"
                )));
            };
            match value {
                0 => Ok(Value::Bool(false)),
                1 => Ok(Value::Bool(true)),
                other => Err(DataLayerError::InvalidInput(format!(
                    "postgres boolean import column '{column_name}' has unsupported integer value {other}"
                ))),
            }
        }
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "0" | "false" => Ok(Value::Bool(false)),
            "1" | "true" => Ok(Value::Bool(true)),
            _ => Ok(Value::String(value.clone())),
        },
        _ => Ok(value.clone()),
    }
}

fn normalize_postgres_timestamp_value(
    column_name: &str,
    value: &Value,
) -> Result<Value, DataLayerError> {
    let Value::Number(number) = value else {
        return Ok(value.clone());
    };
    let Some(timestamp) = number
        .as_i64()
        .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
    else {
        return Err(DataLayerError::InvalidInput(format!(
            "postgres timestamp import column '{column_name}' has non-integer value {number}"
        )));
    };

    let datetime = if column_name.ends_with("_unix_ms")
        || timestamp >= 100_000_000_000
        || timestamp <= -100_000_000_000
    {
        chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp)
    } else {
        chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
    }
    .ok_or_else(|| {
        DataLayerError::InvalidInput(format!(
            "postgres timestamp import column '{column_name}' has out-of-range unix value {timestamp}"
        ))
    })?;

    Ok(Value::String(datetime.to_rfc3339()))
}

fn normalize_postgres_json_value(value: &Value) -> Result<Value, DataLayerError> {
    let Value::String(raw) = value else {
        return Ok(value.clone());
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(value.clone());
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(parsed) => Ok(parsed),
        Err(_) => Ok(value.clone()),
    }
}

async fn import_postgres_billing_row(
    pool: &crate::driver::postgres::PostgresPool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, PostgresImportColumns>,
) -> Result<(), DataLayerError> {
    let (export_table_name, payload) = billing_payload_table(row)?;
    let table_name = postgres_billing_table_name(&export_table_name)?;
    let target_columns = postgres_import_columns_cached(pool, column_cache, table_name).await?;
    import_postgres_row(
        pool,
        table_name,
        &["id"],
        ExportDomain::Billing,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn postgres_billing_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    match table_name {
        "billing_rules" => Ok("public.billing_rules"),
        "dimension_collectors" => Ok("public.dimension_collectors"),
        "usage_settlement_snapshots" => Ok("public.usage_settlement_snapshots"),
        other => Err(DataLayerError::InvalidInput(format!(
            "unsupported postgres billing export table '{other}'"
        ))),
    }
}

async fn import_postgres_wallet_row(
    pool: &crate::driver::postgres::PostgresPool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, PostgresImportColumns>,
) -> Result<(), DataLayerError> {
    let (export_table_name, payload) = domain_payload_table(row, "wallet", Some("wallets"))?;
    let (table_name, id_column) = postgres_wallet_table_name(&export_table_name)?;
    let target_columns = postgres_import_columns_cached(pool, column_cache, table_name).await?;
    import_postgres_row(
        pool,
        table_name,
        &[id_column],
        ExportDomain::Wallets,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn postgres_wallet_tables() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        ("public.wallets", "wallets", "id"),
        ("public.wallet_transactions", "wallet_transactions", "id"),
        (
            "public.wallet_daily_usage_ledgers",
            "wallet_daily_usage_ledgers",
            "id",
        ),
        ("public.payment_orders", "payment_orders", "id"),
        ("public.payment_callbacks", "payment_callbacks", "id"),
        ("public.refund_requests", "refund_requests", "id"),
        ("public.redeem_code_batches", "redeem_code_batches", "id"),
        ("public.redeem_codes", "redeem_codes", "id"),
    ]
}

fn postgres_wallet_table_name(
    table_name: &str,
) -> Result<(&'static str, &'static str), DataLayerError> {
    postgres_wallet_tables()
        .iter()
        .find(|(_, export_table, _)| *export_table == table_name)
        .map(|(table, _, id_column)| (*table, *id_column))
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported postgres wallet export table '{table_name}'"
            ))
        })
}

async fn export_postgres_stats_records(
    pool: &crate::driver::postgres::PostgresPool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for spec in SQLITE_STATS_TABLES {
        let table_name = format!("public.{}", spec.table_name);
        if !postgres_table_exists(pool, &table_name).await? {
            continue;
        }
        let payload_sql = postgres_stats_payload_sql(spec);
        let sql = format!(
            "SELECT {}::text AS export_id, {payload_sql} AS payload FROM {table_name} AS t ORDER BY {} ASC",
            spec.id_column, spec.id_column
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row.try_get::<String, _>("export_id").map_sql_err()?;
            let payload = row.try_get::<Value, _>("payload").map_sql_err()?;
            records.push(DataExportRecord::row(
                ExportDomain::Stats,
                format!("{}:{id}", spec.table_name),
                payload,
            ));
        }
    }
    Ok(())
}

fn postgres_stats_payload_sql(spec: &StatsTableSpec) -> String {
    postgres_multi_table_payload_sql(spec.table_name, spec.timestamp_columns)
}

fn postgres_auxiliary_payload_sql(spec: &AuxiliaryTableSpec) -> String {
    postgres_multi_table_payload_sql(spec.table_name, spec.timestamp_columns)
}

fn postgres_multi_table_payload_sql(table_name: &str, timestamp_columns: &[&str]) -> String {
    let mut fields = vec![format!("'__table'"), format!("'{table_name}'")];
    for column in timestamp_columns {
        fields.push(format!("'{column}'"));
        fields.push(format!(
            "CASE WHEN t.{column} IS NULL THEN NULL ELSE FLOOR(EXTRACT(EPOCH FROM t.{column}))::bigint END"
        ));
    }
    format!("to_jsonb(t) || jsonb_build_object({})", fields.join(", "))
}

async fn export_postgres_auxiliary_records(
    pool: &crate::driver::postgres::PostgresPool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for spec in AUXILIARY_TABLES {
        let table_name = format!("public.{}", spec.table_name);
        if !postgres_table_exists(pool, &table_name).await? {
            continue;
        }
        let export_id_sql = postgres_export_id_expr_sql(spec.id_columns)?;
        let order_by = postgres_export_order_sql(spec.id_columns)?;
        let payload_sql = postgres_auxiliary_payload_sql(spec);
        let sql = format!(
            "SELECT {export_id_sql} AS export_id, {payload_sql} AS payload FROM {table_name} AS t ORDER BY {order_by}"
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row.try_get::<String, _>("export_id").map_sql_err()?;
            let payload = row.try_get::<Value, _>("payload").map_sql_err()?;
            records.push(DataExportRecord::row(
                ExportDomain::Auxiliary,
                format!("{}:{id}", spec.table_name),
                payload,
            ));
        }
    }
    Ok(())
}

fn postgres_export_id_expr_sql(id_columns: &[&str]) -> Result<String, DataLayerError> {
    id_columns
        .iter()
        .map(|column| {
            let quoted = postgres_quote_identifier(column)?;
            Ok(format!("t.{quoted}::text"))
        })
        .collect::<Result<Vec<_>, DataLayerError>>()
        .map(|parts| parts.join(" || ':' || "))
}

fn postgres_export_order_sql(id_columns: &[&str]) -> Result<String, DataLayerError> {
    id_columns
        .iter()
        .map(|column| postgres_quote_identifier(column).map(|quoted| format!("t.{quoted} ASC")))
        .collect::<Result<Vec<_>, DataLayerError>>()
        .map(|parts| parts.join(", "))
}

async fn import_postgres_stats_row(
    pool: &crate::driver::postgres::PostgresPool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, PostgresImportColumns>,
) -> Result<(), DataLayerError> {
    let (export_table_name, payload) = domain_payload_table(row, "stats", None)?;
    let (table_name, id_column) = postgres_stats_table_name(&export_table_name)?;
    let target_columns = postgres_import_columns_cached(pool, column_cache, table_name).await?;
    import_postgres_row(
        pool,
        table_name,
        &[id_column],
        ExportDomain::Stats,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn postgres_stats_table_name(
    table_name: &str,
) -> Result<(&'static str, &'static str), DataLayerError> {
    SQLITE_STATS_TABLES
        .iter()
        .find(|spec| spec.table_name == table_name)
        .map(|spec| {
            let table_name = match spec.table_name {
                "stats_hourly" => "public.stats_hourly",
                "stats_summary" => "public.stats_summary",
                "stats_hourly_user" => "public.stats_hourly_user",
                "stats_hourly_user_model" => "public.stats_hourly_user_model",
                "user_model_usage_counts" => "public.user_model_usage_counts",
                "stats_hourly_model" => "public.stats_hourly_model",
                "stats_hourly_provider" => "public.stats_hourly_provider",
                "stats_daily" => "public.stats_daily",
                "stats_daily_model" => "public.stats_daily_model",
                "stats_daily_provider" => "public.stats_daily_provider",
                "stats_daily_api_key" => "public.stats_daily_api_key",
                "stats_daily_error" => "public.stats_daily_error",
                "stats_user_daily" => "public.stats_user_daily",
                "stats_user_summary" => "public.stats_user_summary",
                "stats_user_daily_model" => "public.stats_user_daily_model",
                "stats_user_daily_provider" => "public.stats_user_daily_provider",
                "stats_user_daily_api_format" => "public.stats_user_daily_api_format",
                "stats_daily_model_provider" => "public.stats_daily_model_provider",
                "stats_user_daily_model_provider" => "public.stats_user_daily_model_provider",
                "stats_daily_cost_savings" => "public.stats_daily_cost_savings",
                "stats_daily_cost_savings_provider" => "public.stats_daily_cost_savings_provider",
                "stats_daily_cost_savings_model" => "public.stats_daily_cost_savings_model",
                "stats_daily_cost_savings_model_provider" => {
                    "public.stats_daily_cost_savings_model_provider"
                }
                "stats_user_daily_cost_savings" => "public.stats_user_daily_cost_savings",
                "stats_user_daily_cost_savings_provider" => {
                    "public.stats_user_daily_cost_savings_provider"
                }
                "stats_user_daily_cost_savings_model" => {
                    "public.stats_user_daily_cost_savings_model"
                }
                "stats_user_daily_cost_savings_model_provider" => {
                    "public.stats_user_daily_cost_savings_model_provider"
                }
                _ => unreachable!("stats table list and postgres mapping must stay in sync"),
            };
            (table_name, spec.id_column)
        })
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported postgres stats export table '{table_name}'"
            ))
        })
}

async fn import_postgres_auxiliary_row(
    pool: &crate::driver::postgres::PostgresPool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, PostgresImportColumns>,
) -> Result<(), DataLayerError> {
    let (export_table_name, payload) = domain_payload_table(row, "auxiliary", None)?;
    let spec = auxiliary_table_spec(&export_table_name)?;
    let table_name = format!("public.{}", spec.table_name);
    let target_columns = postgres_import_columns_cached(pool, column_cache, &table_name).await?;
    import_postgres_row(
        pool,
        &table_name,
        spec.id_columns,
        ExportDomain::Auxiliary,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn auxiliary_table_spec(table_name: &str) -> Result<&'static AuxiliaryTableSpec, DataLayerError> {
    AUXILIARY_TABLES
        .iter()
        .find(|spec| spec.table_name == table_name)
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported auxiliary export table '{table_name}'"
            ))
        })
}

fn postgres_quote_identifier(identifier: &str) -> Result<String, DataLayerError> {
    if identifier.trim().is_empty() {
        return Err(DataLayerError::InvalidInput(
            "postgres import column name cannot be empty".to_string(),
        ));
    }
    if !identifier
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(DataLayerError::InvalidInput(format!(
            "postgres import column name '{identifier}' contains unsupported characters"
        )));
    }
    Ok(format!(r#""{identifier}""#))
}

fn mysql_domain_table(
    domain: ExportDomain,
) -> Result<(&'static str, &'static str), DataLayerError> {
    match domain {
        ExportDomain::Users => Ok(("users", "id")),
        ExportDomain::ApiKeys => Ok(("api_keys", "id")),
        ExportDomain::Providers => Ok(("providers", "id")),
        ExportDomain::ProviderKeys => Ok(("provider_api_keys", "id")),
        ExportDomain::Endpoints => Ok(("provider_endpoints", "id")),
        ExportDomain::Models => Ok(("models", "id")),
        ExportDomain::GlobalModels => Ok(("global_models", "id")),
        ExportDomain::AuthModules => Ok(("auth_modules", "id")),
        ExportDomain::OAuthProviders => Ok(("oauth_providers", "provider_type")),
        ExportDomain::UserOAuthLinks => Ok(("user_oauth_links", "id")),
        ExportDomain::UserGroups => Ok(("user_groups", "id")),
        ExportDomain::UserGroupMembers => Ok(("user_group_members", "group_id")),
        ExportDomain::ProxyNodes => Ok(("proxy_nodes", "id")),
        ExportDomain::SystemConfigs => Ok(("system_configs", "id")),
        ExportDomain::Wallets => Err(DataLayerError::InvalidInput(
            "mysql wallet export uses multiple tables and must be handled as a domain".to_string(),
        )),
        ExportDomain::Usage => Ok(("`usage`", "request_id")),
        ExportDomain::Billing => Err(DataLayerError::InvalidInput(
            "mysql billing export uses multiple tables and must be handled as a domain".to_string(),
        )),
        ExportDomain::Stats => Err(DataLayerError::InvalidInput(
            "mysql stats export uses multiple tables and must be handled as a domain".to_string(),
        )),
        ExportDomain::Auxiliary => Err(DataLayerError::InvalidInput(
            "mysql auxiliary export uses multiple tables and must be handled as a domain"
                .to_string(),
        )),
    }
}

fn mysql_export_row_id(
    domain: ExportDomain,
    row: &sqlx::mysql::MySqlRow,
    id_column: &str,
) -> Result<String, DataLayerError> {
    if domain == ExportDomain::UserGroupMembers {
        let group_id = mysql_required_export_text(row, "group_id", domain)?;
        let user_id = mysql_required_export_text(row, "user_id", domain)?;
        return Ok(format!("{group_id}:{user_id}"));
    }
    mysql_required_export_text(row, id_column, domain)
}

fn mysql_required_export_text(
    row: &sqlx::mysql::MySqlRow,
    column: &str,
    domain: ExportDomain,
) -> Result<String, DataLayerError> {
    row.try_get::<Option<String>, _>(column)
        .map_sql_err()?
        .ok_or_else(|| {
            DataLayerError::UnexpectedValue(format!(
                "{} export row has null id column '{}'",
                domain.as_str(),
                column
            ))
        })
}

async fn export_mysql_billing_records(
    pool: &crate::driver::mysql::MysqlPool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for (table_name, id_column) in [
        ("billing_rules", "id"),
        ("dimension_collectors", "id"),
        ("usage_settlement_snapshots", "request_id"),
    ] {
        let sql = format!("SELECT * FROM {table_name} ORDER BY {id_column} ASC");
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row
                .try_get::<Option<String>, _>(id_column)
                .map_sql_err()?
                .ok_or_else(|| {
                    DataLayerError::UnexpectedValue(format!(
                        "billing export row in table '{table_name}' has null id"
                    ))
                })?;
            records.push(DataExportRecord::row(
                ExportDomain::Billing,
                format!("{table_name}:{id}"),
                payload_with_table(mysql_row_payload(&row)?, table_name)?,
            ));
        }
    }
    Ok(())
}

async fn export_mysql_wallet_records(
    pool: &crate::driver::mysql::MysqlPool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for (table_name, id_column) in mysql_wallet_tables() {
        let sql = format!("SELECT * FROM {table_name} ORDER BY {id_column} ASC");
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row
                .try_get::<Option<String>, _>(id_column)
                .map_sql_err()?
                .ok_or_else(|| {
                    DataLayerError::UnexpectedValue(format!(
                        "wallet export row in table '{table_name}' has null id"
                    ))
                })?;
            records.push(DataExportRecord::row(
                ExportDomain::Wallets,
                format!("{table_name}:{id}"),
                payload_with_table(mysql_row_payload(&row)?, table_name)?,
            ));
        }
    }
    Ok(())
}

async fn export_mysql_stats_records(
    pool: &crate::driver::mysql::MysqlPool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for spec in MYSQL_STATS_TABLES {
        if !mysql_table_exists(pool, spec.table_name).await? {
            continue;
        }
        let export_id_sql = mysql_export_id_sql(&[spec.id_column])?;
        let order_by = mysql_quote_identifier(spec.id_column)?;
        let sql = format!(
            "SELECT *, {export_id_sql} AS `__export_id` FROM {} ORDER BY {order_by} ASC",
            spec.table_name
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row.try_get::<String, _>("__export_id").map_sql_err()?;
            let payload = payload_without_export_id(mysql_row_payload(&row)?)?;
            records.push(DataExportRecord::row(
                ExportDomain::Stats,
                format!("{}:{id}", spec.table_name),
                payload_with_table(payload, spec.table_name)?,
            ));
        }
    }
    Ok(())
}

async fn export_mysql_auxiliary_records(
    pool: &crate::driver::mysql::MysqlPool,
    records: &mut Vec<DataExportRecord>,
) -> Result<(), DataLayerError> {
    for spec in AUXILIARY_TABLES {
        if !mysql_table_exists(pool, spec.table_name).await? {
            continue;
        }
        let export_id_sql = mysql_export_id_sql(spec.id_columns)?;
        let order_by = spec
            .id_columns
            .iter()
            .map(|column| mysql_quote_identifier(column))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");
        let sql = format!(
            "SELECT *, {export_id_sql} AS `__export_id` FROM {} ORDER BY {order_by}",
            spec.table_name
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await.map_sql_err()?;
        for row in rows {
            let id = row.try_get::<String, _>("__export_id").map_sql_err()?;
            let payload = payload_without_export_id(mysql_row_payload(&row)?)?;
            records.push(DataExportRecord::row(
                ExportDomain::Auxiliary,
                format!("{}:{id}", spec.table_name),
                payload_with_table(payload, spec.table_name)?,
            ));
        }
    }
    Ok(())
}

fn mysql_export_id_sql(id_columns: &[&str]) -> Result<String, DataLayerError> {
    id_columns
        .iter()
        .map(|column| {
            let quoted = mysql_quote_identifier(column)?;
            Ok(format!("CAST({quoted} AS CHAR)"))
        })
        .collect::<Result<Vec<_>, DataLayerError>>()
        .map(|parts| parts.join(", ':', "))
        .map(|joined| {
            if id_columns.len() == 1 {
                joined
            } else {
                format!("CONCAT({joined})")
            }
        })
}

async fn import_mysql_row(
    pool: &crate::driver::mysql::MysqlPool,
    table_name: &str,
    domain: ExportDomain,
    row: &ExportRow,
    target_columns: &ImportColumnNames,
) -> Result<(), DataLayerError> {
    let object = filter_import_payload("mysql", table_name, domain, row, target_columns)?;

    let columns = object.keys().map(String::as_str).collect::<Vec<_>>();
    let column_sql = columns
        .iter()
        .map(|column| mysql_quote_identifier(column))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let placeholder_sql = vec!["?"; columns.len()].join(", ");
    let update_sql = columns
        .iter()
        .map(|column| {
            let quoted = mysql_quote_identifier(column)?;
            Ok(format!("{quoted} = VALUES({quoted})"))
        })
        .collect::<Result<Vec<_>, DataLayerError>>()?
        .join(", ");
    let sql = format!(
        "INSERT INTO {table_name} ({column_sql}) VALUES ({placeholder_sql}) ON DUPLICATE KEY UPDATE {update_sql}"
    );
    let mut query = sqlx::query(&sql);
    for column in columns {
        let value = object
            .get(column)
            .expect("column name came from payload object keys");
        query = bind_mysql_json_value(query, value)?;
    }
    query.execute(pool).await.map_sql_err()?;
    Ok(())
}

async fn import_mysql_billing_row(
    pool: &crate::driver::mysql::MysqlPool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, ImportColumnNames>,
) -> Result<(), DataLayerError> {
    let (table_name, payload) = billing_payload_table(row)?;
    let table_name = mysql_billing_table_name(&table_name)?;
    let target_columns = mysql_import_columns_cached(pool, column_cache, table_name).await?;
    import_mysql_row(
        pool,
        table_name,
        ExportDomain::Billing,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn mysql_billing_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    match table_name {
        "billing_rules" => Ok("billing_rules"),
        "dimension_collectors" => Ok("dimension_collectors"),
        "usage_settlement_snapshots" => Ok("usage_settlement_snapshots"),
        other => Err(DataLayerError::InvalidInput(format!(
            "unsupported mysql billing export table '{other}'"
        ))),
    }
}

async fn import_mysql_wallet_row(
    pool: &crate::driver::mysql::MysqlPool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, ImportColumnNames>,
) -> Result<(), DataLayerError> {
    let (table_name, payload) = domain_payload_table(row, "wallet", Some("wallets"))?;
    let table_name = mysql_wallet_table_name(&table_name)?;
    let target_columns = mysql_import_columns_cached(pool, column_cache, table_name).await?;
    import_mysql_row(
        pool,
        table_name,
        ExportDomain::Wallets,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn mysql_wallet_tables() -> &'static [(&'static str, &'static str)] {
    &[
        ("wallets", "id"),
        ("wallet_transactions", "id"),
        ("wallet_daily_usage_ledgers", "id"),
        ("payment_orders", "id"),
        ("payment_callbacks", "id"),
        ("refund_requests", "id"),
        ("redeem_code_batches", "id"),
        ("redeem_codes", "id"),
    ]
}

fn mysql_wallet_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    mysql_wallet_tables()
        .iter()
        .find(|(candidate, _)| *candidate == table_name)
        .map(|(table, _)| *table)
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported mysql wallet export table '{table_name}'"
            ))
        })
}

async fn import_mysql_stats_row(
    pool: &crate::driver::mysql::MysqlPool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, ImportColumnNames>,
) -> Result<(), DataLayerError> {
    let (table_name, payload) = domain_payload_table(row, "stats", None)?;
    let table_name = mysql_stats_table_name(&table_name)?;
    let target_columns = mysql_import_columns_cached(pool, column_cache, table_name).await?;
    import_mysql_row(
        pool,
        table_name,
        ExportDomain::Stats,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn mysql_stats_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    MYSQL_STATS_TABLES
        .iter()
        .find(|spec| spec.table_name == table_name)
        .map(|spec| spec.table_name)
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported mysql stats export table '{table_name}'"
            ))
        })
}

async fn import_mysql_auxiliary_row(
    pool: &crate::driver::mysql::MysqlPool,
    row: &ExportRow,
    column_cache: &mut BTreeMap<String, ImportColumnNames>,
) -> Result<(), DataLayerError> {
    let (table_name, payload) = domain_payload_table(row, "auxiliary", None)?;
    let table_name = mysql_auxiliary_table_name(&table_name)?;
    let target_columns = mysql_import_columns_cached(pool, column_cache, table_name).await?;
    import_mysql_row(
        pool,
        table_name,
        ExportDomain::Auxiliary,
        &ExportRow {
            id: row.id.clone(),
            payload,
        },
        &target_columns,
    )
    .await
}

fn mysql_auxiliary_table_name(table_name: &str) -> Result<&'static str, DataLayerError> {
    AUXILIARY_TABLES
        .iter()
        .find(|spec| spec.table_name == table_name)
        .map(|spec| spec.table_name)
        .ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "unsupported mysql auxiliary export table '{table_name}'"
            ))
        })
}

async fn mysql_table_exists(
    pool: &crate::driver::mysql::MysqlPool,
    table_name: &str,
) -> Result<bool, DataLayerError> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
SELECT COUNT(*)
FROM information_schema.tables
WHERE table_schema = DATABASE()
  AND table_name = ?
"#,
    )
    .bind(table_name)
    .fetch_one(pool)
    .await
    .map_sql_err()?;
    Ok(count > 0)
}

async fn mysql_import_columns_cached(
    pool: &crate::driver::mysql::MysqlPool,
    cache: &mut BTreeMap<String, ImportColumnNames>,
    table_name: &str,
) -> Result<ImportColumnNames, DataLayerError> {
    if let Some(columns) = cache.get(table_name) {
        return Ok(columns.clone());
    }

    let columns = load_mysql_import_columns(pool, table_name).await?;
    cache.insert(table_name.to_string(), columns.clone());
    Ok(columns)
}

async fn load_mysql_import_columns(
    pool: &crate::driver::mysql::MysqlPool,
    table_name: &str,
) -> Result<ImportColumnNames, DataLayerError> {
    let relation_name = table_name.trim_matches('`');
    let rows = sqlx::query(
        r#"
SELECT COLUMN_NAME AS column_name
FROM information_schema.columns
WHERE table_schema = DATABASE()
  AND table_name = ?
"#,
    )
    .bind(relation_name)
    .fetch_all(pool)
    .await
    .map_sql_err()?;

    let mut columns = ImportColumnNames::new();
    for row in rows {
        columns.insert(row.try_get::<String, _>("column_name").map_sql_err()?);
    }

    if columns.is_empty() {
        return Err(DataLayerError::UnexpectedValue(format!(
            "mysql import target table '{table_name}' has no visible columns"
        )));
    }

    Ok(columns)
}

fn mysql_quote_identifier(identifier: &str) -> Result<String, DataLayerError> {
    if identifier.trim().is_empty() {
        return Err(DataLayerError::InvalidInput(
            "mysql import column name cannot be empty".to_string(),
        ));
    }
    if !identifier
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(DataLayerError::InvalidInput(format!(
            "mysql import column name '{identifier}' contains unsupported characters"
        )));
    }
    Ok(format!("`{identifier}`"))
}

fn bind_mysql_json_value<'q>(
    query: sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>,
    value: &'q Value,
) -> Result<sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>, DataLayerError> {
    Ok(match value {
        Value::Null => query.bind(Option::<String>::None),
        Value::Bool(value) => query.bind(i64::from(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                query.bind(value)
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value).map_err(|_| {
                    DataLayerError::InvalidInput(format!(
                        "mysql import integer value {value} exceeds i64"
                    ))
                })?;
                query.bind(value)
            } else if let Some(value) = value.as_f64() {
                query.bind(value)
            } else {
                return Err(DataLayerError::InvalidInput(
                    "mysql import number is not representable".to_string(),
                ));
            }
        }
        Value::String(value) => query.bind(value),
        Value::Array(_) | Value::Object(_) => {
            let value = serde_json::to_string(value)
                .map_err(|err| DataLayerError::UnexpectedValue(err.to_string()))?;
            query.bind(value)
        }
    })
}

fn filter_import_payload(
    driver_name: &str,
    table_name: &str,
    domain: ExportDomain,
    row: &ExportRow,
    target_columns: &ImportColumnNames,
) -> Result<serde_json::Map<String, Value>, DataLayerError> {
    let object = row.payload.as_object().ok_or_else(|| {
        DataLayerError::InvalidInput(format!(
            "{} export row '{}' payload must be a JSON object",
            domain.as_str(),
            row.id
        ))
    })?;
    if object.is_empty() {
        return Err(DataLayerError::InvalidInput(format!(
            "{} export row '{}' payload cannot be empty",
            domain.as_str(),
            row.id
        )));
    }

    let mut filtered = serde_json::Map::new();
    for (column_name, value) in object {
        if target_columns.contains(column_name) {
            filtered.insert(column_name.clone(), value.clone());
        }
    }

    if filtered.is_empty() {
        return Err(DataLayerError::InvalidInput(format!(
            "{} export row '{}' has no columns supported by {driver_name} table '{table_name}'",
            domain.as_str(),
            row.id
        )));
    }

    Ok(filtered)
}

fn payload_with_table(payload: Value, table_name: &str) -> Result<Value, DataLayerError> {
    let mut object = payload.as_object().cloned().ok_or_else(|| {
        DataLayerError::UnexpectedValue("export row payload must be a JSON object".to_string())
    })?;
    normalize_billing_payload(table_name, &mut object)?;
    object.insert("__table".to_string(), Value::String(table_name.to_string()));
    Ok(Value::Object(object))
}

fn payload_without_export_id(payload: Value) -> Result<Value, DataLayerError> {
    let mut object = payload.as_object().cloned().ok_or_else(|| {
        DataLayerError::UnexpectedValue("export row payload must be a JSON object".to_string())
    })?;
    object.remove("__export_id");
    Ok(Value::Object(object))
}

fn normalize_billing_payload(
    table_name: &str,
    object: &mut serde_json::Map<String, Value>,
) -> Result<(), DataLayerError> {
    if table_name != "billing_rules" {
        return Ok(());
    }
    for field_name in ["variables", "dimension_mappings"] {
        let Some(Value::String(raw)) = object.get(field_name) else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        let parsed = serde_json::from_str::<Value>(raw).map_err(|err| {
            DataLayerError::UnexpectedValue(format!(
                "billing_rules.{field_name} contains invalid JSON: {err}"
            ))
        })?;
        object.insert(field_name.to_string(), parsed);
    }
    Ok(())
}

fn billing_payload_table(row: &ExportRow) -> Result<(String, Value), DataLayerError> {
    domain_payload_table(row, "billing", None)
}

fn domain_payload_table(
    row: &ExportRow,
    domain_label: &str,
    default_table: Option<&str>,
) -> Result<(String, Value), DataLayerError> {
    let mut object = row.payload.as_object().cloned().ok_or_else(|| {
        DataLayerError::InvalidInput(format!(
            "{domain_label} export row '{}' payload must be a JSON object",
            row.id,
        ))
    })?;
    let table_name = match object.remove("__table") {
        Some(value) => value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "{domain_label} export row '{}' has non-string __table",
                row.id
            ))
        })?,
        None => default_table.map(str::to_string).ok_or_else(|| {
            DataLayerError::InvalidInput(format!(
                "{domain_label} export row '{}' is missing string __table",
                row.id
            ))
        })?,
    };
    Ok((table_name, Value::Object(object)))
}

fn sqlite_row_payload(row: &sqlx::sqlite::SqliteRow) -> Result<Value, DataLayerError> {
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        object.insert(
            column.name().to_string(),
            sqlite_value_to_json(row, index, column.name())?,
        );
    }
    Ok(Value::Object(object))
}

fn sqlite_value_to_json(
    row: &sqlx::sqlite::SqliteRow,
    index: usize,
    column_name: &str,
) -> Result<Value, DataLayerError> {
    let raw = row.try_get_raw(index).map_sql_err()?;
    if raw.is_null() {
        return Ok(Value::Null);
    }

    match raw.type_info().name().to_ascii_uppercase().as_str() {
        "INTEGER" => {
            let value = row.try_get::<i64, _>(index).map_sql_err()?;
            if sqlite_integer_column_is_boolean(column_name) {
                match value {
                    0 => return Ok(Value::Bool(false)),
                    1 => return Ok(Value::Bool(true)),
                    _ => {}
                }
            }
            Ok(Value::from(value))
        }
        "REAL" | "FLOAT" | "DOUBLE" => {
            let value = row.try_get::<f64, _>(index).map_sql_err()?;
            serde_json::Number::from_f64(value)
                .map(Value::Number)
                .ok_or_else(|| {
                    DataLayerError::UnexpectedValue(format!(
                        "sqlite export column {} contains non-finite float",
                        index
                    ))
                })
        }
        "TEXT" => Ok(Value::String(
            row.try_get::<String, _>(index).map_sql_err()?,
        )),
        "BLOB" => {
            let bytes = row.try_get::<Vec<u8>, _>(index).map_sql_err()?;
            Ok(Value::Array(bytes.into_iter().map(Value::from).collect()))
        }
        other => Err(DataLayerError::UnexpectedValue(format!(
            "unsupported sqlite export column type '{other}' at index {index}"
        ))),
    }
}

fn sqlite_integer_column_is_boolean(column_name: &str) -> bool {
    column_name.starts_with("is_")
        || column_name.starts_with("has_")
        || column_name.starts_with("supports_")
        || column_name.starts_with("enable_")
        || column_name.starts_with("use_")
        || matches!(
            column_name,
            "announcement_notifications"
                | "auto_delete_on_expiry"
                | "auto_fetch_models"
                | "email_notifications"
                | "email_verified"
                | "format_converted"
                | "keep_priority_on_conversion"
                | "signature_valid"
                | "tunnel_connected"
                | "tunnel_mode"
                | "usage_alerts"
                | "webhook_sent"
        )
}

fn mysql_row_payload(row: &sqlx::mysql::MySqlRow) -> Result<Value, DataLayerError> {
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        object.insert(column.name().to_string(), mysql_value_to_json(row, index)?);
    }
    Ok(Value::Object(object))
}

fn mysql_value_to_json(row: &sqlx::mysql::MySqlRow, index: usize) -> Result<Value, DataLayerError> {
    let raw = row.try_get_raw(index).map_sql_err()?;
    if raw.is_null() {
        return Ok(Value::Null);
    }

    match raw.type_info().name().to_ascii_uppercase().as_str() {
        "BOOL" | "BOOLEAN" => Ok(Value::Bool(row.try_get::<bool, _>(index).map_sql_err()?)),
        "TINYINT" | "TINY" | "SMALLINT" | "SHORT" | "MEDIUMINT" | "INT24" | "INT" | "INTEGER"
        | "LONG" | "BIGINT" | "LONGLONG" | "YEAR" => {
            Ok(Value::from(row.try_get::<i64, _>(index).map_sql_err()?))
        }
        "FLOAT" | "DOUBLE" => {
            let value = row.try_get::<f64, _>(index).map_sql_err()?;
            serde_json::Number::from_f64(value)
                .map(Value::Number)
                .ok_or_else(|| {
                    DataLayerError::UnexpectedValue(format!(
                        "mysql export column {} contains non-finite float",
                        index
                    ))
                })
        }
        "DECIMAL" | "NEWDECIMAL" => Ok(Value::String(
            row.try_get::<String, _>(index).map_sql_err()?,
        )),
        "VARCHAR" | "VAR_STRING" | "STRING" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT"
        | "JSON" | "ENUM" | "SET" | "DATE" | "DATETIME" | "TIMESTAMP" | "TIME" => Ok(
            Value::String(row.try_get::<String, _>(index).map_sql_err()?),
        ),
        "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BIT" | "GEOMETRY" => {
            let bytes = row.try_get::<Vec<u8>, _>(index).map_sql_err()?;
            Ok(Value::Array(bytes.into_iter().map(Value::from).collect()))
        }
        other => Err(DataLayerError::UnexpectedValue(format!(
            "unsupported mysql export column type '{other}' at index {index}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{
        build_import_plan, decode_jsonl, encode_jsonl, export_mysql_core_jsonl, export_mysql_jsonl,
        export_postgres_core_jsonl, export_sqlite_core_jsonl, import_mysql_jsonl,
        import_postgres_jsonl, import_sqlite_jsonl, mysql_core_export_domains,
        normalize_postgres_import_payload, omit_request_body_details_from_records,
        postgres_core_export_domains, sqlite_core_export_domains, DataExportManifest,
        DataExportRecord, DataImportPlan, ExportDomain, ExportRow, PostgresImportColumn,
    };
    use crate::driver::postgres::{PostgresPoolConfig, PostgresPoolFactory};
    use crate::lifecycle::migrate::{
        run_migrations as run_postgres_migrations, run_mysql_migrations, run_sqlite_migrations,
    };
    use crate::DatabaseDriver;

    #[test]
    fn jsonl_round_trips_manifest_and_domain_rows() {
        let records = vec![
            DataExportRecord::manifest(DataExportManifest::new(
                1_700_000_000,
                Some(DatabaseDriver::Postgres),
                vec![ExportDomain::Users, ExportDomain::ApiKeys],
            )),
            DataExportRecord::row(
                ExportDomain::Users,
                "user-1",
                json!({
                    "id": "user-1",
                    "email": "owner@example.com"
                }),
            ),
            DataExportRecord::row(
                ExportDomain::ApiKeys,
                "api-key-1",
                json!({
                    "id": "api-key-1",
                    "key_hash": "ciphertext-preserved"
                }),
            ),
        ];

        let encoded = encode_jsonl(&records).expect("records should encode");
        assert_eq!(encoded.lines().count(), 3);

        let decoded = decode_jsonl(&encoded).expect("records should decode");
        assert_eq!(decoded, records);

        let import_plan = build_import_plan(&encoded).expect("import plan should build");
        assert_eq!(
            import_plan.manifest.source_driver,
            Some(DatabaseDriver::Postgres)
        );
        assert_eq!(import_plan.rows(ExportDomain::Users).len(), 1);
        assert_eq!(
            import_plan.rows(ExportDomain::ApiKeys)[0].payload["key_hash"],
            "ciphertext-preserved"
        );
    }

    #[test]
    fn core_export_domains_match_across_sql_drivers() {
        assert_eq!(sqlite_core_export_domains(), mysql_core_export_domains());
        assert_eq!(sqlite_core_export_domains(), postgres_core_export_domains());
    }

    #[test]
    fn postgres_auxiliary_payload_keeps_existing_unix_timestamp_columns() {
        for table_name in [
            "pool_member_scores",
            "background_task_runs",
            "background_task_events",
        ] {
            let spec =
                super::auxiliary_table_spec(table_name).expect("auxiliary table spec should exist");
            assert!(
                spec.timestamp_columns.is_empty(),
                "{table_name} stores unix timestamps directly and must not use EXTRACT(EPOCH)"
            );
            assert!(
                !super::postgres_auxiliary_payload_sql(spec).contains("EXTRACT(EPOCH"),
                "{table_name} export should not treat bigint unix timestamps as Postgres timestamps"
            );
        }
    }

    #[test]
    fn omit_request_body_details_removes_legacy_usage_body_columns() {
        let mut records = vec![
            DataExportRecord::manifest(DataExportManifest::new(
                1_700_000_000,
                Some(DatabaseDriver::Postgres),
                vec![ExportDomain::Usage],
            )),
            DataExportRecord::row(
                ExportDomain::Usage,
                "request-1",
                json!({
                    "request_id": "request-1",
                    "request_body": {"model": "gpt-test"},
                    "provider_request_body": {"temperature": 0.2},
                    "response_body": {"output": "hello"},
                    "client_response_body": {"output_text": "hello"},
                    "request_body_compressed": "compressed-request",
                    "provider_request_body_compressed": "compressed-provider-request",
                    "response_body_compressed": "compressed-response",
                    "client_response_body_compressed": "compressed-client-response",
                    "input_tokens": 10
                }),
            ),
        ];

        omit_request_body_details_from_records(&mut records);

        let DataExportRecord::Row { payload, .. } = &records[1] else {
            panic!("second record should be usage row");
        };
        let object = payload.as_object().expect("payload should be an object");
        for column_name in [
            "request_body",
            "provider_request_body",
            "response_body",
            "client_response_body",
            "request_body_compressed",
            "provider_request_body_compressed",
            "response_body_compressed",
            "client_response_body_compressed",
        ] {
            assert!(!object.contains_key(column_name));
        }
        assert_eq!(payload["request_id"], "request-1");
        assert_eq!(payload["input_tokens"], 10);
    }

    #[test]
    fn jsonl_rejects_missing_manifest() {
        let err =
            decode_jsonl(r#"{"record_type":"row","domain":"users","id":"user-1","payload":{}}"#)
                .expect_err("missing manifest should fail");

        assert!(err.to_string().contains("must start with a manifest"));
    }

    #[test]
    fn jsonl_rejects_rows_outside_manifest_domains() {
        let records = vec![
            DataExportRecord::manifest(DataExportManifest::new(
                1_700_000_000,
                Some(DatabaseDriver::Sqlite),
                vec![ExportDomain::Users],
            )),
            DataExportRecord::row(
                ExportDomain::Wallets,
                "wallet-1",
                json!({ "id": "wallet-1" }),
            ),
        ];

        let err = encode_jsonl(&records).expect_err("undeclared domain should fail");
        assert!(err.to_string().contains("not declared in manifest"));
    }

    #[test]
    fn jsonl_rejects_bad_json_with_line_number() {
        let err = decode_jsonl(
            r#"{"record_type":"manifest","manifest":{"format_version":1,"created_at_unix_secs":1,"source_driver":null,"domains":["users"]}}
not-json"#,
        )
        .expect_err("bad json should fail");

        assert!(err.to_string().contains("line 2"));
    }

    #[test]
    fn jsonl_rejects_duplicate_domain_ids() {
        let records = vec![
            DataExportRecord::manifest(DataExportManifest::new(
                1_700_000_000,
                None,
                vec![ExportDomain::Users],
            )),
            DataExportRecord::row(ExportDomain::Users, "user-1", json!({ "id": "user-1" })),
            DataExportRecord::row(ExportDomain::Users, "user-1", json!({ "id": "user-1" })),
        ];

        let err = encode_jsonl(&records).expect_err("duplicate id should fail");
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn postgres_import_payload_normalizes_sqlite_values_for_target_columns() {
        let target_columns = BTreeMap::from([
            (
                "id".to_string(),
                postgres_column("character varying", "varchar"),
            ),
            (
                "email_verified".to_string(),
                postgres_column("boolean", "bool"),
            ),
            (
                "created_at".to_string(),
                postgres_column("timestamp with time zone", "timestamptz"),
            ),
            (
                "allowed_models".to_string(),
                postgres_column("json", "json"),
            ),
            (
                "role".to_string(),
                postgres_not_null_default_column("USER-DEFINED", "userrole"),
            ),
        ]);
        let row = ExportRow {
            id: "user-1".to_string(),
            payload: json!({
                "id": "user-1",
                "email_verified": 1,
                "created_at": 1,
                "allowed_models": "[\"gpt-test\"]",
                "role": null,
                "legacy_nullable": null
            }),
        };

        let normalized = normalize_postgres_import_payload(
            "public.users",
            ExportDomain::Users,
            &row,
            &target_columns,
        )
        .expect("postgres payload should normalize");

        assert_eq!(normalized["email_verified"], json!(true));
        assert_eq!(normalized["created_at"], json!("1970-01-01T00:00:01+00:00"));
        assert_eq!(normalized["allowed_models"], json!(["gpt-test"]));
        assert!(!normalized.contains_key("role"));
        assert!(!normalized.contains_key("legacy_nullable"));
    }

    #[test]
    fn postgres_import_payload_rejects_non_null_unknown_columns() {
        let target_columns = BTreeMap::from([(
            "id".to_string(),
            postgres_column("character varying", "varchar"),
        )]);
        let row = ExportRow {
            id: "user-1".to_string(),
            payload: json!({
                "id": "user-1",
                "unexpected_column": "value"
            }),
        };

        let err = normalize_postgres_import_payload(
            "public.users",
            ExportDomain::Users,
            &row,
            &target_columns,
        )
        .expect_err("non-null unknown columns should fail");

        assert!(err.to_string().contains("unexpected_column"));
        assert!(err.to_string().contains("does not exist"));
    }

    fn postgres_column(data_type: &str, udt_name: &str) -> PostgresImportColumn {
        PostgresImportColumn {
            data_type: data_type.to_ascii_lowercase(),
            udt_name: udt_name.to_ascii_lowercase(),
            is_nullable: true,
            has_default: false,
        }
    }

    fn postgres_not_null_default_column(data_type: &str, udt_name: &str) -> PostgresImportColumn {
        PostgresImportColumn {
            data_type: data_type.to_ascii_lowercase(),
            udt_name: udt_name.to_ascii_lowercase(),
            is_nullable: false,
            has_default: true,
        }
    }

    #[tokio::test]
    async fn sqlite_core_export_reads_migrated_database_rows() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite pool should connect");
        run_sqlite_migrations(&pool)
            .await
            .expect("sqlite migrations should run");

        sqlx::query(
            r#"
INSERT INTO users (id, email, username, auth_source, created_at, updated_at)
VALUES ('user-1', 'owner@example.com', 'owner', 'local', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO user_groups (id, name, normalized_name, description, priority, allowed_models, allowed_models_mode, created_at, updated_at)
VALUES ('group-1', 'Export Group', 'export group', 'Exported group', 10, '["gpt-test"]', 'specific', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO user_group_members (group_id, user_id, created_at)
VALUES ('group-1', 'user-1', '1970-01-01T00:00:01Z');
INSERT INTO api_keys (id, user_id, key_hash, key_encrypted, name, created_at, updated_at)
VALUES ('api-key-1', 'user-1', 'hash-1', 'ciphertext-1', 'Default', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO providers (id, name, provider_type, created_at, updated_at)
VALUES ('provider-1', 'Provider One', 'openai', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO provider_api_keys (id, provider_id, name, encrypted_key, created_at, updated_at)
VALUES ('provider-key-1', 'provider-1', 'Provider Key', 'ciphertext-provider', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO provider_endpoints (id, provider_id, name, base_url, created_at, updated_at)
VALUES ('endpoint-1', 'provider-1', 'Primary', 'https://example.test', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO global_models (id, name, created_at, updated_at)
VALUES ('global-model-1', 'gpt-test', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO models (id, provider_id, global_model_id, provider_model_name, created_at, updated_at)
VALUES ('model-1', 'provider-1', 'global-model-1', 'gpt-test', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO billing_rules (id, global_model_id, name, task_type, expression, variables, dimension_mappings, is_enabled, created_at, updated_at)
VALUES ('billing-rule-1', 'global-model-1', 'Rule One', 'chat', 'input_tokens * 0.01', '{}', '{"input":"input_tokens"}', 1, '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO dimension_collectors (id, api_format, task_type, dimension_name, source_type, value_type, transform_expression, priority, is_enabled, created_at, updated_at)
VALUES ('collector-1', 'openai', 'chat', 'input_tokens', 'computed', 'float', 'usage.input_tokens', 10, 1, '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO system_configs (id, key, value, created_at, updated_at)
VALUES ('config-1', 'billing.enabled', 'true', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO wallets (id, user_id, created_at, updated_at)
VALUES ('wallet-1', 'user-1', '1970-01-01T00:00:01Z', '1970-01-01T00:00:02Z');
INSERT INTO "usage" (request_id, id, user_id, provider_name, model, status, billing_status, created_at_unix_ms, updated_at_unix_secs)
VALUES ('request-1', 'request-1', 'user-1', 'Provider One', 'gpt-test', 'completed', 'settled', 1, 2);
INSERT INTO usage_routing_snapshots (request_id, selected_provider_id, route_kind)
VALUES ('request-1', 'provider-1', 'direct');
INSERT INTO stats_user_daily_model (id, user_id, date, model, total_requests)
VALUES ('stats-user-model-1', 'user-1', 1715731200, 'gpt-test', 7);
"#,
        )
        .execute(&pool)
        .await
        .expect("sqlite export rows should seed");

        let encoded = export_sqlite_core_jsonl(&pool, 1_700_000_000)
            .await
            .expect("sqlite export should encode");
        let import_plan = build_import_plan(&encoded).expect("sqlite export should decode");

        assert_eq!(
            import_plan.manifest.source_driver,
            Some(DatabaseDriver::Sqlite)
        );
        assert_eq!(import_plan.manifest.domains, sqlite_core_export_domains());
        assert_eq!(
            import_plan.rows(ExportDomain::Users)[0].payload["email"],
            "owner@example.com"
        );
        assert!(import_plan
            .rows(ExportDomain::UserGroups)
            .iter()
            .any(|row| row.id == "group-1" && row.payload["name"] == "Export Group"));
        assert!(import_plan
            .rows(ExportDomain::UserGroupMembers)
            .iter()
            .any(|row| row.id == "group-1:user-1"
                && row.payload["group_id"] == "group-1"
                && row.payload["user_id"] == "user-1"));
        assert_eq!(
            import_plan.rows(ExportDomain::ApiKeys)[0].payload["key_encrypted"],
            "ciphertext-1"
        );
        assert_eq!(
            import_plan.rows(ExportDomain::ProviderKeys)[0].payload["encrypted_key"],
            "ciphertext-provider"
        );
        assert_eq!(import_plan.rows(ExportDomain::Usage)[0].id, "request-1");
        assert!(import_plan
            .rows(ExportDomain::Stats)
            .iter()
            .any(|row| row.id == "stats_user_daily_model:stats-user-model-1"
                && row.payload["__table"] == "stats_user_daily_model"
                && row.payload["total_requests"] == 7));
        assert!(import_plan
            .rows(ExportDomain::Auxiliary)
            .iter()
            .any(|row| row.id == "usage_routing_snapshots:request-1"
                && row.payload["__table"] == "usage_routing_snapshots"
                && row.payload["selected_provider_id"] == "provider-1"));
        assert_eq!(import_plan.rows(ExportDomain::Billing).len(), 2);
        assert_eq!(
            import_plan.rows(ExportDomain::Billing)[0].payload["__table"],
            "billing_rules"
        );
        assert_eq!(
            import_plan.rows(ExportDomain::Billing)[0].payload["dimension_mappings"]["input"],
            "input_tokens"
        );

        let target_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("target sqlite pool should connect");
        run_sqlite_migrations(&target_pool)
            .await
            .expect("target sqlite migrations should run");
        let imported = import_sqlite_jsonl(&target_pool, &encoded)
            .await
            .expect("sqlite import should load exported rows");
        assert_eq!(imported, import_plan_row_count(&import_plan));

        let imported_api_key = sqlx::query_as::<_, (String,)>(
            "SELECT key_encrypted FROM api_keys WHERE id = 'api-key-1'",
        )
        .fetch_one(&target_pool)
        .await
        .expect("imported api key should load");
        assert_eq!(imported_api_key.0, "ciphertext-1");

        let imported_usage = sqlx::query_as::<_, (String,)>(
            "SELECT request_id FROM \"usage\" WHERE request_id = 'request-1'",
        )
        .fetch_one(&target_pool)
        .await
        .expect("imported usage should load");
        assert_eq!(imported_usage.0, "request-1");

        let imported_group_member = sqlx::query_as::<_, (String, String)>(
            "SELECT group_id, user_id FROM user_group_members WHERE group_id = 'group-1' AND user_id = 'user-1'",
        )
        .fetch_one(&target_pool)
        .await
        .expect("imported user group member should load");
        assert_eq!(imported_group_member.0, "group-1");
        assert_eq!(imported_group_member.1, "user-1");

        let imported_billing_rule = sqlx::query_as::<_, (String,)>(
            "SELECT expression FROM billing_rules WHERE id = 'billing-rule-1'",
        )
        .fetch_one(&target_pool)
        .await
        .expect("imported billing rule should load");
        assert_eq!(imported_billing_rule.0, "input_tokens * 0.01");

        let imported_stats = sqlx::query_as::<_, (i64,)>(
            "SELECT total_requests FROM stats_user_daily_model WHERE id = 'stats-user-model-1'",
        )
        .fetch_one(&target_pool)
        .await
        .expect("imported stats row should load");
        assert_eq!(imported_stats.0, 7);

        let imported_routing_snapshot = sqlx::query_as::<_, (String,)>(
            "SELECT selected_provider_id FROM usage_routing_snapshots WHERE request_id = 'request-1'",
        )
        .fetch_one(&target_pool)
        .await
        .expect("imported routing snapshot should load");
        assert_eq!(imported_routing_snapshot.0, "provider-1");

        if let Some(database_url) = std::env::var("AETHER_TEST_POSTGRES_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            let config = PostgresPoolConfig {
                database_url,
                min_connections: 1,
                max_connections: 1,
                acquire_timeout_ms: 1_000,
                idle_timeout_ms: 5_000,
                max_lifetime_ms: 30_000,
                statement_cache_capacity: 64,
                require_ssl: false,
            };
            let postgres_pool = PostgresPoolFactory::new(config)
                .expect("postgres factory should build")
                .connect_lazy()
                .expect("postgres pool should build");
            run_postgres_migrations(&postgres_pool)
                .await
                .expect("postgres migrations should run");

            let imported = import_postgres_jsonl(&postgres_pool, &encoded)
                .await
                .expect("postgres import should load exported rows");
            assert_eq!(imported, import_plan_row_count(&import_plan));

            let imported_api_key = sqlx::query_as::<_, (String,)>(
                "SELECT key_encrypted FROM api_keys WHERE id = 'api-key-1'",
            )
            .fetch_one(&postgres_pool)
            .await
            .expect("imported postgres api key should load");
            assert_eq!(imported_api_key.0, "ciphertext-1");
        }
    }

    #[tokio::test]
    async fn postgres_core_export_reads_migrated_database_rows_when_url_is_set() {
        let Some(database_url) = std::env::var("AETHER_TEST_POSTGRES_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            eprintln!(
                "skipping postgres core export smoke test because AETHER_TEST_POSTGRES_URL is unset"
            );
            return;
        };

        let config = PostgresPoolConfig {
            database_url,
            min_connections: 1,
            max_connections: 1,
            acquire_timeout_ms: 1_000,
            idle_timeout_ms: 5_000,
            max_lifetime_ms: 30_000,
            statement_cache_capacity: 64,
            require_ssl: false,
        };
        let pool = PostgresPoolFactory::new(config)
            .expect("postgres factory should build")
            .connect_lazy()
            .expect("postgres pool should build");
        run_postgres_migrations(&pool)
            .await
            .expect("postgres migrations should run");

        let suffix = unique_suffix();
        let user_id = format!("export-user-{suffix}");
        let api_key_id = format!("export-api-key-{suffix}");
        let provider_id = format!("export-provider-{suffix}");
        let provider_key_id = format!("export-provider-key-{suffix}");
        let endpoint_id = format!("export-endpoint-{suffix}");
        let global_model_id = format!("export-global-model-{suffix}");
        let model_id = format!("export-model-{suffix}");
        let billing_rule_id = format!("export-billing-rule-{suffix}");
        let collector_id = format!("export-collector-{suffix}");
        let config_id = format!("export-config-{suffix}");
        let config_key = format!("export.config.{suffix}");
        let wallet_id = format!("export-wallet-{suffix}");
        let request_id = format!("export-request-{suffix}");
        let group_id = format!("export-group-{suffix}");

        sqlx::query(
            "INSERT INTO users (id, email, username, auth_source, email_verified, created_at, updated_at) VALUES ($1, $2, $3, 'local', TRUE, to_timestamp(1), to_timestamp(2))",
        )
        .bind(&user_id)
        .bind(format!("{user_id}@example.com"))
        .bind(format!("owner-{suffix}"))
        .execute(&pool)
        .await
        .expect("user should seed");
        sqlx::query(
            "INSERT INTO user_groups (id, name, normalized_name, priority, allowed_models, allowed_models_mode, created_at, updated_at) VALUES ($1, $2, $3, 10, '[\"provider-model\"]', 'specific', to_timestamp(1), to_timestamp(2))",
        )
        .bind(&group_id)
        .bind(format!("Export Group {suffix}"))
        .bind(format!("export group {suffix}"))
        .execute(&pool)
        .await
        .expect("user group should seed");
        sqlx::query(
            "INSERT INTO user_group_members (group_id, user_id, created_at) VALUES ($1, $2, to_timestamp(1))",
        )
        .bind(&group_id)
        .bind(&user_id)
        .execute(&pool)
        .await
        .expect("user group member should seed");
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, key_hash, key_encrypted, name, created_at, updated_at) VALUES ($1, $2, $3, 'ciphertext-1', 'Default', to_timestamp(1), to_timestamp(2))",
        )
        .bind(&api_key_id)
        .bind(&user_id)
        .bind(format!("hash-{api_key_id}"))
        .execute(&pool)
        .await
        .expect("api key should seed");
        sqlx::query(
            "INSERT INTO providers (id, name, provider_type, created_at, updated_at) VALUES ($1, $2, 'openai', to_timestamp(1), to_timestamp(2))",
        )
        .bind(&provider_id)
        .bind(format!("Provider {suffix}"))
        .execute(&pool)
        .await
        .expect("provider should seed");
        sqlx::query(
            "INSERT INTO provider_api_keys (id, provider_id, name, encrypted_key, total_tokens, total_cost_usd, created_at, updated_at) VALUES ($1, $2, 'Provider Key', 'ciphertext-provider', 0, 0, to_timestamp(1), to_timestamp(2))",
        )
        .bind(&provider_key_id)
        .bind(&provider_id)
        .execute(&pool)
        .await
        .expect("provider key should seed");
        sqlx::query(
            "INSERT INTO provider_endpoints (id, provider_id, name, base_url, created_at, updated_at) VALUES ($1, $2, 'Primary', 'https://example.test', to_timestamp(1), to_timestamp(2))",
        )
        .bind(&endpoint_id)
        .bind(&provider_id)
        .execute(&pool)
        .await
        .expect("endpoint should seed");
        sqlx::query(
            "INSERT INTO global_models (id, name, created_at, updated_at) VALUES ($1, $2, to_timestamp(1), to_timestamp(2))",
        )
        .bind(&global_model_id)
        .bind(format!("global-model-{suffix}"))
        .execute(&pool)
        .await
        .expect("global model should seed");
        sqlx::query(
            "INSERT INTO models (id, provider_id, global_model_id, provider_model_name, created_at, updated_at) VALUES ($1, $2, $3, 'provider-model', to_timestamp(1), to_timestamp(2))",
        )
        .bind(&model_id)
        .bind(&provider_id)
        .bind(&global_model_id)
        .execute(&pool)
        .await
        .expect("model should seed");
        sqlx::query(
            "INSERT INTO billing_rules (id, global_model_id, name, task_type, expression, variables, dimension_mappings, is_enabled, created_at, updated_at) VALUES ($1, $2, 'Rule One', 'chat', 'input_tokens * 0.01', '{}', '{\"input\":\"input_tokens\"}', TRUE, to_timestamp(1), to_timestamp(2))",
        )
        .bind(&billing_rule_id)
        .bind(&global_model_id)
        .execute(&pool)
        .await
        .expect("billing rule should seed");
        sqlx::query(
            "INSERT INTO dimension_collectors (id, api_format, task_type, dimension_name, source_type, value_type, transform_expression, priority, is_enabled, created_at, updated_at) VALUES ($1, 'openai', 'chat', $2, 'computed', 'float', 'usage.input_tokens', 10, TRUE, to_timestamp(1), to_timestamp(2))",
        )
        .bind(&collector_id)
        .bind(format!("input_tokens_{suffix}"))
        .execute(&pool)
        .await
        .expect("dimension collector should seed");
        sqlx::query(
            "INSERT INTO system_configs (id, key, value, created_at, updated_at) VALUES ($1, $2, 'true', to_timestamp(1), to_timestamp(2))",
        )
        .bind(&config_id)
        .bind(&config_key)
        .execute(&pool)
        .await
        .expect("system config should seed");
        sqlx::query(
            "INSERT INTO wallets (id, user_id, created_at, updated_at) VALUES ($1, $2, to_timestamp(1), to_timestamp(2))",
        )
        .bind(&wallet_id)
        .bind(&user_id)
        .execute(&pool)
        .await
        .expect("wallet should seed");
        sqlx::query(
            "INSERT INTO \"usage\" (request_id, id, user_id, provider_name, model, status, billing_status, created_at_unix_ms, updated_at_unix_secs) VALUES ($1, $2, $3, 'Provider One', 'provider-model', 'completed', 'settled', 1, 2)",
        )
        .bind(&request_id)
        .bind(&request_id)
        .bind(&user_id)
        .execute(&pool)
        .await
        .expect("usage should seed");

        let encoded = export_postgres_core_jsonl(&pool, 1_700_000_000)
            .await
            .expect("postgres export should encode");
        let import_plan = build_import_plan(&encoded).expect("postgres export should decode");

        assert_eq!(
            import_plan.manifest.source_driver,
            Some(DatabaseDriver::Postgres)
        );
        assert_eq!(import_plan.manifest.domains, postgres_core_export_domains());
        assert!(import_plan
            .rows(ExportDomain::Users)
            .iter()
            .any(|row| row.id == user_id));
        assert!(import_plan
            .rows(ExportDomain::UserGroups)
            .iter()
            .any(|row| row.id == group_id));
        assert!(import_plan
            .rows(ExportDomain::UserGroupMembers)
            .iter()
            .any(|row| row.id == format!("{group_id}:{user_id}")));
        assert!(import_plan
            .rows(ExportDomain::ApiKeys)
            .iter()
            .any(|row| row.id == api_key_id && row.payload["key_encrypted"] == "ciphertext-1"));
        assert!(import_plan
            .rows(ExportDomain::ProviderKeys)
            .iter()
            .any(|row| {
                row.id == provider_key_id && row.payload["encrypted_key"] == "ciphertext-provider"
            }));
        assert!(import_plan
            .rows(ExportDomain::GlobalModels)
            .iter()
            .any(|row| row.id == global_model_id));
        assert!(import_plan
            .rows(ExportDomain::Models)
            .iter()
            .any(|row| row.id == model_id));
        assert!(import_plan
            .rows(ExportDomain::Usage)
            .iter()
            .any(|row| row.id == request_id));

        let target_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("target sqlite pool should connect");
        run_sqlite_migrations(&target_pool)
            .await
            .expect("target sqlite migrations should run");
        let imported = import_sqlite_jsonl(&target_pool, &encoded)
            .await
            .expect("sqlite import should load postgres exported rows");
        assert_eq!(imported, import_plan_row_count(&import_plan));

        let imported_api_key =
            sqlx::query_as::<_, (String,)>("SELECT key_encrypted FROM api_keys WHERE id = $1")
                .bind(&api_key_id)
                .fetch_one(&target_pool)
                .await
                .expect("imported sqlite api key should load");
        assert_eq!(imported_api_key.0, "ciphertext-1");
        let imported_group_member = sqlx::query_as::<_, (String, String)>(
            "SELECT group_id, user_id FROM user_group_members WHERE group_id = ? AND user_id = ?",
        )
        .bind(&group_id)
        .bind(&user_id)
        .fetch_one(&target_pool)
        .await
        .expect("imported sqlite user group member should load");
        assert_eq!(imported_group_member.0, group_id);
        assert_eq!(imported_group_member.1, user_id);
    }

    #[tokio::test]
    async fn mysql_core_export_reads_migrated_database_rows_when_url_is_set() {
        let Some(database_url) = std::env::var("AETHER_TEST_MYSQL_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            eprintln!(
                "skipping mysql core export smoke test because AETHER_TEST_MYSQL_URL is unset"
            );
            return;
        };

        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("mysql test pool should connect");
        run_mysql_migrations(&pool)
            .await
            .expect("mysql migrations should run");

        let suffix = unique_suffix();
        let user_id = format!("export-user-{suffix}");
        let api_key_id = format!("export-api-key-{suffix}");
        let provider_id = format!("export-provider-{suffix}");
        let provider_key_id = format!("export-provider-key-{suffix}");
        let endpoint_id = format!("export-endpoint-{suffix}");
        let global_model_id = format!("export-global-model-{suffix}");
        let model_id = format!("export-model-{suffix}");
        let config_id = format!("export-config-{suffix}");
        let wallet_id = format!("export-wallet-{suffix}");
        let request_id = format!("export-request-{suffix}");
        let group_id = format!("export-group-{suffix}");

        sqlx::query(
            "INSERT INTO users (id, email, username, auth_source, created_at, updated_at) VALUES (?, ?, ?, 'local', 1, 2)",
        )
        .bind(&user_id)
        .bind(format!("{user_id}@example.com"))
        .bind(format!("owner-{suffix}"))
        .execute(&pool)
        .await
        .expect("user should seed");
        sqlx::query(
            "INSERT INTO user_groups (id, name, normalized_name, priority, allowed_models, allowed_models_mode, created_at, updated_at) VALUES (?, ?, ?, 10, '[\"provider-model\"]', 'specific', 1, 2)",
        )
        .bind(&group_id)
        .bind(format!("Export Group {suffix}"))
        .bind(format!("export group {suffix}"))
        .execute(&pool)
        .await
        .expect("user group should seed");
        sqlx::query(
            "INSERT INTO user_group_members (group_id, user_id, created_at) VALUES (?, ?, 1)",
        )
        .bind(&group_id)
        .bind(&user_id)
        .execute(&pool)
        .await
        .expect("user group member should seed");
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, key_hash, key_encrypted, name, created_at, updated_at) VALUES (?, ?, ?, 'ciphertext-1', 'Default', 1, 2)",
        )
        .bind(&api_key_id)
        .bind(&user_id)
        .bind(format!("hash-{api_key_id}"))
        .execute(&pool)
        .await
        .expect("api key should seed");
        sqlx::query(
            "INSERT INTO providers (id, name, provider_type, created_at, updated_at) VALUES (?, ?, 'openai', 1, 2)",
        )
        .bind(&provider_id)
        .bind(format!("Provider {suffix}"))
        .execute(&pool)
        .await
        .expect("provider should seed");
        sqlx::query(
            "INSERT INTO provider_api_keys (id, provider_id, name, encrypted_key, created_at, updated_at) VALUES (?, ?, 'Provider Key', 'ciphertext-provider', 1, 2)",
        )
        .bind(&provider_key_id)
        .bind(&provider_id)
        .execute(&pool)
        .await
        .expect("provider key should seed");
        sqlx::query(
            "INSERT INTO provider_endpoints (id, provider_id, name, base_url, created_at, updated_at) VALUES (?, ?, 'Primary', 'https://example.test', 1, 2)",
        )
        .bind(&endpoint_id)
        .bind(&provider_id)
        .execute(&pool)
        .await
        .expect("endpoint should seed");
        sqlx::query(
            "INSERT INTO global_models (id, name, created_at, updated_at) VALUES (?, ?, 1, 2)",
        )
        .bind(&global_model_id)
        .bind(format!("global-model-{suffix}"))
        .execute(&pool)
        .await
        .expect("global model should seed");
        sqlx::query(
            "INSERT INTO models (id, provider_id, global_model_id, provider_model_name, created_at, updated_at) VALUES (?, ?, ?, 'provider-model', 1, 2)",
        )
        .bind(&model_id)
        .bind(&provider_id)
        .bind(&global_model_id)
        .execute(&pool)
        .await
        .expect("model should seed");
        sqlx::query(
            "INSERT INTO system_configs (id, `key`, value, created_at, updated_at) VALUES (?, ?, 'true', 1, 2)",
        )
        .bind(&config_id)
        .bind(format!("export.config.{suffix}"))
        .execute(&pool)
        .await
        .expect("system config should seed");
        sqlx::query(
            "INSERT INTO wallets (id, user_id, created_at, updated_at) VALUES (?, ?, 1, 2)",
        )
        .bind(&wallet_id)
        .bind(&user_id)
        .execute(&pool)
        .await
        .expect("wallet should seed");
        sqlx::query(
            "INSERT INTO `usage` (request_id, id, user_id, provider_name, model, status, billing_status, created_at_unix_ms, updated_at_unix_secs) VALUES (?, ?, ?, 'Provider One', 'provider-model', 'completed', 'settled', 1, 2)",
        )
        .bind(&request_id)
        .bind(&request_id)
        .bind(&user_id)
        .execute(&pool)
        .await
        .expect("usage should seed");

        let encoded = export_mysql_core_jsonl(&pool, 1_700_000_000)
            .await
            .expect("mysql export should encode");
        let import_plan = build_import_plan(&encoded).expect("mysql export should decode");

        assert_eq!(
            import_plan.manifest.source_driver,
            Some(DatabaseDriver::Mysql)
        );
        assert_eq!(import_plan.manifest.domains, mysql_core_export_domains());
        assert!(import_plan
            .rows(ExportDomain::Users)
            .iter()
            .any(|row| row.id == user_id));
        assert!(import_plan
            .rows(ExportDomain::UserGroups)
            .iter()
            .any(|row| row.id == group_id));
        assert!(import_plan
            .rows(ExportDomain::UserGroupMembers)
            .iter()
            .any(|row| row.id == format!("{group_id}:{user_id}")));
        assert!(import_plan
            .rows(ExportDomain::ApiKeys)
            .iter()
            .any(|row| row.id == api_key_id && row.payload["key_encrypted"] == "ciphertext-1"));
        assert!(import_plan
            .rows(ExportDomain::ProviderKeys)
            .iter()
            .any(|row| {
                row.id == provider_key_id && row.payload["encrypted_key"] == "ciphertext-provider"
            }));
        assert!(import_plan
            .rows(ExportDomain::Usage)
            .iter()
            .any(|row| row.id == request_id));

        let selected_export = export_mysql_jsonl(
            &pool,
            vec![
                ExportDomain::Users,
                ExportDomain::UserGroups,
                ExportDomain::UserGroupMembers,
                ExportDomain::ApiKeys,
                ExportDomain::ProviderKeys,
                ExportDomain::Usage,
            ],
            1_700_000_001,
        )
        .await
        .expect("selected mysql export should encode");
        let imported = import_mysql_jsonl(&pool, &selected_export)
            .await
            .expect("mysql import should be idempotent");
        assert!(imported >= 6);

        let imported_api_key =
            sqlx::query_as::<_, (String,)>("SELECT key_encrypted FROM api_keys WHERE id = ?")
                .bind(&api_key_id)
                .fetch_one(&pool)
                .await
                .expect("imported mysql api key should load");
        assert_eq!(imported_api_key.0, "ciphertext-1");
    }

    fn unique_suffix() -> String {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let counter = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{:016x}", nanos ^ counter.rotate_left(17))
    }

    fn import_plan_row_count(plan: &DataImportPlan) -> usize {
        plan.manifest
            .domains
            .iter()
            .map(|domain| plan.rows(*domain).len())
            .sum()
    }
}
