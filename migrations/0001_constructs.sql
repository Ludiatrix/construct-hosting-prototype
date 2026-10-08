CREATE TABLE registry.constructs (
    address TEXT PRIMARY KEY CHECK (address ~ '^sha256:[0-9a-f]{64}$'),
    filename TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('usda', 'usdc', 'usdz')),
    size_bytes BIGINT NOT NULL CHECK (size_bytes BETWEEN 1 AND 134217728),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    default_prim TEXT NOT NULL,
    prim_count BIGINT NOT NULL CHECK (prim_count > 0)
);
CREATE INDEX constructs_collection ON registry.constructs (created_at DESC, address DESC);
