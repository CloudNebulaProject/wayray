//! QUIC transport server for wrsrvd.
//!
//! Runs a tokio runtime in a background thread, accepting a single client
//! connection. Communicates with the compositor via `std::sync::mpsc` channels.
//!
//! Three logical QUIC streams:
//! - **Control** (bidirectional): handshake, ping/pong, frame acks
//! - **Display** (server→client, unidirectional): frame updates
//! - **Input** (client→server, unidirectional): keyboard/pointer events
//!
//! ## Quinn stream semantics
//!
//! Quinn streams are lazily materialized on the wire: `open_bi()` and
//! `open_uni()` return immediately, but the peer's `accept_bi()`/`accept_uni()`
//! won't resolve until data is actually written to the stream. The handshake
//! protocol accounts for this by having each side write data before the other
//! side tries to accept.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;

use quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rcgen::CertifiedKey;
use tokio::runtime::Runtime;
use tracing::{error, info, warn};
use wayray_protocol::codec;
use wayray_protocol::messages::{
    ClientHello, ControlMessage, DisplayMessage, FrameUpdate, InputMessage, ServerHello,
    ServerInfoMsg, SessionEvent, SessionLookupResponse,
};

/// Set of session tokens this server currently hosts in a resumable state.
///
/// Published by the compositor thread (which owns the session registry) and
/// read by the network thread when answering `SessionLookupRequest` probes from
/// peer servers. Shared so a peer can discover that a token's session lives here
/// without round-tripping through the calloop event loop on the probe path.
pub type LocalTokens = Arc<Mutex<HashSet<String>>>;

/// Messages sent from the compositor to the network thread.
pub enum CompositorToNet {
    /// Send a frame update to the connected client.
    SendFrame(FrameUpdate),
    /// Shut down the network thread.
    Shutdown,
}

/// Resolved session binding returned by the compositor in response to a
/// `ClientConnected` event. Carries the real session id and resume flag so
/// the network thread can compose an accurate `ServerHello`.
pub struct SessionBinding {
    /// The session id assigned/resolved by the session registry.
    pub session_id: u64,
    /// Whether this connection resumed an existing session (hot-desking).
    pub resumed: bool,
    /// The token bound to the session (echoed back to the client).
    pub token: String,
}

/// The compositor's decision for a connecting client: either bind the session
/// locally (the common single-server path and hot-desk resume) or redirect the
/// client to the server that actually hosts (or should host) its session.
pub enum ConnectDecision {
    /// Serve the session on this server.
    Bind(SessionBinding),
    /// Send the client to another server (session affinity or load balancing).
    Redirect {
        /// Target server id.
        server_id: String,
        /// Target server address as `host:port`.
        addr: String,
    },
}

/// Messages sent from the network thread to the compositor.
pub enum NetToCompositor {
    /// An input event received from the client.
    Input(InputMessage),
    /// A control message (e.g., FrameAck) from the client.
    Control(ControlMessage),
    /// Client connected with the given hello. The compositor resolves the
    /// session via the registry and replies with a `ConnectDecision` so the
    /// network thread can either compose an accurate `ServerHello` or redirect
    /// the client to the server that owns its session.
    ClientConnected {
        hello: ClientHello,
        reply: mpsc::Sender<ConnectDecision>,
    },
    /// Client disconnected.
    ClientDisconnected,
}

/// Configuration for the QUIC server.
pub struct ServerConfig {
    /// Address to bind to. Defaults to `0.0.0.0:4433`.
    pub bind_addr: SocketAddr,
    /// Virtual output dimensions for the ServerHello.
    pub output_width: u32,
    /// Virtual output dimensions for the ServerHello.
    pub output_height: u32,
    /// This server's id within the cluster (empty for single-server mode).
    /// Reported in `ServerInfo` responses so peers can identify it.
    pub server_id: String,
    /// Session capacity reported in `ServerInfo` responses (for load
    /// balancing). Defaults to [`DEFAULT_CAPACITY`].
    pub capacity: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:4433".parse().unwrap(),
            output_width: 1280,
            output_height: 720,
            server_id: String::new(),
            capacity: wayray_protocol::cluster::DEFAULT_CAPACITY,
        }
    }
}

/// Handle to a running network server thread.
pub struct NetworkHandle {
    /// Send commands to the network thread.
    pub tx: mpsc::Sender<CompositorToNet>,
    /// Receive events from the network thread.
    pub rx: mpsc::Receiver<NetToCompositor>,
    /// Live count of active sessions, published by the compositor and read by
    /// the network thread when answering `ServerInfo` queries from peers.
    pub active_sessions: Arc<AtomicU32>,
    /// Tokens whose sessions are resumable on this server, published by the
    /// compositor and read by the network thread when answering peer
    /// `SessionLookupRequest` probes.
    pub local_tokens: LocalTokens,
    /// Join handle for the background thread.
    join: Option<thread::JoinHandle<()>>,
}

