use crate::protocol::Manifest;
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
#[derive(Clone)]
pub struct Store(pub PathBuf);
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let s = Self(path.to_owned());
        s.db()?
            .execute_batch(include_str!("../migrations/001.sql"))?;
        s.db()?
            .execute_batch(include_str!("../migrations/002.sql"))?;
        Ok(s)
    }
    pub fn limits(&self) -> Result<Option<crate::engine::TransferLimits>> {
        let raw = self
            .db()?
            .query_row(
                "SELECT value FROM settings WHERE key='transfer_limits'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        raw.map(|raw| Ok(serde_json::from_str(&raw)?)).transpose()
    }
    pub fn save_limits(&self, limits: &crate::engine::TransferLimits) -> Result<()> {
        self.db()?.execute(
            "INSERT OR REPLACE INTO settings VALUES ('transfer_limits',?)",
            [serde_json::to_string(limits)?],
        )?;
        Ok(())
    }
    fn db(&self) -> Result<Connection> {
        let c = Connection::open(&self.0)?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        c.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")?;
        Ok(c)
    }
    pub fn trusted(&self, p: &str) -> Result<bool> {
        Ok(self
            .db()?
            .query_row(
                "SELECT 1 FROM trusted_peers WHERE fingerprint=?",
                [p],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_some())
    }
    pub fn trust(&self, p: &str, name: &str) -> Result<()> {
        self.db()?.execute(
            "INSERT OR REPLACE INTO trusted_peers VALUES (?,?,?)",
            params![p, name, now()],
        )?;
        Ok(())
    }
    pub fn forget(&self, p: &str) -> Result<()> {
        self.db()?
            .execute("DELETE FROM trusted_peers WHERE fingerprint=?", [p])?;
        Ok(())
    }
    pub fn peers(&self) -> Result<Vec<(String, String)>> {
        let c = self.db()?;
        let mut q = c.prepare("SELECT fingerprint,name FROM trusted_peers")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn register(
        &self,
        m: &Manifest,
        peer: &str,
        direction: &str,
        source: Option<&str>,
        dest: &str,
        address: Option<&str>,
    ) -> Result<()> {
        let c = self.db()?;
        if let Some((old, p, d, old_source, old_dest)) = c
            .query_row(
                "SELECT manifest,peer,direction,source,destination FROM transfers WHERE id=?",
                [&m.id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?
        {
            ensure!(
                serde_json::from_str::<Manifest>(&old)? == *m && p == peer && d == direction,
                "transfer identity/manifest mismatch"
            );
            ensure!(
                old_source.as_deref() == source && old_dest == dest,
                "transfer storage path changed; use the original folder or start a new transfer"
            );
            return Ok(());
        }
        c.execute(
            "INSERT INTO transfers VALUES (?,?,?,?,?,?,?,?,?,?,?)",
            params![
                m.id,
                peer,
                direction,
                source,
                dest,
                address,
                serde_json::to_string(m)?,
                "pending",
                Option::<String>::None,
                now(),
                now()
            ],
        )?;
        Ok(())
    }
    pub fn chunk(&self, id: &str, index: u32, hash: &str) -> Result<()> {
        self.db()?.execute(
            "INSERT OR REPLACE INTO chunks VALUES (?,?,?,1)",
            params![id, index, hash],
        )?;
        Ok(())
    }
    pub fn receive_reservations(&self, exclude_id: &str) -> Result<(u64, usize)> {
        let db = self.db()?;
        let mut query = db.prepare("SELECT manifest FROM transfers WHERE direction='receive' AND status!='complete' AND id!=?")?;
        let mut rows = query.query([exclude_id])?;
        let mut bytes = 0u64;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            let manifest: Manifest = serde_json::from_str(&row.get::<_, String>(0)?)?;
            bytes = bytes
                .checked_add(manifest.size)
                .ok_or_else(|| anyhow::anyhow!("receive reservation overflow"))?;
            count += 1;
        }
        Ok((bytes, count))
    }
    pub fn registered_transfer(&self, id: &str) -> Result<(Manifest, String, String)> {
        let db = self.db()?;
        let (manifest, direction, destination) = db.query_row(
            "SELECT manifest,direction,destination FROM transfers WHERE id=?",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )?;
        Ok((serde_json::from_str(&manifest)?, direction, destination))
    }
    pub fn remove_transfer(&self, id: &str) -> Result<()> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        tx.execute("DELETE FROM chunks WHERE transfer_id=?", [id])?;
        tx.execute("DELETE FROM transfers WHERE id=?", [id])?;
        tx.commit()?;
        Ok(())
    }
    pub fn indexes(&self, id: &str) -> Result<Vec<u32>> {
        let c = self.db()?;
        let mut q =
            c.prepare("SELECT idx FROM chunks WHERE transfer_id=? AND verified=1 ORDER BY idx")?;
        let rows = q.query_map([id], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn invalidate(&self, id: &str, index: u32) -> Result<()> {
        self.db()?.execute(
            "DELETE FROM chunks WHERE transfer_id=? AND idx=?",
            params![id, index],
        )?;
        Ok(())
    }
    pub fn status(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.db()?.execute(
            "UPDATE transfers SET status=?,error=?,updated_at=? WHERE id=?",
            params![status, error, now(), id],
        )?;
        Ok(())
    }
    pub fn relocate(&self, id: &str, address: &str) -> Result<()> {
        self.db()?.execute(
            "UPDATE transfers SET address=?,updated_at=? WHERE id=? AND direction='send'",
            params![address, now(), id],
        )?;
        Ok(())
    }
    pub fn outbound(&self, id: &str) -> Result<(Manifest, String, String, String)> {
        let c = self.db()?;
        let (m, p, a, s) = c.query_row(
            "SELECT manifest,peer,address,source FROM transfers WHERE id=? AND direction='send'",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )?;
        Ok((serde_json::from_str(&m)?, p, a, s))
    }
    pub fn transfer_status(&self, id: &str) -> Result<String> {
        Ok(self
            .db()?
            .query_row("SELECT status FROM transfers WHERE id=?", [id], |r| {
                r.get(0)
            })?)
    }
    pub fn history(&self) -> Result<String> {
        let c = self.db()?;
        let mut query = c.prepare(
            "SELECT id,direction,destination,status,error,peer,
             json_extract(manifest,'$.name'),json_extract(manifest,'$.size'),
             json_extract(manifest,'$.whole_hash'),updated_at,
             json_extract(manifest,'$.version')
             FROM transfers ORDER BY updated_at DESC LIMIT 100",
        )?;
        let rows = query.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "direction": row.get::<_, String>(1)?,
                "name": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "error": row.get::<_, Option<String>>(4)?,
                "peer": row.get::<_, String>(5)?,
                "fileName": row.get::<_, String>(6)?,
                "fileBytes": row.get::<_, u64>(7)?,
                "sha256": row.get::<_, String>(8)?,
                "updatedAt": row.get::<_, i64>(9)?,
                "manifestVersion": row.get::<_, u32>(10)?,
            }))
        })?;
        Ok(serde_json::to_string(
            &rows.collect::<std::result::Result<Vec<_>, _>>()?,
        )?)
    }
    pub fn snapshot(&self) -> Result<Vec<crate::engine::TransferProgress>> {
        let c = self.db()?;
        let mut query=c.prepare("SELECT id,direction,destination,status,error,peer,json_extract(manifest,'$.name'),json_extract(manifest,'$.size'),json_extract(manifest,'$.chunk_size') FROM transfers ORDER BY CASE WHEN direction='receive' AND status!='complete' THEN 0 WHEN status!='complete' THEN 1 ELSE 2 END, updated_at DESC LIMIT 256")?;
        let mut rows = query.query([])?;
        let mut result = vec![];
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let direction: String = row.get(1)?;
            let status: String = row.get(3)?;
            let total_bytes: u64 = row.get(7)?;
            let chunk_size: u64 = row.get(8)?;
            let mut verified_bytes = 0u64;
            for index in self.indexes(&id)? {
                let offset = (index as u64)
                    .checked_mul(chunk_size)
                    .ok_or_else(|| anyhow::anyhow!("chunk offset overflow"))?;
                ensure!(offset < total_bytes, "invalid persisted chunk index");
                verified_bytes += chunk_size.min(total_bytes - offset);
            }
            if status == "complete" {
                verified_bytes = total_bytes;
            }
            let error: Option<String> = row.get(4)?;
            result.push(crate::engine::TransferProgress {
                id,
                direction: direction.clone(),
                name: if direction == "receive" {
                    row.get(2)?
                } else {
                    row.get(6)?
                },
                status,
                peer: row.get(5)?,
                verified_bytes,
                total_bytes,
                error: error.map(|message| {
                    serde_json::from_str(&message)
                        .unwrap_or_else(|_| crate::errors::from_error(&anyhow::anyhow!(message)))
                }),
            });
        }
        Ok(result)
    }
    pub fn confirm_missing(&self, m: &Manifest, missing: &[u32]) -> Result<()> {
        let missing: std::collections::HashSet<u32> = missing.iter().copied().collect();
        let mut c = self.db()?;
        let transaction = c.transaction()?;
        transaction.execute("DELETE FROM chunks WHERE transfer_id=?", [&m.id])?;
        {
            let mut insert = transaction.prepare("INSERT INTO chunks VALUES (?,?,?,1)")?;
            for (index, hash) in m.hashes.iter().enumerate() {
                if !missing.contains(&(index as u32)) {
                    insert.execute(params![m.id, index as u32, hash])?;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }
}
