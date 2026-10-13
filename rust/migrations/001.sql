PRAGMA journal_mode=WAL;
PRAGMA synchronous=FULL;
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS trusted_peers (
 fingerprint TEXT PRIMARY KEY, name TEXT NOT NULL, approved_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS transfers (
 id TEXT PRIMARY KEY, peer TEXT NOT NULL, direction TEXT NOT NULL,
 source TEXT, destination TEXT NOT NULL, address TEXT, manifest TEXT NOT NULL,
 status TEXT NOT NULL, error TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 UNIQUE(direction,destination)
);
CREATE TABLE IF NOT EXISTS chunks (
 transfer_id TEXT NOT NULL REFERENCES transfers(id), idx INTEGER NOT NULL,
 hash TEXT NOT NULL, verified INTEGER NOT NULL CHECK(verified IN (0,1)),
 PRIMARY KEY(transfer_id,idx)
);
PRAGMA user_version=1;
