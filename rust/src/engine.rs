use crate::{
    identity::{fingerprint, Identity},
    protocol::*,
    store::Store,
    transfer::{self, Receiver},
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast, oneshot, Mutex, Semaphore},
    task::JoinHandle,
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_util::sync::CancellationToken;
const DEADLINE: Duration = Duration::from_secs(120);
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub kind: String,
    pub payload: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Peer {
    pub fingerprint: String,
    pub name: String,
    pub address: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub data_dir: String,
    pub receive_dir: String,
    pub name: String,
    pub port: u16,
    pub in_flight: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferLimits {
    pub in_flight: u32,
    pub max_file_bytes: u64,
    pub max_reserved_bytes: u64,
}
impl TransferLimits {
    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=8).contains(&self.in_flight),
            "chunk window must be 1–8"
        );
        ensure!(
            self.max_file_bytes > 0 && self.max_file_bytes <= self.max_reserved_bytes,
            "file limit must be positive and no greater than total receive quota"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferProgress {
    pub id: String,
    pub direction: String,
    pub name: String,
    pub status: String,
    pub peer: String,
    pub verified_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<crate::errors::FailureInfo>,
}
pub struct Engine {
    pub(crate) remote: Arc<crate::remote_engine::RemoteState>,
    pub identity: Identity,
    pub config: Config,
    pub store: Store,
    pub port: u16,
    pub events: broadcast::Sender<Event>,
    discovered: std::sync::Mutex<HashMap<String, Event>>,
    connection_states: std::sync::Mutex<HashMap<String, Event>>,
    limits: std::sync::Mutex<TransferLimits>,
    configure_gate: Mutex<()>,
    stop: CancellationToken,
    client: Arc<rustls::ClientConfig>,
    server: Arc<rustls::ServerConfig>,
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    pub(crate) codes: Mutex<crate::pairing::CodeGate>,
    session_trust: std::sync::Mutex<HashMap<String, String>>,
    pub(crate) tasks: Mutex<Vec<JoinHandle<()>>>,
    transfers: Mutex<HashMap<String, (String, CancellationToken)>>,
    controls: Mutex<HashMap<String, CancellationToken>>,
    receive_gate: Arc<Semaphore>,
    send_gate: Arc<Semaphore>,
    mdns: mdns_sd::ServiceDaemon,
}
fn lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(x) => x.is_private() || x.is_loopback() || x.is_link_local(),
        IpAddr::V6(x) => {
            x.is_loopback()
                || (x.segments()[0] & 0xfe00) == 0xfc00
                || (x.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}
fn address(s: &str) -> Result<SocketAddr> {
    let a: SocketAddr = s
        .parse()
        .context("enter a numeric LAN IP:port (IPv6: [address%scope]:port)")?;
    ensure!(
        lan(a.ip()) && a.port() != 0,
        "only private, link-local or loopback addresses allowed"
    );
    Ok(a)
}
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f).await?
}
async fn read_deadline<S: tokio::io::AsyncRead + Unpin>(s: &mut S) -> Result<Message> {
    timeout(DEADLINE, read_message(s))
        .await
        .context("peer response deadline exceeded")?
}
impl Engine {
    pub async fn start(config: Config) -> Result<Arc<Self>> {
        ensure!(
            !config.name.is_empty() && config.name.len() <= 64,
            "display name must be 1–64 bytes"
        );
        ensure!(
            (1..=8).contains(&config.in_flight),
            "in-flight chunk window must be between 1 and 8"
        );
        let cfg = config.clone();
        let (identity, store, limits) = blocking(move || {
            std::fs::create_dir_all(&cfg.receive_dir)?;
            let i = Identity::load(&PathBuf::from(&cfg.data_dir))?;
            let s = Store::open(&PathBuf::from(&cfg.data_dir).join("state.sqlite"))?;
            let limits = s.limits()?.unwrap_or(TransferLimits {
                in_flight: cfg.in_flight,
                max_file_bytes: 64 * 1024 * 1024 * 1024,
                max_reserved_bytes: 128 * 1024 * 1024 * 1024,
            });
            limits.validate()?;
            Ok((i, s, limits))
        })
        .await?;
        let (server, client) = identity.configs()?;
        let listener = TcpListener::bind(("0.0.0.0", config.port)).await?;
        let port = listener.local_addr()?.port();
        let ipv6 = (|| -> Result<TcpListener> {
            let socket = socket2::Socket::new(
                socket2::Domain::IPV6,
                socket2::Type::STREAM,
                Some(socket2::Protocol::TCP),
            )?;
            socket.set_only_v6(true)?;
            socket.set_reuse_address(true)?;
            socket.set_nonblocking(true)?;
            socket.bind(&std::net::SocketAddr::from(([0u16; 8], port)).into())?;
            socket.listen(16)?;
            Ok(TcpListener::from_std(socket.into())?)
        })();

        let mdns = mdns_sd::ServiceDaemon::new()?;
        let (events, _) = broadcast::channel(256);
        let e = Arc::new(Self {
            remote: Arc::new(crate::remote_engine::RemoteState::default()),
            identity,
            config,
            store,
            port,
            events,
            discovered: std::sync::Mutex::new(HashMap::new()),
            connection_states: std::sync::Mutex::new(HashMap::new()),
            limits: std::sync::Mutex::new(limits),
            configure_gate: Mutex::new(()),
            stop: CancellationToken::new(),
            client,
            server,
            pending: Mutex::new(HashMap::new()),
            codes: Mutex::new(crate::pairing::CodeGate::default()),
            session_trust: std::sync::Mutex::new(HashMap::new()),
            tasks: Mutex::new(vec![]),
            transfers: Mutex::new(HashMap::new()),
            controls: Mutex::new(HashMap::new()),
            receive_gate: Arc::new(Semaphore::new(1)),
            send_gate: Arc::new(Semaphore::new(1)),
            mdns,
        });
        let engine = e.clone();
        e.tasks.lock().await.push(tokio::spawn(async move {
            engine.listen(listener).await;
        }));
        match ipv6 {
            Ok(listener) => {
                let engine = e.clone();
                e.tasks.lock().await.push(tokio::spawn(async move {
                    engine.listen(listener).await;
                }));
            }
            Err(error) => e.emit(
                "capability",
                serde_json::json!({"ipv6Listening":false,"reason":error.to_string()}),
            ),
        }
        if let Err(error) = e.discovery().await {
            e.shutdown().await;
            return Err(error);
        }
        e.emit(
            "started",
            serde_json::json!({"fingerprint":e.identity.fingerprint,"port":port}),
        );
        Ok(e)
    }
    pub fn emit(&self, kind: &str, payload: serde_json::Value) {
        if kind == "connection" {
            if let Some(peer) = payload["peer"].as_str() {
                let mut states = self
                    .connection_states
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if states.len() < 256 || states.contains_key(peer) {
                    states.insert(
                        peer.into(),
                        Event {
                            kind: kind.into(),
                            payload: payload.to_string(),
                        },
                    );
                }
            }
        }

        let _ = self.events.send(Event {
            kind: kind.into(),
            payload: payload.to_string(),
        });
    }
    pub(crate) async fn trusted(&self, p: &str) -> Result<bool> {
        if self.session_trust.lock().unwrap_or_else(|error| error.into_inner()).contains_key(p) {
            return Ok(true);
        }
        let s = self.store.clone();
        let p = p.to_string();
        blocking(move || s.trusted(&p)).await
    }
    pub async fn peers(&self) -> Result<String> {
        let s = self.store.clone();
        let mut rows = blocking(move || s.peers()).await?;
        {
            let session = self.session_trust.lock().unwrap_or_else(|error| error.into_inner());
            for (fingerprint, name) in session.iter() {
                if !rows.iter().any(|(peer, _)| peer == fingerprint) {
                    rows.push((fingerprint.clone(), name.clone()));
                }
            }
        }
        Ok(serde_json::to_string(&rows)?)
    }
    pub async fn begin_pairing_code(&self, remember: bool) -> Result<String> {
        ensure!(!self.stop.is_cancelled(), "engine stopped");
        Ok(serde_json::to_string(&self.codes.lock().await.begin(remember)?)?)
    }
    pub async fn cancel_pairing_code(&self) {
        self.codes.lock().await.clear();
    }
    async fn remember_choice(&self, peer: &str, name: &str, remember: bool) -> Result<()> {
        if remember {
            self.remember(peer, name).await?;
            self.session_trust.lock().unwrap_or_else(|error| error.into_inner()).remove(peer);
        } else {
            let store = self.store.clone();
            let fingerprint = peer.to_owned();
            blocking(move || store.forget(&fingerprint)).await?;
            self.session_trust.lock().unwrap_or_else(|error| error.into_inner())
                .insert(peer.to_owned(), name.to_owned());
        }
        Ok(())
    }
    pub async fn history(&self) -> Result<String> {
        let s = self.store.clone();
        blocking(move || s.history()).await
    }
    async fn remember(&self, p: &str, n: &str) -> Result<()> {
        let s = self.store.clone();
        let p = p.to_owned();
        let n = n.to_owned();
        blocking(move || s.trust(&p, &n)).await
    }
    pub async fn forget(&self, p: String) -> Result<()> {
        let store = self.store.clone();
        let fingerprint = p.clone();
        blocking(move || store.forget(&fingerprint)).await?;
        self.session_trust.lock().unwrap_or_else(|error| error.into_inner()).remove(&p);
        if let Some(c) = self.controls.lock().await.remove(&p) {
            c.cancel();
        }
        for (peer, token) in self.transfers.lock().await.values() {
            if peer == &p {
                token.cancel();
            }
        }
        for completion in self.remote.revoke(&p) {
            crate::remote_engine::wait_input_done(completion).await?;
        }
        Ok(())
    }
    pub async fn approve(&self, p: String, allow: bool) -> Result<()> {
        let tx = self
            .pending
            .lock()
            .await
            .remove(&p)
            .context("pairing expired")?;
        let _ = tx.send(allow);
        Ok(())
    }
    pub(crate) async fn connect(
        &self,
        addr: &str,
        expected: Option<&str>,
    ) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, Peer)> {
        let original = match self.connect_endpoint(addr, expected).await {
            Ok(connection) => return Ok(connection),
            Err(error) => error,
        };
        let Some(expected) = expected else {
            return Err(original);
        };
        // mDNS only suggests endpoints. Each alternative must prove the already-approved certificate.
        let candidates = {
            let discovered = self.discovered.lock().unwrap_or_else(|e| e.into_inner());
            discovered
                .values()
                .filter_map(|event| serde_json::from_str::<serde_json::Value>(&event.payload).ok())
                .filter(|data| data["id"].as_str() == Some(expected))
                .flat_map(|data| data["addresses"].as_array().cloned().unwrap_or_default())
                .filter_map(|value| value.as_str().map(str::to_owned))
                .filter(|candidate| candidate != addr)
                .take(8)
                .collect::<Vec<_>>()
        };
        for candidate in candidates {
            if let Ok(connection) = self.connect_endpoint(&candidate, Some(expected)).await {
                return Ok(connection);
            }
        }
        Err(original)
    }
    async fn connect_endpoint(
        &self,
        addr: &str,
        expected: Option<&str>,
    ) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, Peer)> {
        let tcp = timeout(Duration::from_secs(10), TcpStream::connect(address(addr)?)).await??;
        tcp.set_nodelay(true)?;
        let mut stream = timeout(
            Duration::from_secs(10),
            TlsConnector::from(self.client.clone()).connect(
                rustls::pki_types::ServerName::try_from("localconnect.local")?.to_owned(),
                tcp,
            ),
        )
        .await??;
        let fp = fingerprint(
            stream
                .get_ref()
                .1
                .peer_certificates()
                .context("server certificate missing")?
                .first()
                .context("empty certificate chain")?
                .as_ref(),
        );
        if let Some(expected) = expected {
            ensure!(fp == expected, "certificate changed: verify and pair again");
        }
        let name = match read_deadline(&mut stream).await? {
            Message::Hello { version, name } => {
                ensure!(
                    version == WIRE_VERSION && name.len() <= 64,
                    "unsupported peer"
                );
                name
            }
            _ => bail!("missing hello"),
        };
        Ok((
            stream,
            Peer {
                fingerprint: fp,
                name,
                address: addr.into(),
            },
        ))
    }
    pub async fn probe(&self, addr: String) -> Result<Peer> {
        tokio::select! {
            biased;
            _ = self.stop.cancelled() => bail!("cancelled: engine stopped"),
            result = async {
                let (mut s, p) = self.connect(&addr, None).await?;
                write_message(&mut s, &Message::Bye).await?;
                Ok(p)
            } => result,
        }
    }
    pub async fn pair_with_code(
        self: &Arc<Self>,
        addr: String,
        code: String,
        remember: bool,
    ) -> Result<Peer> {
        tokio::select! {
            biased;
            _ = self.stop.cancelled() => bail!("cancelled: engine stopped"),
            result = self.pair_code_inner(addr, code, remember) => result,
        }
    }
    async fn pair_code_inner(
        self: &Arc<Self>,
        addr: String,
        code: String,
        remember: bool,
    ) -> Result<Peer> {
        crate::pairing::validate_code(&code)?;
        let (mut stream, peer) = self.connect(&addr, None).await?;
        let mut channel = [0u8; 32];
        stream.get_ref().1.export_keying_material(
            &mut channel, b"localconnect/code/v1", None,
        )?;
        let (state, exchange) = crate::pairing::start_a(
            &code, &self.identity.fingerprint, &peer.fingerprint,
        )?;
        write_message(&mut stream, &Message::CodePair {
            name: self.config.name.clone(),
            port: self.port,
            exchange,
        }).await?;
        let exchange = match read_deadline(&mut stream).await? {
            Message::CodeExchange { exchange } => exchange,
            Message::Denied { reason } => bail!("pairing unavailable: {reason}"),
            _ => bail!("Code pairing requires updated LocalConnect on both devices"),
        };
        let key = crate::pairing::finish(state, &exchange)?;
        write_message(&mut stream, &Message::CodeProof {
            proof: crate::pairing::proof(&key, &channel, "client")?,
        }).await?;
        match read_deadline(&mut stream).await? {
            Message::CodeProof { proof } => crate::pairing::verify(
                &key, &channel, "server", &proof,
            )?,
            _ => bail!("Pairing code failed or expired"),
        }
        write_message(&mut stream, &Message::Commit).await?;
        ensure!(
            matches!(read_deadline(&mut stream).await?, Message::Ready),
            "pairing commit failed"
        );
        self.remember_choice(&peer.fingerprint, &peer.name, remember).await?;
        self.start_control(peer.clone()).await?;
        self.emit("paired", serde_json::json!({
            "fingerprint": peer.fingerprint, "name": peer.name,
            "address": peer.address, "sessionOnly": !remember,
        }));
        Ok(peer)
    }

    pub async fn pair(self: &Arc<Self>, addr: String, expected: String) -> Result<Peer> {
        tokio::select! {
            biased;
            _ = self.stop.cancelled() => bail!("cancelled: engine stopped"),
            result = self.pair_inner(addr, expected) => result,
        }
    }
    async fn pair_inner(self: &Arc<Self>, addr: String, expected: String) -> Result<Peer> {
        self.emit(
            "connection",
            serde_json::json!({"state":"pairing","peer":expected}),
        );
        let (mut s, p) = self.connect(&addr, Some(&expected)).await?;
        write_message(
            &mut s,
            &Message::Pair {
                name: self.config.name.clone(),
            },
        )
        .await?;
        match read_deadline(&mut s).await? {
            Message::Approved => {}
            Message::Denied { reason } => bail!("pairing denied: {reason}"),
            _ => bail!("pairing protocol error"),
        }
        // This command is invoked only AFTER the initiating UI confirms its fingerprint.
        write_message(&mut s, &Message::Commit).await?;
        ensure!(
            matches!(read_deadline(&mut s).await?, Message::Ready),
            "pairing commit failed"
        );
        self.remember(&p.fingerprint, &p.name).await?;
        self.start_control(p.clone()).await?;
        Ok(p)
    }
    pub async fn start_control(self: &Arc<Self>, p: Peer) -> Result<()> {
        ensure!(self.trusted(&p.fingerprint).await?, "peer is not paired");
        let token = self.stop.child_token();
        if let Some(old) = self
            .controls
            .lock()
            .await
            .insert(p.fingerprint.clone(), token.clone())
        {
            old.cancel();
        }
        let e = self.clone();
        self.tasks.lock().await.push(tokio::spawn(async move{
   let mut attempt=0u32;
   loop {
    if token.is_cancelled(){break}
    e.emit("connection",serde_json::json!({"peer":p.fingerprint,"state":if attempt==0{"connecting"}else{"reconnecting"}}));
    let result=tokio::select!{_ = token.cancelled()=>break,r=e.control_once(&p)=>r};
    if token.is_cancelled(){break}
    e.emit("error",serde_json::json!({"code":"connection_lost","peer":p.fingerprint,"message":format!("{result:?}")}));
    attempt+=1; if attempt>6{e.emit("connection",serde_json::json!({"peer":p.fingerprint,"state":"failed"}));break}
    let delay=(1u64<<attempt.min(5))*500+rand::random::<u16>() as u64%500;
    tokio::select!{_ = token.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(delay))=>{}}
   }
   e.emit("connection",serde_json::json!({"peer":p.fingerprint,"state":"disconnected"}));
  }));
        Ok(())
    }
    async fn control_once(&self, p: &Peer) -> Result<()> {
        ensure!(self.trusted(&p.fingerprint).await?, "trust revoked");
        let (mut s, _) = self.connect(&p.address, Some(&p.fingerprint)).await?;
        write_message(&mut s, &Message::Control).await?;
        ensure!(
            matches!(read_deadline(&mut s).await?, Message::Ready),
            "control rejected"
        );
        self.emit(
            "connection",
            serde_json::json!({"peer":p.fingerprint,"state":"connected"}),
        );
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            ensure!(self.trusted(&p.fingerprint).await?, "trust revoked");
            let nonce = rand::random();
            write_message(&mut s, &Message::Ping { nonce }).await?;
            ensure!(
                matches!(timeout(Duration::from_secs(10),read_message(&mut s)).await??,Message::Pong{nonce:n} if n==nonce),
                "heartbeat failed"
            );
        }
    }
    async fn listen(self: Arc<Self>, listener: TcpListener) {
        let gate = Arc::new(Semaphore::new(16));
        loop {
            let accepted = tokio::select! {_=self.stop.cancelled()=>break,r=listener.accept()=>r};
            let Ok((tcp, addr)) = accepted else { continue };
            if !lan(addr.ip()) {
                continue;
            }
            let Ok(permit) = gate.clone().try_acquire_owned() else {
                continue;
            };
            self.tasks.lock().await.retain(|task| !task.is_finished());
            let e = self.clone();
            let stop = self.stop.clone();
            self.tasks.lock().await.push(tokio::spawn(async move{let _permit=permit;
    tokio::select!{_=stop.cancelled()=>{},r=e.serve(tcp)=>{if let Err(error)=r{e.emit("error",serde_json::json!({"code":"incoming_connection","message":error.to_string()}));}}}
   }));
        }
    }
    async fn serve(self: &Arc<Self>, tcp: TcpStream) -> Result<()> {
        ensure!(lan(tcp.peer_addr()?.ip()), "only local-network peers allowed");
        tcp.set_nodelay(true)?;
        let mut s = timeout(
            Duration::from_secs(10),
            TlsAcceptor::from(self.server.clone()).accept(tcp),
        )
        .await??;
        let fp = fingerprint(
            s.get_ref()
                .1
                .peer_certificates()
                .context("client certificate required")?
                .first()
                .context("empty certificate")?
                .as_ref(),
        );
        write_message(
            &mut s,
            &Message::Hello {
                version: WIRE_VERSION,
                name: self.config.name.clone(),
            },
        )
        .await?;
        let known_peer = self.trusted(&fp).await?;
        let first = timeout(DEADLINE, read_message_bounded(&mut s, known_peer)).await??;
        if let Message::CodePair { name, port, exchange } = &first {
            ensure!(name.len() <= 64 && *port > 0, "invalid code pairing metadata");
            let ticket = match self.codes.lock().await.attempt() {
                Ok(ticket) => ticket,
                Err(_) => {
                    write_message(&mut s, &Message::Denied {
                        reason: "Generate a new code on this device; codes expire after two minutes and three attempts".into(),
                    }).await?;
                    return Ok(());
                }
            };
            let mut channel = [0u8; 32];
            s.get_ref().1.export_keying_material(
                &mut channel, b"localconnect/code/v1", None,
            )?;
            let (state, reply) = crate::pairing::start_b(
                &ticket.code, &fp, &self.identity.fingerprint,
            )?;
            let key = crate::pairing::finish(state, exchange)?;
            write_message(&mut s, &Message::CodeExchange { exchange: reply }).await?;
            let proof = match timeout(Duration::from_secs(15), read_message_bounded(&mut s, false)).await?? {
                Message::CodeProof { proof } => proof,
                _ => bail!("pairing confirmation required"),
            };
            crate::pairing::verify(&key, &channel, "client", &proof)?;
            self.codes.lock().await.consume(&ticket.id)?;
            write_message(&mut s, &Message::CodeProof {
                proof: crate::pairing::proof(&key, &channel, "server")?,
            }).await?;
            ensure!(
                matches!(timeout(Duration::from_secs(15), read_message_bounded(&mut s, false)).await??, Message::Commit),
                "pairing commit required"
            );
            self.remember_choice(&fp, name, ticket.remember).await?;
            write_message(&mut s, &Message::Ready).await?;
            let mut peer_address = s.get_ref().0.peer_addr()?;
            peer_address.set_port(*port);
            self.emit("paired", serde_json::json!({
                "fingerprint": fp, "name": name,
                "address": peer_address.to_string(),
                "sessionOnly": !ticket.remember, "codeWindow": ticket.id,
            }));
            return Ok(());
        }
        if let Message::Pair { name } = first {
            ensure!(name.len() <= 64, "name too long");
            let (tx, rx) = oneshot::channel();
            {
                let mut pending = self.pending.lock().await;
                ensure!(!pending.contains_key(&fp), "pair request already pending");
                pending.insert(fp.clone(), tx);
            }
            self.emit(
                "pair_request",
                serde_json::json!({"fingerprint":fp,"name":name}),
            );
            let allowed = matches!(timeout(Duration::from_secs(90), rx).await, Ok(Ok(true)));
            self.pending.lock().await.remove(&fp);
            if !allowed {
                write_message(
                    &mut s,
                    &Message::Denied {
                        reason: "not approved or expired".into(),
                    },
                )
                .await?;
                return Ok(());
            }
            write_message(&mut s, &Message::Approved).await?;
            ensure!(
                matches!(
                    timeout(Duration::from_secs(15), read_message(&mut s)).await??,
                    Message::Commit
                ),
                "pair commit required"
            );
            self.remember(&fp, &name).await?;
            write_message(&mut s, &Message::Ready).await?;
            self.emit("paired", serde_json::json!({"fingerprint":fp,"name":name}));
            return Ok(());
        }
        if matches!(first, Message::Bye) {
            return Ok(());
        }
        if !self.trusted(&fp).await? {
            write_message(
                &mut s,
                &Message::Error {
                    code: "unpaired".into(),
                    message: "Pair with a new code on both devices before transferring".into(),
                },
            )
            .await?;
            return Ok(());
        }
        match first {
            message @ (Message::RemoteRequest { .. } | Message::RemoteVideo { .. }) => {
                return self.serve_remote(s, fp, message).await;
            }
            Message::Control => {
                write_message(&mut s, &Message::Ready).await?;
                loop {
                    ensure!(self.trusted(&fp).await?, "trust revoked");
                    match timeout(Duration::from_secs(35), read_message(&mut s)).await?? {
                        Message::Ping { nonce } => {
                            write_message(&mut s, &Message::Pong { nonce }).await?
                        }
                        Message::Bye => break,
                        _ => bail!("invalid control message"),
                    }
                }
            }
            Message::Bulk { manifest } => {
                let _permit = self
                    .receive_gate
                    .clone()
                    .try_acquire_owned()
                    .context("receiver busy: retry later")?;
                let id = manifest.id.clone();
                let token = self.stop.child_token();
                self.transfers
                    .lock()
                    .await
                    .insert(id.clone(), (fp.clone(), token.clone()));
                let result = tokio::select! {_=token.cancelled()=>Err(anyhow::anyhow!("cancelled")),r=self.receive(&mut s,manifest,&fp)=>r};
                self.transfers.lock().await.remove(&id);
                if let Err(ref error) = result {
                    let store = self.store.clone();
                    let eid = id.clone();
                    let failure = crate::errors::from_error(error);
                    let message = failure.message.clone();
                    let em = message.clone();
                    let _ = blocking(move || {
                        let status = store.transfer_status(&eid)?;
                        if status != "finishing" && status != "complete" {
                            store.status(&eid, "recoverable_error", Some(&em))?;
                        }
                        Ok(())
                    })
                    .await;
                    self.emit("progress",serde_json::json!({"id":id,"direction":"receive","status":"recoverable_error","error":message}));
                    self.emit(
                        "error",
                        serde_json::json!({"code":failure.code,"id":id,"message":message,"action":failure.action,"retryable":failure.retryable}),
                    );
                    let _ = write_message(
                        &mut s,
                        &Message::Error {
                            code: failure.code,
                            message,
                        },
                    )
                    .await;
                }
                result?;
            }
            _ => bail!("unsupported message; remote control is unavailable"),
        }
        Ok(())
    }
    async fn receive<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
        &self,
        s: &mut S,
        m: Manifest,
        fp: &str,
    ) -> Result<()> {
        m.validate()?;
        let limits = self.transfer_limits();
        ensure!(
            m.size <= limits.max_file_bytes,
            "file exceeds configured receive limit"
        );
        let quota_store = self.store.clone();
        let quota_id = m.id.clone();
        let (reserved, count) =
            blocking(move || quota_store.receive_reservations(&quota_id)).await?;
        ensure!(
            count < 128,
            "too many incomplete receives; clean up transfer history"
        );
        ensure!(
            reserved
                .checked_add(m.size)
                .is_some_and(|n| n <= limits.max_reserved_bytes),
            "receive quota exceeded; clean up incomplete transfers or increase the limit"
        );
        let dir = PathBuf::from(&self.config.receive_dir);
        let store = self.store.clone();
        let peer = fp.to_string();
        let manifest = m.clone();
        let receiver =
            Arc::new(blocking(move || Receiver::prepare(&dir, manifest, store, &peer)).await?);
        let r = receiver.clone();
        let missing = blocking(move || r.reconcile()).await?;
        write_message(
            s,
            &Message::Missing {
                indexes: missing.clone(),
            },
        )
        .await?;
        let started = std::time::Instant::now();
        let baseline = {
            let r = receiver.clone();
            blocking(move || r.verified_bytes()).await?
        };
        self.emit("progress",serde_json::json!({"id":m.id,"direction":"receive","name":receiver.destination.to_string_lossy(),"verifiedBytes":baseline,"totalBytes":m.size,"status":"receiving"}));
        let mut last = std::time::Instant::now() - Duration::from_secs(1);
        for expected in missing {
            ensure!(self.trusted(fp).await?, "trust revoked");
            let (index, data) = timeout(DEADLINE, read_chunk(s, &m)).await??;
            ensure!(index == expected, "unexpected chunk order");
            let r = receiver.clone();
            let bytes = blocking(move || r.accept(index, &data)).await?;
            write_message(
                s,
                &Message::Ack {
                    index,
                    verified_bytes: bytes,
                },
            )
            .await?;
            if last.elapsed() >= Duration::from_millis(200) {
                self.emit("progress",serde_json::json!({"id":m.id,"direction":"receive","verifiedBytes":bytes,"totalBytes":m.size,"bytesPerSecond":(bytes-baseline) as f64/started.elapsed().as_secs_f64().max(0.001),"status":"receiving"}));
                last = std::time::Instant::now();
            }
        }
        ensure!(
            matches!(read_deadline(s).await?, Message::Finish),
            "finish expected"
        );
        let r = receiver.clone();
        blocking(move || r.finish()).await?;
        write_message(s, &Message::Complete).await?;
        self.emit("progress",serde_json::json!({"id":m.id,"verifiedBytes":m.size,"totalBytes":m.size,"status":"complete"}));
        Ok(())
    }
    pub async fn send(self: &Arc<Self>, path: String, p: Peer) -> Result<String> {
        ensure!(self.trusted(&p.fingerprint).await?, "pair before sending");
        let permit = self
            .send_gate
            .clone()
            .try_acquire_owned()
            .context("one outgoing transfer at a time")?;
        let id = uuid::Uuid::new_v4().to_string();
        let i = id.clone();
        let source = path.clone();
        let cancel = self.cancellation();
        let m = blocking(move || {
            transfer::manifest_cancellable(&PathBuf::from(source), i, Some(&cancel))
        })
        .await?;
        let store = self.store.clone();
        let mc = m.clone();
        let pc = p.clone();
        let sp = path.clone();
        blocking(move || {
            store.register(
                &mc,
                &pc.fingerprint,
                "send",
                Some(&sp),
                &format!("{}:{}", pc.fingerprint, mc.id),
                Some(&pc.address),
            )
        })
        .await?;
        self.launch_send(m, p, path, permit).await?;
        Ok(id)
    }
    pub async fn relocate(&self, id: String, address: String) -> Result<()> {
        let store = self.store.clone();
        let tid = id.clone();
        let (_, fp, _, _) = blocking(move || store.outbound(&tid)).await?;
        let p = self.probe(address.clone()).await?;
        ensure!(
            p.fingerprint == fp,
            "certificate changed: verify and pair again"
        );
        ensure!(self.trusted(&p.fingerprint).await?, "trust revoked");
        let store = self.store.clone();
        blocking(move || store.relocate(&id, &address)).await
    }
    pub async fn resume(self: &Arc<Self>, id: String) -> Result<()> {
        let store = self.store.clone();
        let (m, fp, address, path) = blocking(move || store.outbound(&id)).await?;
        let permit = self
            .send_gate
            .clone()
            .try_acquire_owned()
            .context("one outgoing transfer at a time")?;
        self.launch_send(
            m,
            Peer {
                fingerprint: fp,
                name: "trusted peer".into(),
                address,
            },
            path,
            permit,
        )
        .await
    }
    async fn launch_send(
        self: &Arc<Self>,
        m: Manifest,
        p: Peer,
        path: String,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<()> {
        let id = m.id.clone();
        self.emit("progress",serde_json::json!({"id":id,"direction":"send","name":m.name,"verifiedBytes":0,"totalBytes":m.size,"status":"sending"}));
        let token = self.stop.child_token();
        self.transfers
            .lock()
            .await
            .insert(id.clone(), (p.fingerprint.clone(), token.clone()));
        let e = self.clone();
        self.tasks.lock().await.push(tokio::spawn(async move{let _permit=permit;let mut attempts=0;
   let result=loop{
    let result=tokio::select!{_=token.cancelled()=>Err(anyhow::anyhow!("cancelled")),r=e.send_once(&m,&p,&path,&token)=>r};
    if result.is_ok() || token.is_cancelled() || attempts>=5 || result.as_ref().err().is_some_and(|error|!crate::errors::from_error(error).retryable){break result}
    attempts+=1;e.emit("progress",serde_json::json!({"id":id,"status":"reconnecting","attempt":attempts,"error":format!("{result:?}")}));
    let delay=(1u64<<attempts.min(5))*500+rand::random::<u16>() as u64%500;
    tokio::select!{_=token.cancelled()=>break Err(anyhow::anyhow!("cancelled")),_=tokio::time::sleep(Duration::from_millis(delay))=>{}}
   };
   let (status,error)=match result{Ok(())=>("complete",None),Err(error)=>(if token.is_cancelled(){"cancelled"}else{"recoverable_error"},Some(error.to_string()))};
   let st=e.store.clone();let tid=id.clone();let err=error.clone();let _=blocking(move||st.status(&tid,status,err.as_deref())).await;
   e.emit("progress",serde_json::json!({"id":id,"status":status,"error":error}));e.transfers.lock().await.remove(&id);
  }));
        Ok(())
    }
    async fn send_once(
        &self,
        m: &Manifest,
        p: &Peer,
        path: &str,
        token: &CancellationToken,
    ) -> Result<()> {
        ensure!(self.trusted(&p.fingerprint).await?, "trust revoked");
        // Full rehash before every resume catches changes in already-acknowledged chunks.
        let source = PathBuf::from(path);
        let id = m.id.clone();
        let cancel = token.clone();
        let mm =
            blocking(move || transfer::manifest_cancellable(&source, id, Some(&cancel))).await?;
        ensure!(mm == *m, "source changed; unsafe resumption rejected");
        let (mut s, connected) = self.connect(&p.address, Some(&p.fingerprint)).await?;
        if connected.address != p.address {
            let store = self.store.clone();
            let id = m.id.clone();
            let address = connected.address;
            blocking(move || store.relocate(&id, &address)).await?;
        }
        write_message(
            &mut s,
            &Message::Bulk {
                manifest: m.clone(),
            },
        )
        .await?;
        let missing = match read_deadline(&mut s).await? {
            Message::Missing { indexes } => indexes,
            Message::Error { code, message } => return Err(crate::errors::remote(code, message)),
            _ => bail!("missing-list expected"),
        };
        ensure!(missing.len() <= m.hashes.len(), "invalid missing list");
        let mut seen = std::collections::HashSet::new();
        let mut remain = 0;
        for idx in &missing {
            ensure!(seen.insert(*idx), "duplicate missing index");
            remain += m.length(*idx)? as u64;
        }
        let baseline = m.size - remain;
        let acknowledged = self.store.clone();
        let manifest = m.clone();
        let pending = missing.clone();
        blocking(move || acknowledged.confirm_missing(&manifest, &pending)).await?;
        let started = std::time::Instant::now();
        let mut last = std::time::Instant::now() - Duration::from_secs(1);
        let mut expected_verified = baseline;
        for window in missing.chunks(self.transfer_limits().in_flight as usize) {
            // Only one file chunk is held in user memory at a time. Socket/TLS
            // backpressure bounds writes; at most this window awaits durable ACKs.
            for idx in window {
                ensure!(self.trusted(&p.fingerprint).await?, "trust revoked");
                let src = PathBuf::from(path);
                let mc = m.clone();
                let index = *idx;
                let bytes = blocking(move || transfer::source_chunk(&src, &mc, index)).await?;
                timeout(DEADLINE, write_chunk(&mut s, index, &bytes)).await??;
            }
            for idx in window {
                expected_verified += m.length(*idx)? as u64;
                let verified = match read_deadline(&mut s).await? {
                    Message::Ack {
                        index,
                        verified_bytes,
                    } => {
                        ensure!(
                            index == *idx && verified_bytes == expected_verified,
                            "invalid ACK"
                        );
                        verified_bytes
                    }
                    Message::Error { code, message } => {
                        return Err(crate::errors::remote(code, message))
                    }
                    _ => bail!("ACK expected"),
                };
                let acknowledged = self.store.clone();
                let id = m.id.clone();
                let index = *idx;
                let hash = m.hashes[index as usize].clone();
                blocking(move || acknowledged.chunk(&id, index, &hash)).await?;
                if last.elapsed() >= Duration::from_millis(200) {
                    self.emit("progress",serde_json::json!({"id":m.id,"verifiedBytes":verified,"totalBytes":m.size,"bytesPerSecond":verified.saturating_sub(baseline) as f64/started.elapsed().as_secs_f64().max(0.001),"status":"sending"}));
                    last = std::time::Instant::now();
                }
            }
        }
        write_message(&mut s, &Message::Finish).await?;
        ensure!(
            matches!(read_deadline(&mut s).await?, Message::Complete),
            "completion not confirmed"
        );
        self.emit("progress",serde_json::json!({"id":m.id,"verifiedBytes":m.size,"totalBytes":m.size,"status":"complete"}));
        Ok(())
    }
    pub async fn cancel(&self, id: String) -> Result<()> {
        if let Some((_, t)) = self.transfers.lock().await.get(&id) {
            t.cancel();
        }
        Ok(())
    }
    async fn discovery(self: &Arc<Self>) -> Result<()> {
        let props = [
            ("id", self.identity.fingerprint.as_str()),
            ("name", self.config.name.as_str()),
            ("version", "2"),
        ];
        let instance = &self.identity.fingerprint[..16];
        let hostname = format!("lc-{instance}.local.");
        let info = mdns_sd::ServiceInfo::new(
            "_localconnect._tcp.local.",
            instance,
            &hostname,
            "",
            self.port,
            &props[..],
        )?
        .enable_addr_auto();
        self.mdns.register(info)?;
        let browser = self.mdns.browse("_localconnect._tcp.local.")?;
        let e = self.clone();
        self.tasks.lock().await.push(tokio::task::spawn_blocking(move||{while !e.stop.is_cancelled(){match browser.recv_timeout(Duration::from_millis(500)){
   Ok(mdns_sd::ServiceEvent::ServiceResolved(info))=>{let id=info.get_property_val_str("id").unwrap_or("");if id==e.identity.fingerprint{continue}let name=info.get_property_val_str("name").unwrap_or("Unknown");if id.len()!=64||name.len()>64||info.get_property_val_str("version")!=Some("2"){continue}let addresses:Vec<String>=info.get_addresses().iter().filter(|ip|lan(**ip) && !(matches!(ip,IpAddr::V6(v) if (v.segments()[0]&0xffc0)==0xfe80))).map(|ip|SocketAddr::new(*ip,info.get_port()).to_string()).collect();e.device_update(info.get_fullname().to_string(),serde_json::json!({"service":info.get_fullname(),"id":id,"name":name,"addresses":addresses,"untrustedMetadata":true}));},
   Ok(mdns_sd::ServiceEvent::ServiceRemoved(_,name))=>e.device_remove(name),_=>{}}
  }}));
        Ok(())
    }
    pub fn cancellation(&self) -> CancellationToken {
        self.stop.clone()
    }
    pub fn subscribe_events(&self) -> (broadcast::Receiver<Event>, Vec<Event>) {
        let devices = self.discovered.lock().unwrap_or_else(|e| e.into_inner());
        let receiver = self.events.subscribe();
        let mut snapshot = vec![Event {
            kind: "discovery_reset".into(),
            payload: "{}".into(),
        }];
        snapshot.extend(devices.values().cloned());
        snapshot.extend(
            self.connection_states
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .cloned(),
        );
        snapshot.extend(self.remote.snapshot());
        (receiver, snapshot)
    }
    pub fn transfer_limits(&self) -> TransferLimits {
        self.limits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub async fn configure_transfer(&self, limits: TransferLimits) -> Result<()> {
        limits.validate()?;
        let _serial = self.configure_gate.lock().await;
        let store = self.store.clone();
        let saved = limits.clone();
        blocking(move || store.save_limits(&saved)).await?;
        *self.limits.lock().unwrap_or_else(|e| e.into_inner()) = limits;
        Ok(())
    }
    pub async fn cleanup_transfer(&self, id: String) -> Result<()> {
        let _incoming = self
            .receive_gate
            .clone()
            .try_acquire_owned()
            .context("stop active receives before cleanup")?;
        let _outgoing = self
            .send_gate
            .clone()
            .try_acquire_owned()
            .context("stop active sends before cleanup")?;
        let store = self.store.clone();
        let directory = PathBuf::from(&self.config.receive_dir);
        blocking(move || {
            let (manifest, direction, destination) = store.registered_transfer(&id)?;
            manifest.validate()?;
            if direction == "receive" {
                let directory = std::fs::canonicalize(directory)?;
                ensure!(
                    PathBuf::from(destination) == directory.join(&manifest.name),
                    "cleanup folder differs from registered destination"
                );
                let partial = directory.join(format!(".{}.localconnect.part", manifest.id));
                match std::fs::symlink_metadata(&partial) {
                    Ok(meta) => {
                        ensure!(meta.is_file(), "cleanup only removes regular partial files");
                        std::fs::remove_file(partial)?;
                        #[cfg(unix)]
                        std::fs::File::open(directory)?.sync_all()?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            // Original sources and published destination files are retained.
            store.remove_transfer(&id)
        })
        .await
    }
    fn device_update(&self, service: String, payload: serde_json::Value) {
        let mut devices = self.discovered.lock().unwrap_or_else(|e| e.into_inner());
        if devices.len() >= 256 && !devices.contains_key(&service) {
            return;
        }
        let event = Event {
            kind: "device".into(),
            payload: payload.to_string(),
        };
        devices.insert(service, event.clone());
        let _ = self.events.send(event);
    }
    fn device_remove(&self, service: String) {
        let mut devices = self.discovered.lock().unwrap_or_else(|e| e.into_inner());
        devices.remove(&service);
        self.emit("device_removed", serde_json::json!({"service":service}));
    }
    pub async fn shutdown(&self) {
        self.stop.cancel();
        self.codes.lock().await.clear();
        self.session_trust.lock().unwrap_or_else(|error| error.into_inner()).clear();
        let input_cleanup = self.remote.stop_all();
        let _ = self.mdns.shutdown();
        loop {
            let tasks = std::mem::take(&mut *self.tasks.lock().await);
            if tasks.is_empty() {
                break;
            }
            for task in tasks {
                let _ = task.await;
            }
        }
        for completion in input_cleanup {
            let _ = crate::remote_engine::wait_input_done(completion).await;
        }
        self.emit("stopped", serde_json::json!({}));
    }
}
