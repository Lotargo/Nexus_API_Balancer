-- Initial schema for Nexus API Balancer

-- 1. API Key Pools
CREATE TABLE IF NOT EXISTS pools (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    pool_type TEXT NOT NULL, -- e.g., 'balanced', 'priority'
    capacity INTEGER NOT NULL DEFAULT 1,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 2. API Keys
CREATE TABLE IF NOT EXISTS keys (
    id TEXT PRIMARY KEY,
    pool_id TEXT NOT NULL,
    secret TEXT NOT NULL,
    tier_limit INTEGER NOT NULL,
    concurrency INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active', -- 'active', 'revoked', 'expired'
    expires_at DATETIME,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (pool_id) REFERENCES pools(id) ON DELETE CASCADE
);

-- 3. Clients
CREATE TABLE IF NOT EXISTS clients (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    auth_token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'active', -- 'active', 'suspended'
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 3.1 Client to Pool Mapping (Isolation)
CREATE TABLE IF NOT EXISTS client_pools (
    client_id TEXT NOT NULL,
    pool_id TEXT NOT NULL,
    PRIMARY KEY (client_id, pool_id),
    FOREIGN KEY (client_id) REFERENCES clients(id) ON DELETE CASCADE,
    FOREIGN KEY (pool_id) REFERENCES pools(id) ON DELETE CASCADE
);

-- 4. Request Logs (for analytics and history)
CREATE TABLE IF NOT EXISTS request_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    client_id TEXT,
    key_id TEXT,
    pool_id TEXT,
    status TEXT NOT NULL, -- 'success', 'rate_limited', 'error'
    latency_ms INTEGER,
    error_message TEXT,
    request_ip TEXT,
    FOREIGN KEY (client_id) REFERENCES clients(id) ON DELETE SET NULL,
    FOREIGN KEY (key_id) REFERENCES keys(id) ON DELETE SET NULL
);

-- 5. Access Control List
CREATE TABLE IF NOT EXISTS access_control (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    acl_type TEXT NOT NULL, -- 'blacklist', 'whitelist'
    target_type TEXT NOT NULL, -- 'ip', 'client_id'
    value TEXT NOT NULL,
    reason TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Indexes for performance
CREATE INDEX IF NOT EXISTS idx_request_logs_timestamp ON request_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_keys_pool_id ON keys(pool_id);
CREATE INDEX IF NOT EXISTS idx_clients_auth_token ON clients(auth_token);
