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