impl NetworkHandle {
    /// Publish the current active-session count so peer `ServerInfo` queries
    /// report an accurate load.
    pub fn set_active_sessions(&self, count: u32) {
        self.active_sessions.store(count, Ordering::Relaxed);
    }

    /// Replace the published set of resumable tokens hosted by this server.
    /// Peers consult this (via `SessionLookupRequest`) to discover a token's
    /// home server and redirect reconnecting clients here.
    pub fn set_local_tokens(&self, tokens: impl IntoIterator<Item = String>) {
        if let Ok(mut guard) = self.local_tokens.lock() {
            guard.clear();
            guard.extend(tokens);
        }
    }

    /// Shut down the network thread and wait for it to exit.
    pub fn shutdown(mut self) {
        let _ = self.tx.send(CompositorToNet::Shutdown);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for NetworkHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(CompositorToNet::Shutdown);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

/// Generate a self-signed TLS certificate for the QUIC server.
fn generate_self_signed_cert() -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    let CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "wayray".to_string()])
            .expect("certificate generation failed");
    let cert_der = CertificateDer::from(cert);
    let key_der = PrivateKeyDer::try_from(key_pair.serialize_der()).expect("key serialization");
    (vec![cert_der], key_der)
}

/// Build a quinn `ServerConfig` from a self-signed cert.
fn build_server_config() -> quinn::ServerConfig {
    let (certs, key) = generate_self_signed_cert();
    let provider = rustls::crypto::ring::default_provider();
    let crypto = rustls::ServerConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .expect("TLS protocol versions")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("TLS server config");
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(crypto)
        .expect("QUIC server crypto config");
    quinn::ServerConfig::with_crypto(std::sync::Arc::new(crypto))
}

/// Start the QUIC server on a background thread.
///
/// Returns a `NetworkHandle` with channels for communication.
/// The server accepts one client at a time. When a client disconnects,
/// it loops back to accept the next one.
pub fn start_server(config: ServerConfig) -> NetworkHandle {
    let (comp_tx, net_rx) = mpsc::channel::<CompositorToNet>();
    let (net_tx, comp_rx) = mpsc::channel::<NetToCompositor>();

    let active_sessions = Arc::new(AtomicU32::new(0));
    let active_for_thread = active_sessions.clone();
    let local_tokens: LocalTokens = Arc::new(Mutex::new(HashSet::new()));
    let tokens_for_thread = local_tokens.clone();

    let join = thread::Builder::new()
        .name("wayray-net".into())
        .spawn(move || {
            let rt = Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                if let Err(e) =
                    server_loop(config, net_rx, net_tx, active_for_thread, tokens_for_thread).await
                {
                    error!("network server error: {e}");
                }
            });
        })
        .expect("spawn network thread");

    NetworkHandle {
        tx: comp_tx,
        rx: comp_rx,
        active_sessions,
        local_tokens,
        join: Some(join),
    }
}

/// Main server accept loop.
async fn server_loop(
    config: ServerConfig,
    compositor_rx: mpsc::Receiver<CompositorToNet>,
    compositor_tx: mpsc::Sender<NetToCompositor>,
    active_sessions: Arc<AtomicU32>,
    local_tokens: LocalTokens,
) -> Result<(), Box<dyn std::error::Error>> {
    let server_config = build_server_config();
    let endpoint = quinn::Endpoint::server(server_config, config.bind_addr)?;
    info!(addr = %config.bind_addr, "QUIC server listening");

    loop {
        // Check for shutdown before waiting for connection.
        if let Ok(CompositorToNet::Shutdown) = compositor_rx.try_recv() {
            info!("network: shutdown requested");
            break;
        }

        let incoming = tokio::select! {
            incoming = endpoint.accept() => {
                match incoming {
                    Some(incoming) => incoming,
                    None => {
                        info!("QUIC endpoint closed");
                        break;
                    }
                }
            }
            _ = check_shutdown_async(&compositor_rx) => {
                info!("network: shutdown during accept");
                break;
            }
        };

        let connection = match incoming.await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("failed to accept connection: {e}");
                continue;
            }
        };

        info!(
            remote = %connection.remote_address(),
            "client connected"
        );

        if let Err(e) = handle_connection(
            &connection,
            &config,
            &compositor_rx,
            &compositor_tx,
            &active_sessions,
            &local_tokens,
        )
        .await
        {
            warn!("client session ended: {e}");
        }

        let _ = compositor_tx.send(NetToCompositor::ClientDisconnected);
        info!("client disconnected, waiting for next connection");
    }

    endpoint.close(0u32.into(), b"shutdown");
    Ok(())
}

