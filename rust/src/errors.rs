use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailureInfo {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub action: String,
}
#[derive(Debug)]
pub struct PeerFailure(pub FailureInfo);
impl std::fmt::Display for PeerFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.0.code, self.0.message)
    }
}
impl std::error::Error for PeerFailure {}
pub fn from_error(error: &anyhow::Error) -> FailureInfo {
    if let Some(peer) = error.downcast_ref::<PeerFailure>() {
        return peer.0.clone();
    }
    let raw = format!("{error:#}");
    let message = raw.chars().take(4096).collect::<String>();
    let lower = message.to_lowercase();
    let mut code = "operation_failed";
    let mut retryable = false;
    let mut action = "Check the displayed error and retry explicitly.";
    for cause in error.chain() {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind;
            match io.kind() {
                ErrorKind::PermissionDenied => {
                    code = "permission_denied";
                    action = "Choose a writable folder or grant the required OS permission.";
                }
                ErrorKind::NotFound => {
                    code = "file_unavailable";
                    action = "Restore the source/folder or select a new file.";
                }
                ErrorKind::StorageFull => {
                    code = "disk_full";
                    action = "Free disk space, then resume the transfer.";
                }
                ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::NotConnected
                | ErrorKind::TimedOut
                | ErrorKind::BrokenPipe
                | ErrorKind::UnexpectedEof => {
                    code = "connection_lost";
                    retryable = true;
                    action = "Check Wi-Fi, firewall and peer availability; reconnect/resume.";
                }
                _ => {}
            }
        }
        if let Some(rusqlite::Error::SqliteFailure(sql, _)) =
            cause.downcast_ref::<rusqlite::Error>()
        {
            if sql.code == rusqlite::ErrorCode::DiskFull {
                code = "disk_full";
                action = "Free space in application storage, then resume.";
            }
        }
        if cause.is::<tokio::time::error::Elapsed>() {
            code = "connection_lost";
            retryable = true;
            action = "Peer did not respond; check LAN connectivity and retry.";
        }
    }
    if lower.contains("no space left") || lower.contains("disk full") || lower.contains("disk_full")
    {
        code = "disk_full";
        retryable = false;
        action = "Free disk space, then resume explicitly.";
    } else if lower.contains("permission denied") {
        code = "permission_denied";
        retryable = false;
        action = "Choose a writable folder or grant the required OS permission.";
    } else if lower.contains("quota") || lower.contains("configured receive limit") {
        code = "quota_exceeded";
        retryable = false;
        action = "Clean up incomplete receives or adjust Transfer limits.";
    } else if lower.contains("source changed") || lower.contains("source content changed") {
        code = "source_changed";
        retryable = false;
        action = "Select and send the changed file as a new transfer.";
    } else if lower.contains("hash mismatch") || lower.contains("corrupted chunk") {
        code = "integrity_failed";
        retryable = true;
        action = "Resume to re-request invalid chunks; investigate persistent corruption.";
    } else if lower.contains("unpaired")
        || lower.contains("trust revoked")
        || lower.contains("not paired")
    {
        code = "unpaired";
        retryable = false;
        action = "Verify fingerprints and pair explicitly on both devices.";
    } else if lower.contains("certificate changed") || lower.contains("certificate mismatch") {
        code = "identity_changed";
        retryable = false;
        action = "Stop and independently verify the new fingerprint before pairing.";
    } else if lower.contains("unsafe filename")
        || lower.contains("storage path changed")
        || lower.contains("regular file")
        || lower.contains("destination exists")
    {
        code = "invalid_path";
        retryable = false;
        action = "Use the original recovery folder or choose another safe destination/name.";
    } else if lower.contains("cancelled") || lower == "stopped" {
        code = "cancelled";
        retryable = false;
        action = "Resume explicitly if you want to continue.";
    } else if lower.contains("not approved")
        || lower.contains("denied")
        || lower.contains("consent expired")
    {
        code = "not_approved";
        retryable = false;
        action = "Request again only when the controlled device's user can approve.";
    }
    FailureInfo {
        code: code.into(),
        message,
        retryable,
        action: action.into(),
    }
}
pub fn remote(code: String, message: String) -> anyhow::Error {
    let error = anyhow::anyhow!(message.clone());
    let mut info = from_error(&error);
    info.code = code;
    info.retryable = matches!(
        info.code.as_str(),
        "connection_lost" | "receiver_busy" | "integrity_failed"
    );
    PeerFailure(info).into()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_and_connection_retry_policy() {
        let disk = anyhow::Error::new(std::io::Error::from_raw_os_error(if cfg!(windows) {
            112
        } else {
            28
        }));
        assert_eq!(from_error(&disk).code, "disk_full");
        assert!(!from_error(&disk).retryable);
        let connection = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "lost",
        ));
        assert!(from_error(&connection).retryable);
        assert!(!from_error(&anyhow::anyhow!("source changed")).retryable);
    }
}
