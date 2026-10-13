use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub const VERSION: u16 = 1;
pub const WIRE_VERSION: u16 = 2;
pub const MAX_CONTROL: usize = 64 * 1024;
pub const CHUNK: u32 = 4 * 1024 * 1024;
pub const MAX_META: usize = 16 * 1024 * 1024;
pub const MAX_HASHES: usize = 200_000; // ~781 GiB, explicit bounded manifest policy

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: u16,
    pub id: String,
    pub name: String,
    pub size: u64,
    pub chunk_size: u32,
    pub hashes: Vec<String>,
    pub whole_hash: String,
    pub file_identity: String,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == VERSION && self.chunk_size == CHUNK,
            "unsupported manifest version/chunk size"
        );
        ensure!(
            uuid::Uuid::parse_str(&self.id).is_ok_and(|id| id.to_string() == self.id),
            "invalid transfer id"
        );
        validate_name(&self.name)?;
        ensure!(
            !self.file_identity.is_empty() && self.file_identity.len() <= 128,
            "invalid source identity metadata"
        );
        ensure!(
            self.hashes.len() <= MAX_HASHES,
            "manifest exceeds supported file size"
        );
        let count = self.size / CHUNK as u64 + u64::from(self.size % CHUNK as u64 != 0);
        ensure!(count == self.hashes.len() as u64, "invalid chunk count");
        for hash in self.hashes.iter().chain(std::iter::once(&self.whole_hash)) {
            ensure!(
                hash.len() == 64 && hex::decode(hash)?.len() == 32,
                "invalid SHA-256"
            );
        }
        Ok(())
    }
    pub fn length(&self, idx: u32) -> Result<usize> {
        ensure!(
            (idx as usize) < self.hashes.len(),
            "chunk index out of range"
        );
        Ok((self.size - idx as u64 * CHUNK as u64).min(CHUNK as u64) as usize)
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= 160 && name != "." && name != "..",
        "invalid filename"
    );
    ensure!(
        !name.ends_with('.') && !name.ends_with(' ') && !name.ends_with(".localconnect.part"),
        "invalid filename suffix"
    );
    ensure!(
        !name
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c)),
        "unsafe filename"
    );
    let stem = name.split('.').next().unwrap().to_ascii_uppercase();
    ensure!(
        ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"
        ]
        .contains(&stem.as_str()),
        "reserved filename"
    );
    Ok(())
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Message {
    Hello {
        version: u16,
        name: String,
    },
    Pair {
        name: String,
    },
    Approved,
    Denied {
        reason: String,
    },
    Commit,
    Control,
    Ready,
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    Bye,
    Bulk {
        manifest: Manifest,
    },
    Missing {
        indexes: Vec<u32>,
    },
    Ack {
        index: u32,
        verified_bytes: u64,
    },
    Finish,
    Complete,
    Error {
        code: String,
        message: String,
    },
    RemoteRequest {
        request: String,
        input: bool,
    },
    RemoteApproved {
        session: String,
        geometry: crate::remote::Geometry,
        input: bool,
        platform: String,
    },
    RemoteDenied {
        reason: String,
    },
    RemoteInput {
        event: crate::remote::RemoteMessage,
        geometry: crate::remote::Geometry,
    },
    RemoteVideo {
        session: String,
    },
    VideoFrame {
        session: String,
        sequence: u64,
        timestamp_us: u64,
        codec: String,
        keyframe: bool,
        width: u32,
        height: u32,
        geometry: crate::remote::Geometry,
        bytes: u32,
    },
}
// Wire v2: u32 BE payload length; u8 kind; payload. Kinds are checked BEFORE
// allocation: 1 bounded control JSON, 2 chunk, 3 manifest JSON, 4 missing-list JSON.
// Raw chunk: u32 BE index then bytes. Allocation is bounded before reading.
pub async fn write_message<W: AsyncWrite + Unpin>(io: &mut W, msg: &Message) -> Result<()> {
    let data = serde_json::to_vec(msg)?;
    let (kind, limit) = match msg {
        Message::Bulk { .. } => (3, MAX_META),
        Message::Missing { .. } => (4, MAX_HASHES * 12 + 128),
        _ => (1, MAX_CONTROL),
    };
    ensure!(data.len() <= limit, "metadata too large for message type");
    io.write_u32((data.len() + 1) as u32).await?;
    io.write_u8(kind).await?;
    io.write_all(&data).await?;
    io.flush().await?;
    Ok(())
}
pub async fn read_message<R: AsyncRead + Unpin>(io: &mut R) -> Result<Message> {
    read_message_bounded(io, true).await
}
pub async fn read_message_bounded<R: AsyncRead + Unpin>(
    io: &mut R,
    trusted: bool,
) -> Result<Message> {
    let len = io.read_u32().await? as usize;
    ensure!(
        (2..=MAX_META + 1).contains(&len),
        "invalid metadata frame length"
    );
    let kind = io.read_u8().await?;
    let limit = match kind {
        1 => MAX_CONTROL,
        3 => MAX_META,
        4 => MAX_HASHES * 12 + 128,
        _ => bail!("unknown metadata message type"),
    };
    let limit = if trusted {
        limit
    } else {
        limit.min(MAX_CONTROL)
    };
    ensure!(len - 1 <= limit, "message type payload limit exceeded");
    let mut data = vec![0; len - 1];
    io.read_exact(&mut data).await?;
    let message: Message = serde_json::from_slice(&data)?;
    let actual = match message {
        Message::Bulk { .. } => 3,
        Message::Missing { .. } => 4,
        _ => 1,
    };
    ensure!(actual == kind, "message does not match declared frame type");
    Ok(message)
}
pub async fn write_chunk<W: AsyncWrite + Unpin>(io: &mut W, index: u32, data: &[u8]) -> Result<()> {
    ensure!(data.len() <= CHUNK as usize, "chunk too large");
    io.write_u32((data.len() + 5) as u32).await?;
    io.write_u8(2).await?;
    io.write_u32(index).await?;
    io.write_all(data).await?;
    io.flush().await?;
    Ok(())
}
pub async fn read_chunk<R: AsyncRead + Unpin>(io: &mut R, m: &Manifest) -> Result<(u32, Vec<u8>)> {
    let len = io.read_u32().await? as usize;
    ensure!(
        (5..=CHUNK as usize + 5).contains(&len),
        "invalid chunk frame length"
    );
    if io.read_u8().await? != 2 {
        bail!("unexpected chunk frame type")
    }
    let idx = io.read_u32().await?;
    ensure!(m.length(idx)? == len - 5, "invalid chunk length");
    let mut data = vec![0; len - 5];
    io.read_exact(&mut data).await?;
    Ok((idx, data))
}
pub async fn write_video<W: AsyncWrite + Unpin>(
    io: &mut W,
    sequence: u64,
    data: &[u8],
) -> Result<()> {
    ensure!(data.len() <= 8 * 1024 * 1024, "screen frame exceeds limit");
    io.write_u32((data.len() + 9) as u32).await?;
    io.write_u8(5).await?;
    io.write_u64(sequence).await?;
    io.write_all(data).await?;
    io.flush().await?;
    Ok(())
}
pub async fn read_video<R: AsyncRead + Unpin>(
    io: &mut R,
    sequence: u64,
    bytes: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    ensure!(
        width > 0
            && height > 0
            && width <= 1920
            && height <= 1920
            && width as u64 * height as u64 <= 1920 * 1080,
        "invalid encoded frame geometry"
    );
    let len = io.read_u32().await? as usize;
    ensure!(
        (33..=8 * 1024 * 1024 + 9).contains(&len) && len - 9 == bytes as usize,
        "invalid video payload length"
    );
    ensure!(
        io.read_u8().await? == 5 && io.read_u64().await? == sequence,
        "video sequence/type mismatch"
    );
    let mut data = vec![0; len - 9];
    io.read_exact(&mut data).await?;
    ensure!(
        data.starts_with(b"\x89PNG\r\n\x1a\n") && &data[12..16] == b"IHDR",
        "expected PNG frame"
    );
    ensure!(
        u32::from_be_bytes(data[16..20].try_into()?) == width
            && u32::from_be_bytes(data[20..24].try_into()?) == height,
        "PNG dimensions mismatch"
    );
    Ok(data)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths() {
        for x in ["../bad", "/tmp/x", "a\\b", "CON.txt", "x:", "..", "a."] {
            assert!(validate_name(x).is_err());
        }
        assert!(validate_name("photo.jpg").is_ok());
    }
    #[tokio::test]
    async fn oversized_frame() {
        let (mut a, mut b) = tokio::io::duplex(8);
        a.write_u32(u32::MAX).await.unwrap();
        assert!(read_message(&mut b).await.is_err());
    }
    #[tokio::test]
    async fn ping_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        write_message(&mut a, &Message::Ping { nonce: 42 })
            .await
            .unwrap();
        assert!(matches!(
            read_message(&mut b).await.unwrap(),
            Message::Ping { nonce: 42 }
        ));
    }
    #[tokio::test]
    async fn per_kind_limits_are_checked_before_payload_read() {
        for (kind, trusted) in [(1, true), (3, false), (255, true)] {
            let (mut a, mut b) = tokio::io::duplex(8);
            a.write_u32((MAX_CONTROL + 2) as u32).await.unwrap();
            a.write_u8(kind).await.unwrap();
            assert!(tokio::time::timeout(
                std::time::Duration::from_secs(1),
                read_message_bounded(&mut b, trusted)
            )
            .await
            .unwrap()
            .is_err());
        }
    }
    #[tokio::test]
    async fn declared_type_must_match_json_message() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let payload = serde_json::to_vec(&Message::Ping { nonce: 1 }).unwrap();
        a.write_u32((payload.len() + 1) as u32).await.unwrap();
        a.write_u8(3).await.unwrap();
        a.write_all(&payload).await.unwrap();
        assert!(read_message(&mut b).await.is_err());
    }
}