/// Await a `ConnectDecision` reply from the compositor, polling the blocking
/// `std::sync::mpsc` channel from this async context with a short interval and
/// a hard deadline. Returns `None` on timeout or if the sender is dropped.
async fn recv_binding_with_timeout(
    rx: mpsc::Receiver<ConnectDecision>,
    timeout: std::time::Duration,
) -> Option<ConnectDecision> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match rx.try_recv() {
            Ok(binding) => return Some(binding),
            Err(mpsc::TryRecvError::Disconnected) => return None,
            Err(mpsc::TryRecvError::Empty) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        }
    }
}

/// Poll for shutdown on a blocking channel from an async context.
async fn check_shutdown_async(rx: &mpsc::Receiver<CompositorToNet>) {
    loop {
        match rx.try_recv() {
            Ok(CompositorToNet::Shutdown) => return,
            Ok(_) => continue, // drain non-shutdown messages
            Err(mpsc::TryRecvError::Disconnected) => return,
            Err(mpsc::TryRecvError::Empty) => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// Handle a single client connection: handshake on control stream, then
/// relay messages between compositor and client until disconnect.
///
/// Stream setup protocol (accounts for quinn's lazy stream creation):
/// 1. Client opens bidi stream and immediately sends `ClientHello` (triggers
///    server's `accept_bi`).
/// 2. Server reads `ClientHello`, sends `ServerHello` on control stream.
/// 3. Server opens display uni stream and sends an initial empty frame
///    (triggers client's `accept_uni` for display).
/// 4. Client opens input uni stream — server's `accept_uni` for input is
///    handled asynchronously when the client first writes input data.
async fn handle_connection(
    connection: &quinn::Connection,
    config: &ServerConfig,
    compositor_rx: &mpsc::Receiver<CompositorToNet>,
    compositor_tx: &mpsc::Sender<NetToCompositor>,
    active_sessions: &Arc<AtomicU32>,
    local_tokens: &LocalTokens,
) -> Result<(), Box<dyn std::error::Error>> {
    // Step 1: Accept control stream. The peer writes its first control message
    // immediately after opening, so accept_bi resolves once that data arrives.
    let (mut control_send, mut control_recv) = connection.accept_bi().await?;
    info!("control stream established");

    // Step 2: Read the first control message. A peer load-balancing probe sends
    // `ServerInfoRequest` and expects a `ServerInfo` reply; a real client sends
    // `ClientHello`.
    let first: ControlMessage = read_message(&mut control_recv).await?;
    let hello = match first {
        ControlMessage::ClientHello(hello) => hello,
        ControlMessage::ServerInfoRequest => {
            let info = ControlMessage::ServerInfo(ServerInfoMsg {
                server_id: config.server_id.clone(),
                active_sessions: active_sessions.load(Ordering::Relaxed),
                capacity: config.capacity,
            });
            write_message(&mut control_send, &info).await?;
            info!("answered ServerInfo probe");
            return Ok(());
        }
        ControlMessage::SessionLookupRequest { token } => {
            let hosts = local_tokens
                .lock()
                .map(|t| t.contains(&token))
                .unwrap_or(false);
            let resp = ControlMessage::SessionLookupResponse(SessionLookupResponse {
                server_id: config.server_id.clone(),
                hosts,
            });
            write_message(&mut control_send, &resp).await?;
            info!(%token, hosts, "answered SessionLookup probe");
            return Ok(());
        }
        other => {
            return Err(format!(
                "expected ClientHello, ServerInfoRequest, or SessionLookupRequest, got {other:?}"
            )
            .into());
        }
    };
    info!(version = hello.version, "received ClientHello");
    let fallback_token = hello.token.clone().unwrap_or_default();

    // Round-trip to the compositor: it resolves the session via the registry
    // and replies with either a binding (serve here) or a redirect (the session
    // lives on another server / load balancing). We bound the wait so a stalled
    // compositor cannot wedge the handshake; on timeout we fall back to a
    // best-effort local ServerHello.
    let (reply_tx, reply_rx) = mpsc::channel::<ConnectDecision>();
    let connect_start = std::time::Instant::now();
    let _ = compositor_tx.send(NetToCompositor::ClientConnected {
        hello,
        reply: reply_tx,
    });

    let decision = recv_binding_with_timeout(reply_rx, std::time::Duration::from_millis(250)).await;
    let (session_id, resumed, token) = match decision {
        Some(ConnectDecision::Bind(b)) => {
            info!(
                session_id = b.session_id,
                resumed = b.resumed,
                elapsed_ms = connect_start.elapsed().as_millis() as u64,
                "session binding resolved"
            );
            (b.session_id, b.resumed, b.token)
        }
        Some(ConnectDecision::Redirect { server_id, addr }) => {
            // The session belongs elsewhere (affinity) or this server is
            // shedding load. Tell the client where to go and close cleanly.
            info!(%server_id, %addr, "redirecting client to peer server");
            let redirect = ControlMessage::SessionRedirect { server_id, addr };
            write_message(&mut control_send, &redirect).await?;
            let _ = control_send.finish();
            return Ok(());
        }
        None => {
            warn!("timed out waiting for session binding; using fallback ServerHello");
            (0, false, fallback_token)
        }
    };

    let server_hello = ControlMessage::ServerHello(ServerHello {
        version: wayray_protocol::PROTOCOL_VERSION,
        session_id,
        output_width: config.output_width,
        output_height: config.output_height,
        resumed,
        token,
    });
    write_message(&mut control_send, &server_hello).await?;
    info!("sent ServerHello");

    // On a resumed (hot-desked) session, tell the client to drop any frame
    // cache and expect a full redraw.
    if resumed {
        let resumed_event = ControlMessage::SessionEvent(SessionEvent::Resumed { session_id });
        if let Err(e) = write_message(&mut control_send, &resumed_event).await {
            warn!(error = %e, "failed to send Resumed event");
        }
    }

    // Step 3: Open display uni stream. Writing data triggers the client's
    // accept_uni for this stream.
    let mut display_send = connection.open_uni().await?;
    info!("display stream opened");

    // Step 4: Message relay loop. Accept the input uni stream concurrently
    // with handling control messages and compositor commands.
    let mut input_recv: Option<quinn::RecvStream> = None;
    let mut accepting_input = true;

    loop {
        tokio::select! {
            // Accept input stream from client (one-shot).
            result = connection.accept_uni(), if accepting_input => {
                match result {
                    Ok(recv) => {
                        info!("input stream established");
                        input_recv = Some(recv);
                        accepting_input = false;
                    }
                    Err(e) => {
                        return Err(format!("failed to accept input stream: {e}").into());
                    }
                }
            }

            // Read control messages from client.
            msg = read_message::<ControlMessage>(&mut control_recv) => {
                match msg {
                    Ok(ctrl) => {
                        let _ = compositor_tx.send(NetToCompositor::Control(ctrl));
                    }
                    Err(e) => {
                        return Err(format!("control stream error: {e}").into());
                    }
                }
            }

            // Read input messages from client (only when stream is established).
            msg = async {
                match input_recv.as_mut() {
                    Some(recv) => read_message::<InputMessage>(recv).await,
                    None => std::future::pending().await,
                }
            } => {
                match msg {
                    Ok(input) => {
                        let _ = compositor_tx.send(NetToCompositor::Input(input));
                    }
                    Err(e) => {
                        return Err(format!("input stream error: {e}").into());
                    }
                }
            }

            // Check for messages from the compositor.
            _ = check_compositor_commands(
                compositor_rx,
                &mut display_send,
            ) => {
                // Shutdown requested.
                return Ok(());
            }
        }
    }
}

/// Process commands from the compositor channel. Returns when shutdown
/// is requested or the channel is disconnected.
async fn check_compositor_commands(
    rx: &mpsc::Receiver<CompositorToNet>,
    display_send: &mut quinn::SendStream,
) {
    loop {
        match rx.try_recv() {
            Ok(CompositorToNet::SendFrame(frame)) => {
                let msg = DisplayMessage::FrameUpdate(frame);
                if let Err(e) = write_message(display_send, &msg).await {
                    warn!("failed to send frame: {e}");
                    return;
                }
            }
            Ok(CompositorToNet::Shutdown) => return,
            Err(mpsc::TryRecvError::Disconnected) => return,
            Err(mpsc::TryRecvError::Empty) => {
                // Yield to let other select branches run.
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        }
    }
}

/// Build a quinn client config that skips server certificate verification.
/// Used for short-lived peer `ServerInfo` probes (dev self-signed certs).
fn build_peer_client_config() -> quinn::ClientConfig {
    let provider = rustls::crypto::ring::default_provider();
    let crypto = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .expect("TLS protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(PeerSkipVerification))
        .with_no_client_auth();
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
        .expect("QUIC client crypto config");
    quinn::ClientConfig::new(std::sync::Arc::new(crypto))
}

/// Certificate verifier for peer probes that accepts any server cert (dev).
#[derive(Debug)]
struct PeerSkipVerification;

impl rustls::client::danger::ServerCertVerifier for PeerSkipVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Query a peer server's load by opening a short-lived QUIC connection and
/// exchanging `ServerInfoRequest`/`ServerInfo` on the control stream.
///
/// Returns `None` on any error or timeout; callers treat that as "peer
/// unavailable" and fall back to local placement.
pub async fn query_peer_info(addr: SocketAddr) -> Option<ServerInfoMsg> {
    let result = tokio::time::timeout(std::time::Duration::from_millis(300), async {
        let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse::<SocketAddr>().ok()?).ok()?;
        endpoint.set_default_client_config(build_peer_client_config());

        let connection = endpoint.connect(addr, "localhost").ok()?.await.ok()?;
        let (mut send, mut recv) = connection.open_bi().await.ok()?;

        write_message(&mut send, &ControlMessage::ServerInfoRequest)
            .await
            .ok()?;

        let resp: ControlMessage = read_message(&mut recv).await.ok()?;
        match resp {
            ControlMessage::ServerInfo(info) => Some(info),
            _ => None,
        }
    })
    .await;

    match result {
        Ok(info) => info,
        Err(_) => {
            warn!(%addr, "peer ServerInfo query timed out");
            None
        }
    }
}

/// Ask a peer server whether it hosts a resumable session for `token`, by
/// opening a short-lived QUIC connection and exchanging
/// `SessionLookupRequest`/`SessionLookupResponse` on the control stream.
///
/// Returns `Some(server_id)` when the peer confirms it hosts the session,
/// `None` on a negative answer, error, or timeout. Used for cross-server
/// session affinity: a server that receives a client for an unknown token
/// probes its peers to find the session's home server before deciding whether
/// to redirect the client there or create a new local session.
pub async fn query_peer_for_token(addr: SocketAddr, token: &str) -> Option<String> {
    let token = token.to_string();
    let result = tokio::time::timeout(std::time::Duration::from_millis(300), async {
        let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse::<SocketAddr>().ok()?).ok()?;
        endpoint.set_default_client_config(build_peer_client_config());

        let connection = endpoint.connect(addr, "localhost").ok()?.await.ok()?;
        let (mut send, mut recv) = connection.open_bi().await.ok()?;

        write_message(&mut send, &ControlMessage::SessionLookupRequest { token })
            .await
            .ok()?;

        let resp: ControlMessage = read_message(&mut recv).await.ok()?;
        match resp {
            ControlMessage::SessionLookupResponse(r) if r.hosts => Some(r.server_id),
            _ => None,
        }
    })
    .await;

    match result {
        Ok(server_id) => server_id,
        Err(_) => {
            warn!(%addr, "peer SessionLookup query timed out");
            None
        }
    }
}

/// Read a length-prefixed message from a QUIC receive stream.
async fn read_message<T: serde::de::DeserializeOwned>(
    recv: &mut quinn::RecvStream,
) -> Result<T, Box<dyn std::error::Error>> {
    // Read 4-byte length prefix.
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    // Read payload.
    let mut payload = vec![0u8; len];
    recv.read_exact(&mut payload).await?;

    let msg = codec::decode(&payload)?;
    Ok(msg)
}

/// Write a length-prefixed message to a QUIC send stream.
async fn write_message<T: serde::Serialize>(
    send: &mut quinn::SendStream,
    msg: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    let encoded = codec::encode(msg)?;
    send.write_all(&encoded).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayray_protocol::messages::ControlMessage;

    /// Build a test client endpoint with certificate verification disabled.
    fn build_test_client_endpoint() -> quinn::Endpoint {
        let provider = rustls::crypto::ring::default_provider();
        let crypto = rustls::ClientConfig::builder_with_provider(provider.into())
            .with_safe_default_protocol_versions()
            .expect("TLS protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(SkipServerVerification))
            .with_no_client_auth();
        let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto).unwrap();
        let client_config = quinn::ClientConfig::new(std::sync::Arc::new(crypto));

        let mut endpoint =
            quinn::Endpoint::client("127.0.0.1:0".parse::<SocketAddr>().unwrap()).unwrap();
        endpoint.set_default_client_config(client_config);
        endpoint
    }

    /// Dummy certificate verifier that accepts any server cert.
    #[derive(Debug)]
    struct SkipServerVerification;

    impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_client_hello_exchange() {
        let server_config = build_server_config();
        let endpoint =
            quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let actual_addr = endpoint.local_addr().unwrap();

        // Server side: accept bi, read ClientHello, send ServerHello.
        let server_handle = tokio::spawn(async move {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();

            // Client writes ClientHello immediately, so accept_bi resolves.
            let (mut control_send, mut control_recv) = connection.accept_bi().await.unwrap();

            let hello: ControlMessage = read_message(&mut control_recv).await.unwrap();
            let ControlMessage::ClientHello(client_hello) = hello else {
                panic!("expected ClientHello");
            };
            assert_eq!(client_hello.version, wayray_protocol::PROTOCOL_VERSION);

            let server_hello = ControlMessage::ServerHello(ServerHello {
                version: wayray_protocol::PROTOCOL_VERSION,
                session_id: 42,
                output_width: 1920,
                output_height: 1080,
                resumed: false,
                token: "test-token".to_string(),
            });
            write_message(&mut control_send, &server_hello)
                .await
                .unwrap();

            // Wait for client to read the response before closing.
            let _ = read_message::<ControlMessage>(&mut control_recv).await;
            endpoint.close(0u32.into(), b"done");
        });

        // Client side: open bi and immediately send ClientHello.
        let client_endpoint = build_test_client_endpoint();
        let connection = client_endpoint
            .connect(actual_addr, "localhost")
            .unwrap()
            .await
            .unwrap();

        let (mut control_send, mut control_recv) = connection.open_bi().await.unwrap();

        // Write immediately to trigger server's accept_bi.
        let client_hello = ControlMessage::ClientHello(ClientHello {
            version: wayray_protocol::PROTOCOL_VERSION,
            capabilities: vec!["display".to_string()],
            token: Some("test-token".to_string()),
        });
        write_message(&mut control_send, &client_hello)
            .await
            .unwrap();

        // Read ServerHello.
        let response: ControlMessage = read_message(&mut control_recv).await.unwrap();
        let ControlMessage::ServerHello(server_hello) = response else {
            panic!("expected ServerHello, got {response:?}");
        };
        assert_eq!(server_hello.version, wayray_protocol::PROTOCOL_VERSION);
        assert_eq!(server_hello.session_id, 42);
        assert_eq!(server_hello.output_width, 1920);
        assert_eq!(server_hello.output_height, 1080);

        // Signal server to close.
        let ping = ControlMessage::Ping(wayray_protocol::messages::Ping { timestamp: 0 });
        let _ = write_message(&mut control_send, &ping).await;

        server_handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_display_and_input_streams() {
        let server_config = build_server_config();
        let endpoint =
            quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let actual_addr = endpoint.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();

            // Accept control and do handshake.
            let (mut control_send, mut control_recv) = connection.accept_bi().await.unwrap();
            let _hello: ControlMessage = read_message(&mut control_recv).await.unwrap();
            let server_hello = ControlMessage::ServerHello(ServerHello {
                version: wayray_protocol::PROTOCOL_VERSION,
                session_id: 1,
                output_width: 1280,
                output_height: 720,
                resumed: false,
                token: String::new(),
            });
            write_message(&mut control_send, &server_hello)
                .await
                .unwrap();

            // Open display uni and send a frame (triggers client's accept_uni).
            let mut display_send = connection.open_uni().await.unwrap();
            let frame = DisplayMessage::FrameUpdate(FrameUpdate {
                sequence: 1,
                regions: vec![],
            });
            write_message(&mut display_send, &frame).await.unwrap();

            // Accept input uni (triggered by client writing input).
            let mut input_recv = connection.accept_uni().await.unwrap();
            let input: InputMessage = read_message(&mut input_recv).await.unwrap();
            assert!(matches!(input, InputMessage::Keyboard(_)));

            endpoint.close(0u32.into(), b"done");
        });

        // Client side.
        let client_endpoint = build_test_client_endpoint();
        let connection = client_endpoint
            .connect(actual_addr, "localhost")
            .unwrap()
            .await
            .unwrap();

        // Open control and send ClientHello.
        let (mut control_send, mut control_recv) = connection.open_bi().await.unwrap();
        let client_hello = ControlMessage::ClientHello(ClientHello {
            version: wayray_protocol::PROTOCOL_VERSION,
            capabilities: vec![],
            token: None,
        });
        write_message(&mut control_send, &client_hello)
            .await
            .unwrap();
        let _: ControlMessage = read_message(&mut control_recv).await.unwrap();

        // Open input uni and send keyboard event (triggers server's accept_uni).
        let mut input_send = connection.open_uni().await.unwrap();
        let input = InputMessage::Keyboard(wayray_protocol::messages::KeyboardEvent {
            keycode: 42,
            state: wayray_protocol::messages::KeyState::Pressed,
            time: 1000,
        });
        write_message(&mut input_send, &input).await.unwrap();

        // Accept display uni from server (triggered by server writing frame).
        let mut display_recv = connection.accept_uni().await.unwrap();
        let frame: DisplayMessage = read_message(&mut display_recv).await.unwrap();
        let DisplayMessage::FrameUpdate(update) = frame;
        assert_eq!(update.sequence, 1);
        assert!(update.regions.is_empty());

        server_handle.await.unwrap();
    }

    #[test]
    fn cert_generation_works() {
        let (certs, _key) = generate_self_signed_cert();
        assert_eq!(certs.len(), 1);
    }

    #[tokio::test]
    async fn binding_timeout_returns_none() {
        // No sender ever sends; should time out quickly and return None.
        let (_tx, rx) = mpsc::channel::<ConnectDecision>();
        let binding = recv_binding_with_timeout(rx, std::time::Duration::from_millis(20)).await;
        assert!(binding.is_none());
    }

    #[tokio::test]
    async fn binding_received_before_timeout() {
        let (tx, rx) = mpsc::channel::<ConnectDecision>();
        tx.send(ConnectDecision::Bind(SessionBinding {
            session_id: 7,
            resumed: true,
            token: "t".into(),
        }))
        .unwrap();
        let decision = recv_binding_with_timeout(rx, std::time::Duration::from_millis(250))
            .await
            .expect("decision should arrive");
        match decision {
            ConnectDecision::Bind(b) => {
                assert_eq!(b.session_id, 7);
                assert!(b.resumed);
            }
            ConnectDecision::Redirect { .. } => panic!("expected Bind"),
        }
    }

    /// Full handshake through `handle_connection`, with a fake compositor that
    /// replies to the `ClientConnected` reply channel. Verifies the ServerHello
    /// carries the session id the compositor chose (not a hardcoded value) and
    /// that `resumed` reflects the reply.
    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn handle_connection_uses_reply_binding() {
        let server_config = build_server_config();
        let endpoint =
            quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let actual_addr = endpoint.local_addr().unwrap();

        let (comp_to_net_tx, comp_to_net_rx) = mpsc::channel::<CompositorToNet>();
        let (net_to_comp_tx, net_to_comp_rx) = mpsc::channel::<NetToCompositor>();

        // Fake compositor: wait for ClientConnected, reply with a chosen id.
        let fake_compositor = std::thread::spawn(move || {
            loop {
                match net_to_comp_rx.recv() {
                    Ok(NetToCompositor::ClientConnected { hello, reply }) => {
                        assert_eq!(hello.token.as_deref(), Some("session-token"));
                        reply
                            .send(ConnectDecision::Bind(SessionBinding {
                                session_id: 314,
                                resumed: true,
                                token: "session-token".into(),
                            }))
                            .unwrap();
                    }
                    Ok(NetToCompositor::ClientDisconnected) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });

        // Server side: accept the connection and run handle_connection.
        // The compositor channels are not `Send`, so we drive the server side
        // on this task via `join!` rather than `tokio::spawn`.
        let cfg = ServerConfig::default();
        let active = Arc::new(AtomicU32::new(0));
        let tokens: LocalTokens = Arc::new(Mutex::new(HashSet::new()));
        let server_fut = async {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();
            let _ = handle_connection(
                &connection,
                &cfg,
                &comp_to_net_rx,
                &net_to_comp_tx,
                &active,
                &tokens,
            )
            .await;
            let _ = net_to_comp_tx.send(NetToCompositor::ClientDisconnected);
            endpoint.close(0u32.into(), b"done");
        };

        // Client side: connect, send ClientHello, read ServerHello.
        let client_fut = async {
            let client_endpoint = build_test_client_endpoint();
            let connection = client_endpoint
                .connect(actual_addr, "localhost")
                .unwrap()
                .await
                .unwrap();
            let (mut control_send, mut control_recv) = connection.open_bi().await.unwrap();
            let client_hello = ControlMessage::ClientHello(ClientHello {
                version: wayray_protocol::PROTOCOL_VERSION,
                capabilities: vec![],
                token: Some("session-token".to_string()),
            });
            write_message(&mut control_send, &client_hello)
                .await
                .unwrap();

            let response: ControlMessage = read_message(&mut control_recv).await.unwrap();
            let ControlMessage::ServerHello(server_hello) = response else {
                panic!("expected ServerHello, got {response:?}");
            };
            // session_id must equal the value the fake compositor replied with,
            // proving it is no longer hardcoded.
            assert_eq!(server_hello.session_id, 314);
            assert!(server_hello.resumed);

            // Because resumed == true, the server emits a Resumed event next.
            let next: ControlMessage = read_message(&mut control_recv).await.unwrap();
            assert!(matches!(
                next,
                ControlMessage::SessionEvent(SessionEvent::Resumed { session_id: 314 })
            ));

            // Tear down: drop client so handle_connection's relay loop ends.
            drop(control_send);
            drop(control_recv);
            drop(connection);
            let _ = comp_to_net_tx.send(CompositorToNet::Shutdown);
        };

        tokio::join!(server_fut, client_fut);
        let _ = fake_compositor.join();
    }

    /// A `ServerInfoRequest` over a loopback QUIC pair gets a `ServerInfo`
    /// reply carrying the configured server id, active session count, and
    /// capacity. Exercised end-to-end through `query_peer_info`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn server_info_round_trips() {
        let server_config = build_server_config();
        let endpoint =
            quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let actual_addr = endpoint.local_addr().unwrap();

        let (_comp_to_net_tx, comp_to_net_rx) = mpsc::channel::<CompositorToNet>();
        let (net_to_comp_tx, _net_to_comp_rx) = mpsc::channel::<NetToCompositor>();

        let cfg = ServerConfig {
            server_id: "b".to_string(),
            capacity: 50,
            ..ServerConfig::default()
        };
        let active = Arc::new(AtomicU32::new(7));
        let tokens: LocalTokens = Arc::new(Mutex::new(HashSet::new()));

        let server_fut = async {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();
            let _ = handle_connection(
                &connection,
                &cfg,
                &comp_to_net_rx,
                &net_to_comp_tx,
                &active,
                &tokens,
            )
            .await;
            // Keep the connection alive until the client has finished reading
            // the buffered ServerInfo, then let it close cleanly on drop.
            connection.closed().await;
        };

        let client_fut = async {
            let info = query_peer_info(actual_addr).await.expect("ServerInfo");
            assert_eq!(info.server_id, "b");
            assert_eq!(info.active_sessions, 7);
            assert_eq!(info.capacity, 50);
        };

        tokio::join!(server_fut, client_fut);
    }

    /// A `SessionLookupRequest` over a loopback QUIC pair is answered from the
    /// server's published token set. A peer probing for a token the server
    /// hosts gets its server id back (cross-server affinity discovery); a token
    /// it does not host yields `None`. Exercised end-to-end through
    /// `query_peer_for_token`, the runtime data source that feeds
    /// `note_remote_session`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn session_lookup_round_trips() {
        let server_config = build_server_config();
        let endpoint =
            quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let actual_addr = endpoint.local_addr().unwrap();

        let (_comp_to_net_tx, comp_to_net_rx) = mpsc::channel::<CompositorToNet>();
        let (net_to_comp_tx, _net_to_comp_rx) = mpsc::channel::<NetToCompositor>();

        let cfg = ServerConfig {
            server_id: "a".to_string(),
            capacity: 50,
            ..ServerConfig::default()
        };
        let active = Arc::new(AtomicU32::new(1));
        // Server `a` hosts a resumable session for "token-T".
        let tokens: LocalTokens = Arc::new(Mutex::new(HashSet::from(["token-T".to_string()])));

        // Two probes arrive on this connection-accepting server: one for a
        // hosted token, one for an unknown token. Each probe is its own
        // short-lived connection, so accept twice.
        let server_fut = async {
            for _ in 0..2 {
                let incoming = endpoint.accept().await.unwrap();
                let connection = incoming.await.unwrap();
                let _ = handle_connection(
                    &connection,
                    &cfg,
                    &comp_to_net_rx,
                    &net_to_comp_tx,
                    &active,
                    &tokens,
                )
                .await;
                connection.closed().await;
            }
        };

        let client_fut = async {
            let hit = query_peer_for_token(actual_addr, "token-T").await;
            assert_eq!(hit.as_deref(), Some("a"));

            let miss = query_peer_for_token(actual_addr, "nope").await;
            assert_eq!(miss, None);
        };

        tokio::join!(server_fut, client_fut);
    }

    /// When the compositor replies with `ConnectDecision::Redirect`, the client
    /// receives a `SessionRedirect` carrying the expected target id and addr.
    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn redirect_decision_sends_session_redirect() {
        let server_config = build_server_config();
        let endpoint =
            quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let actual_addr = endpoint.local_addr().unwrap();

        let (comp_to_net_tx, comp_to_net_rx) = mpsc::channel::<CompositorToNet>();
        let (net_to_comp_tx, net_to_comp_rx) = mpsc::channel::<NetToCompositor>();

        let fake_compositor = std::thread::spawn(move || {
            loop {
                match net_to_comp_rx.recv() {
                    Ok(NetToCompositor::ClientConnected { reply, .. }) => {
                        reply
                            .send(ConnectDecision::Redirect {
                                server_id: "a".to_string(),
                                addr: "10.0.0.1:4433".to_string(),
                            })
                            .unwrap();
                    }
                    Ok(NetToCompositor::ClientDisconnected) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });

        let cfg = ServerConfig::default();
        let active = Arc::new(AtomicU32::new(0));
        let tokens: LocalTokens = Arc::new(Mutex::new(HashSet::new()));
        let server_fut = async {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();
            let _ = handle_connection(
                &connection,
                &cfg,
                &comp_to_net_rx,
                &net_to_comp_tx,
                &active,
                &tokens,
            )
            .await;
            let _ = net_to_comp_tx.send(NetToCompositor::ClientDisconnected);
            // Keep the connection alive until the client reads the buffered
            // SessionRedirect, then close on drop.
            connection.closed().await;
        };

        let client_fut = async {
            let client_endpoint = build_test_client_endpoint();
            let connection = client_endpoint
                .connect(actual_addr, "localhost")
                .unwrap()
                .await
                .unwrap();
            let (mut control_send, mut control_recv) = connection.open_bi().await.unwrap();
            let client_hello = ControlMessage::ClientHello(ClientHello {
                version: wayray_protocol::PROTOCOL_VERSION,
                capabilities: vec![],
                token: Some("remote-token".to_string()),
            });
            write_message(&mut control_send, &client_hello)
                .await
                .unwrap();

            let response: ControlMessage = read_message(&mut control_recv).await.unwrap();
            match response {
                ControlMessage::SessionRedirect { server_id, addr } => {
                    assert_eq!(server_id, "a");
                    assert_eq!(addr, "10.0.0.1:4433");
                }
                other => panic!("expected SessionRedirect, got {other:?}"),
            }

            drop(control_send);
            drop(control_recv);
            drop(connection);
            let _ = comp_to_net_tx.send(CompositorToNet::Shutdown);
        };

        tokio::join!(server_fut, client_fut);
        let _ = fake_compositor.join();
    }
}
