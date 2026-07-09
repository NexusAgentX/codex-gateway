ALTER TABLE upstreams ADD COLUMN api_key_secret_version INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS admin_audit_logs (
    id TEXT PRIMARY KEY,
    actor_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    actor_email TEXT NOT NULL,
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT,
    status TEXT NOT NULL,
    metadata_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_admin_audit_actor_created ON admin_audit_logs(actor_user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_admin_audit_resource_created ON admin_audit_logs(resource_type, resource_id, created_at DESC);
