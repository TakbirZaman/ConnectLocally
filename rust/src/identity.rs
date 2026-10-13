//! Pairing bootstrap checks TLS proof-of-private-key possession, but accepts an
//! unknown certificate. Application commands are gated on explicit fingerprint
//! approval. This verifier MUST NOT be reused for general HTTPS clients.
use anyhow::{ensure, Result};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    DigitallySignedStruct, DistinguishedName, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
#[derive(Clone)]
pub struct Identity {
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
    pub fingerprint: String,
}
pub fn fingerprint(cert: &[u8]) -> String {
    hex::encode(Sha256::digest(cert))
}
impl Identity {
    pub fn load(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let cp = dir.join("identity.der");
        let kp = dir.join("identity.key");
        #[cfg(target_os = "windows")]
        let vault = keyring::Entry::new(
            "LocalConnect TLS identity",
            &hex::encode(Sha256::digest(
                std::fs::canonicalize(dir)?.to_string_lossy().as_bytes(),
            )),
        )?;
        #[cfg(target_os = "windows")]
        let protected = match vault.get_secret() {
            Ok(key) => Some(key),
            Err(keyring::Error::NoEntry) => None,
            Err(error) => return Err(error.into()),
        };
        #[cfg(not(target_os = "windows"))]
        let protected: Option<Vec<u8>> = None;

        if !cp.exists() && !kp.exists() && protected.is_none() {
            let rcgen::CertifiedKey { cert, key_pair } =
                rcgen::generate_simple_self_signed(vec!["localconnect.local".into()])?;
            // Create key first; interruption fails closed rather than regenerating identity.
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            use std::io::Write;
            #[cfg(target_os = "windows")]
            vault.set_secret(&key_pair.serialize_der())?;
            #[cfg(not(target_os = "windows"))]
            {
                let mut k = opts.open(&kp)?;
                k.write_all(&key_pair.serialize_der())?;
                k.sync_all()?;
            }

            let mut c = opts.open(&cp)?;
            c.write_all(cert.der().as_ref())?;
            c.sync_all()?;
        }
        ensure!(
            cp.exists() && (kp.exists() || protected.is_some() || cfg!(target_os = "windows")),
            "incomplete identity: restore backup or reset explicitly"
        );
        let cert = std::fs::read(cp)?;
        #[cfg(target_os = "windows")]
        let key = match protected {
            Some(key) => key,
            None => match vault.get_secret() {
                Ok(key) => key,
                Err(keyring::Error::NoEntry) => {
                    let key = std::fs::read(&kp)?;
                    vault.set_secret(&key)?;
                    key
                }
                Err(error) => return Err(error.into()),
            },
        };
        #[cfg(target_os = "windows")]
        if kp.exists() {
            std::fs::remove_file(&kp)?;
        }
        #[cfg(not(target_os = "windows"))]
        let key = std::fs::read(kp)?;

        Ok(Self {
            fingerprint: fingerprint(&cert),
            cert,
            key,
        })
    }
    pub fn configs(&self) -> Result<(Arc<rustls::ServerConfig>, Arc<rustls::ClientConfig>)> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let certs = vec![CertificateDer::from(self.cert.clone())];
        let key =
            || rustls::pki_types::PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone()));
        let mut server = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(Arc::new(PairVerifier))
            .with_single_cert(certs.clone(), key())?;
        let mut client = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PairVerifier))
            .with_client_auth_cert(certs, key())?;
        server.alpn_protocols = vec![b"localconnect/2".to_vec()];
        client.alpn_protocols = server.alpn_protocols.clone();
        Ok((Arc::new(server), Arc::new(client)))
    }
}
#[derive(Debug)]
struct PairVerifier;
fn algorithms() -> rustls::crypto::WebPkiSupportedAlgorithms {
    rustls::crypto::ring::default_provider().signature_verification_algorithms
}
impl ServerCertVerifier for PairVerifier {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &algorithms())
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &algorithms())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        algorithms().supported_schemes()
    }
}
impl ClientCertVerifier for PairVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        ServerCertVerifier::verify_tls12_signature(self, m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        ServerCertVerifier::verify_tls13_signature(self, m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        algorithms().supported_schemes()
    }
}
