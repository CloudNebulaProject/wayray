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

use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;

use quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rcgen::CertifiedKey;
use tokio::runtime::Runtime;
use tracing::{error, info, warn};
use wayray_protocol::codec;
use wayray_protocol::messages::{
    ClientHello, ControlMessage, DisplayMessage, FrameUpdate, InputMessage, ServerHello,
    SessionEvent,
};

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

/// Messages sent from the network thread to the compositor.
pub enum NetToCompositor {
    /// An input event received from the client.
    Input(InputMessage),
    /// A control message (e.g., FrameAck) from the client.
    Control(ControlMessage),
    /// Client connected with the given hello. The compositor resolves the
    /// session via the registry and replies with a `SessionBinding` so the
    /// network thread can fill in the real session id / resumed flag.
    ClientConnected {
        hello: ClientHello,
        reply: mpsc::Sender<SessionBinding>,
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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:4433".parse().unwrap(),
            output_width: 1280,
            output_height: 720,
        }
    }
}

/// Handle to a running network server thread.
pub struct NetworkHandle {
    /// Send commands to the network thread.
    pub tx: mpsc::Sender<CompositorToNet>,
    /// Receive events from the network thread.
    pub rx: mpsc::Receiver<NetToCompositor>,
    /// Join handle for the background thread.
    join: Option<thread::JoinHandle<()>>,
}

impl NetworkHandle {
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

    let join = thread::Builder::new()
        .name("wayray-net".into())
        .spawn(move || {
            let rt = Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                if let Err(e) = server_loop(config, net_rx, net_tx).await {
                    error!("network server error: {e}");
                }
            });
        })
        .expect("spawn network thread");

    NetworkHandle {
        tx: comp_tx,
        rx: comp_rx,
        join: Some(join),
    }
}

/// Main server accept loop.
async fn server_loop(
    config: ServerConfig,
    compositor_rx: mpsc::Receiver<CompositorToNet>,
    compositor_tx: mpsc::Sender<NetToCompositor>,
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

        if let Err(e) =
            handle_connection(&connection, &config, &compositor_rx, &compositor_tx).await
        {
            warn!("client session ended: {e}");
        }

        let _ = compositor_tx.send(NetToCompositor::ClientDisconnected);
        info!("client disconnected, waiting for next connection");
    }

    endpoint.close(0u32.into(), b"shutdown");
    Ok(())
}

/// Await a `SessionBinding` reply from the compositor, polling the blocking
/// `std::sync::mpsc` channel from this async context with a short interval and
/// a hard deadline. Returns `None` on timeout or if the sender is dropped.
async fn recv_binding_with_timeout(
    rx: mpsc::Receiver<SessionBinding>,
    timeout: std::time::Duration,
) -> Option<SessionBinding> {
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
) -> Result<(), Box<dyn std::error::Error>> {
    // Step 1: Accept control stream. The client writes ClientHello immediately
    // after opening, so accept_bi resolves once that data arrives.
    let (mut control_send, mut control_recv) = connection.accept_bi().await?;
    info!("control stream established");

    // Step 2: Read ClientHello, send ServerHello.
    let client_hello: ControlMessage = read_message(&mut control_recv).await?;
    let ControlMessage::ClientHello(hello) = client_hello else {
        return Err(format!("expected ClientHello, got {client_hello:?}").into());
    };
    info!(version = hello.version, "received ClientHello");
    let fallback_token = hello.token.clone().unwrap_or_default();

    // Round-trip to the compositor: it resolves the session via the registry
    // and replies with the real session id / resumed flag. We bound the wait
    // so a stalled compositor cannot wedge the handshake; on timeout we fall
    // back to a best-effort ServerHello.
    let (reply_tx, reply_rx) = mpsc::channel::<SessionBinding>();
    let connect_start = std::time::Instant::now();
    let _ = compositor_tx.send(NetToCompositor::ClientConnected {
        hello,
        reply: reply_tx,
    });

    let binding = recv_binding_with_timeout(reply_rx, std::time::Duration::from_millis(250)).await;
    let (session_id, resumed, token) = match binding {
        Some(b) => {
            info!(
                session_id = b.session_id,
                resumed = b.resumed,
                elapsed_ms = connect_start.elapsed().as_millis() as u64,
                "session binding resolved"
            );
            (b.session_id, b.resumed, b.token)
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
        let (_tx, rx) = mpsc::channel::<SessionBinding>();
        let binding = recv_binding_with_timeout(rx, std::time::Duration::from_millis(20)).await;
        assert!(binding.is_none());
    }

    #[tokio::test]
    async fn binding_received_before_timeout() {
        let (tx, rx) = mpsc::channel::<SessionBinding>();
        tx.send(SessionBinding {
            session_id: 7,
            resumed: true,
            token: "t".into(),
        })
        .unwrap();
        let binding = recv_binding_with_timeout(rx, std::time::Duration::from_millis(250))
            .await
            .expect("binding should arrive");
        assert_eq!(binding.session_id, 7);
        assert!(binding.resumed);
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
                            .send(SessionBinding {
                                session_id: 314,
                                resumed: true,
                                token: "session-token".into(),
                            })
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
        let server_fut = async {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();
            let _ = handle_connection(&connection, &cfg, &comp_to_net_rx, &net_to_comp_tx).await;
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
}
