//! QUIC transport client for wrclient.
//!
//! Connects to a wrsrvd server, establishes three logical streams:
//! - **Control** (bidirectional): handshake, ping/pong, frame acks
//! - **Display** (server→client, unidirectional): frame updates
//! - **Input** (client→server, unidirectional): keyboard/pointer events
//!
//! ## Quinn stream semantics
//!
//! Quinn streams are lazily materialized on the wire: the peer's
//! `accept_bi()`/`accept_uni()` won't resolve until data is written.
//! The handshake protocol accounts for this by having each side write
//! data before the other side tries to accept.

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};

use tracing::info;
use wayray_protocol::codec;
use wayray_protocol::messages::{
    ClientHello, ControlMessage, DisplayMessage, InputMessage, ServerHello,
};
pub use wayray_protocol::tls::PinStore;
use wayray_protocol::tls::verifier::{PinPolicy, PinnedServerCertVerifier};

/// Configuration for the QUIC client connection.
pub struct ClientConfig {
    /// Server address to connect to (IP:port or hostname:port).
    pub server_addr: SocketAddr,
    /// Hostname for TLS SNI (used in QUIC connect).
    pub server_name: String,
    /// Client capabilities to advertise in the hello.
    pub capabilities: Vec<String>,
    /// Session token for session binding. If None, no token is sent.
    pub token: Option<String>,
    /// Expected server certificate fingerprint (`sha256:<hex>`). When set, the
    /// connection is pinned strictly to it. When `None`, trust-on-first-use
    /// against `pin_store` applies.
    pub expected_fingerprint: Option<String>,
    /// Shared trust-on-first-use pin store. Required when no fingerprint is
    /// configured so the client never blindly accepts an unknown certificate.
    pub pin_store: Arc<Mutex<PinStore>>,
}

/// Resolve a "host:port" string to a SocketAddr, supporting both
/// IP addresses and DNS hostnames.
pub fn resolve_server_addr(addr_str: &str) -> Result<(SocketAddr, String), String> {
    // Try parsing as a direct SocketAddr first (e.g., "192.168.1.1:4433").
    if let Ok(addr) = addr_str.parse::<SocketAddr>() {
        return Ok((addr, "localhost".to_string()));
    }

    // Otherwise treat as hostname:port and resolve via DNS.
    let addrs: Vec<SocketAddr> = addr_str
        .to_socket_addrs()
        .map_err(|e| format!("failed to resolve '{}': {}", addr_str, e))?
        .collect();

    let addr = addrs
        .first()
        .ok_or_else(|| format!("no addresses found for '{}'", addr_str))?;

    // Extract hostname (everything before the last ':port').
    let hostname = addr_str
        .rsplit_once(':')
        .map(|(h, _)| h.to_string())
        .unwrap_or_else(|| addr_str.to_string());

    Ok((*addr, hostname))
}

/// Outcome of a connection attempt: either an established session or a
/// redirect to another server (multi-server affinity / load balancing).
pub enum ConnectOutcome {
    /// The server accepted the session; streams are ready.
    Connected(quinn::Endpoint, ServerConnection),
    /// The server redirected us to another address. The client should
    /// reconnect there with the same token.
    Redirect {
        /// Target server id (for logging).
        server_id: String,
        /// Target server address as `host:port`.
        addr: String,
    },
}

/// An established connection to a wrsrvd server with all streams ready.
///
/// After `connect()`, the control stream and input stream are ready to use.
/// The display stream is accepted lazily when the server sends the first frame.
pub struct ServerConnection {
    /// The underlying QUIC connection.
    pub connection: quinn::Connection,
    /// Bidirectional control stream -- send side.
    pub control_send: quinn::SendStream,
    /// Bidirectional control stream -- receive side.
    pub control_recv: quinn::RecvStream,
    /// Unidirectional input stream to server (send only).
    pub input_send: quinn::SendStream,
    /// The server hello received during handshake.
    pub server_hello: ServerHello,
}

impl ServerConnection {
    /// Send a frame acknowledgment to the server.
    pub async fn send_frame_ack(
        &mut self,
        sequence: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let msg = ControlMessage::FrameAck(wayray_protocol::messages::FrameAck { sequence });
        write_message(&mut self.control_send, &msg).await
    }

    /// Ask the server for a full keyframe to resynchronize after detecting the
    /// reconstructed framebuffer has drifted (a frame-checksum mismatch).
    pub async fn request_keyframe(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        write_message(&mut self.control_send, &ControlMessage::RequestKeyframe).await
    }

    /// Send an input message to the server.
    pub async fn send_input(
        &mut self,
        input: &InputMessage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        write_message(&mut self.input_send, input).await
    }

    /// Accept the display stream from the server and read the next frame.
    ///
    /// On first call, accepts the unidirectional stream from the server.
    /// The server triggers this by writing the first frame update.
    pub async fn accept_display_stream(
        &mut self,
    ) -> Result<quinn::RecvStream, Box<dyn std::error::Error>> {
        let recv = self.connection.accept_uni().await?;
        info!("display stream accepted");
        Ok(recv)
    }

    /// Read the next control message from the server.
    pub async fn recv_control(&mut self) -> Result<ControlMessage, Box<dyn std::error::Error>> {
        read_message(&mut self.control_recv).await
    }

    /// Send a pong response.
    pub async fn send_pong(&mut self, timestamp: u64) -> Result<(), Box<dyn std::error::Error>> {
        let msg = ControlMessage::Pong(wayray_protocol::messages::Pong { timestamp });
        write_message(&mut self.control_send, &msg).await
    }
}

/// Build a quinn client config that pins the server certificate.
///
/// A configured fingerprint is enforced strictly; otherwise the connection is
/// trusted on first use and pinned in the shared store. Either way the server
/// can no longer be silently impersonated by a man-in-the-middle (which would
/// otherwise harvest the session token sent in the `ClientHello`).
fn build_client_config(config: &ClientConfig) -> quinn::ClientConfig {
    let policy = match &config.expected_fingerprint {
        Some(fp) => PinPolicy::Fixed(fp.clone()),
        None => PinPolicy::Tofu {
            identity: config.server_addr.to_string(),
            store: config.pin_store.clone(),
        },
    };
    let verifier = Arc::new(PinnedServerCertVerifier::new(policy));

    let provider = rustls::crypto::ring::default_provider();
    let crypto = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .expect("TLS protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
        .expect("QUIC client crypto config");
    quinn::ClientConfig::new(std::sync::Arc::new(crypto))
}

/// Connect to a wrsrvd server and perform the handshake.
///
/// Returns a `ServerConnection` with the control and input streams ready.
/// The display stream is accepted lazily via `accept_display_stream()`.
///
/// The caller must keep the returned `quinn::Endpoint` alive for the
/// duration of the connection.
pub async fn connect(config: &ClientConfig) -> Result<ConnectOutcome, Box<dyn std::error::Error>> {
    let client_config = build_client_config(config);

    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse::<SocketAddr>()?)?;
    endpoint.set_default_client_config(client_config);

    info!(server = %config.server_addr, "connecting to wrsrvd");
    let connection = endpoint
        .connect(config.server_addr, &config.server_name)?
        .await?;
    info!("QUIC connection established");

    // Open control stream (bidirectional) and immediately send ClientHello.
    // Writing data triggers the server's accept_bi().
    let (mut control_send, mut control_recv) = connection.open_bi().await?;
    info!("control stream opened");

    let client_hello = ControlMessage::ClientHello(ClientHello {
        version: wayray_protocol::PROTOCOL_VERSION,
        capabilities: config.capabilities.clone(),
        token: config.token.clone(),
    });
    write_message(&mut control_send, &client_hello).await?;
    info!("sent ClientHello");

    // Read the server's response: ServerHello (accepted) or SessionRedirect
    // (our session lives elsewhere / load balancing).
    let response: ControlMessage = read_message(&mut control_recv).await?;
    let server_hello = match response {
        ControlMessage::ServerHello(hello) => {
            // The token is a session credential — never log it in the clear.
            info!(
                version = hello.version,
                session_id = hello.session_id,
                width = hello.output_width,
                height = hello.output_height,
                resumed = hello.resumed,
                "received ServerHello"
            );
            hello
        }
        ControlMessage::SessionRedirect { server_id, addr } => {
            info!(%server_id, %addr, "server redirected us to a peer");
            return Ok(ConnectOutcome::Redirect { server_id, addr });
        }
        other => {
            return Err(format!("expected ServerHello, got {other:?}").into());
        }
    };

    // Open input stream (unidirectional to server).
    // Data written later via send_input() triggers the server's accept_uni().
    let input_send = connection.open_uni().await?;
    info!("input stream opened");

    Ok(ConnectOutcome::Connected(
        endpoint,
        ServerConnection {
            connection,
            control_send,
            control_recv,
            input_send,
            server_hello,
        },
    ))
}

/// Read a length-prefixed message from a QUIC receive stream.
async fn read_message<T: serde::de::DeserializeOwned>(
    recv: &mut quinn::RecvStream,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

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

/// Read a frame update from a display receive stream.
pub async fn read_display_message(
    recv: &mut quinn::RecvStream,
) -> Result<DisplayMessage, Box<dyn std::error::Error>> {
    read_message(recv).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use quinn::rustls::pki_types::CertificateDer;
    use wayray_protocol::messages::{FrameUpdate, KeyState, KeyboardEvent};

    /// A throwaway trust-on-first-use pin store backed by a unique temp file, so
    /// tests pin the self-signed test cert without touching the real
    /// `known_hosts` or accepting arbitrary certificates.
    fn temp_pin_store(tag: &str) -> Arc<Mutex<PinStore>> {
        let path =
            std::env::temp_dir().join(format!("wayray-test-pins-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Arc::new(Mutex::new(PinStore::open(path).unwrap()))
    }

    /// Helper: start a test server on an ephemeral port, return its address.
    async fn start_test_server() -> (quinn::Endpoint, SocketAddr) {
        let CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "wayray".to_string()])
                .unwrap();
        let cert_der = CertificateDer::from(cert);
        let key_der =
            quinn::rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap();

        let provider = rustls::crypto::ring::default_provider();
        let crypto = rustls::ServerConfig::builder_with_provider(provider.into())
            .with_safe_default_protocol_versions()
            .expect("TLS protocol versions")
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();
        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(crypto).unwrap();
        let server_config = quinn::ServerConfig::with_crypto(std::sync::Arc::new(crypto));

        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let endpoint = quinn::Endpoint::server(server_config, addr).unwrap();
        let actual_addr = endpoint.local_addr().unwrap();
        (endpoint, actual_addr)
    }

    use rcgen::CertifiedKey;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_connect_and_handshake() {
        let (endpoint, addr) = start_test_server().await;

        // Server side: accept bi (triggered by client's ClientHello write),
        // read ClientHello, send ServerHello.
        let server = tokio::spawn(async move {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();

            let (mut control_send, mut control_recv) = connection.accept_bi().await.unwrap();

            let hello: ControlMessage = read_message(&mut control_recv).await.unwrap();
            assert!(matches!(hello, ControlMessage::ClientHello(_)));

            let server_hello = ControlMessage::ServerHello(ServerHello {
                version: wayray_protocol::PROTOCOL_VERSION,
                session_id: 99,
                output_width: 1920,
                output_height: 1080,
                resumed: false,
                token: String::new(),
            });
            write_message(&mut control_send, &server_hello)
                .await
                .unwrap();

            // Wait for the client to finish setup (it opens input uni).
            // Read next control message or wait for disconnect.
            let _ = read_message::<ControlMessage>(&mut control_recv).await;

            endpoint.close(0u32.into(), b"done");
        });

        let config = ClientConfig {
            server_addr: addr,
            server_name: "localhost".to_string(),
            capabilities: vec!["test".to_string()],
            token: Some("test-token".to_string()),
            expected_fingerprint: None,
            pin_store: temp_pin_store("handshake"),
        };

        let (_endpoint, mut conn) = match connect(&config).await.unwrap() {
            ConnectOutcome::Connected(ep, conn) => (ep, conn),
            ConnectOutcome::Redirect { .. } => panic!("unexpected redirect"),
        };
        assert_eq!(conn.server_hello.session_id, 99);
        assert_eq!(conn.server_hello.output_width, 1920);

        // Send a ping so the server's read resolves and it can shut down.
        let _ = conn.send_pong(0).await;

        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_send_input_receive_frame() {
        let (endpoint, addr) = start_test_server().await;

        let server = tokio::spawn(async move {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();

            // Handshake on control stream.
            let (mut control_send, mut control_recv) = connection.accept_bi().await.unwrap();
            let _: ControlMessage = read_message(&mut control_recv).await.unwrap();
            let server_hello = ControlMessage::ServerHello(ServerHello {
                version: wayray_protocol::PROTOCOL_VERSION,
                session_id: 1,
                output_width: 800,
                output_height: 600,
                resumed: false,
                token: String::new(),
            });
            write_message(&mut control_send, &server_hello)
                .await
                .unwrap();

            // Send frame on display uni (triggers client's accept_uni).
            let mut display_send = connection.open_uni().await.unwrap();
            let frame = DisplayMessage::FrameUpdate(FrameUpdate {
                sequence: 7,
                regions: vec![],
                checksum: 0,
            });
            write_message(&mut display_send, &frame).await.unwrap();

            // Accept input uni (triggered by client writing input).
            let mut input_recv = connection.accept_uni().await.unwrap();
            let input: InputMessage = read_message(&mut input_recv).await.unwrap();
            assert!(matches!(input, InputMessage::Keyboard(_)));

            endpoint.close(0u32.into(), b"done");
        });

        let config = ClientConfig {
            server_addr: addr,
            server_name: "localhost".to_string(),
            capabilities: vec![],
            token: None,
            expected_fingerprint: None,
            pin_store: temp_pin_store("frame"),
        };

        let (_endpoint, mut conn) = match connect(&config).await.unwrap() {
            ConnectOutcome::Connected(ep, conn) => (ep, conn),
            ConnectOutcome::Redirect { .. } => panic!("unexpected redirect"),
        };

        // Send input (triggers server's accept_uni for input stream).
        let input = InputMessage::Keyboard(KeyboardEvent {
            keycode: 42,
            state: KeyState::Pressed,
            time: 1000,
        });
        conn.send_input(&input).await.unwrap();

        // Accept display stream (triggered by server writing frame).
        let mut display_recv = conn.accept_display_stream().await.unwrap();
        let frame = read_display_message(&mut display_recv).await.unwrap();
        let DisplayMessage::FrameUpdate(update) = frame;
        assert_eq!(update.sequence, 7);

        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_handles_session_redirect() {
        let (endpoint, addr) = start_test_server().await;

        // Server side: read ClientHello, reply with a SessionRedirect.
        let server = tokio::spawn(async move {
            let incoming = endpoint.accept().await.unwrap();
            let connection = incoming.await.unwrap();
            let (mut control_send, mut control_recv) = connection.accept_bi().await.unwrap();
            let _: ControlMessage = read_message(&mut control_recv).await.unwrap();

            let redirect = ControlMessage::SessionRedirect {
                server_id: "a".to_string(),
                addr: "10.0.0.1:4433".to_string(),
            };
            write_message(&mut control_send, &redirect).await.unwrap();
            let _ = control_send.finish();
            // Give the client a moment to read before closing.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            endpoint.close(0u32.into(), b"done");
        });

        let config = ClientConfig {
            server_addr: addr,
            server_name: "localhost".to_string(),
            capabilities: vec![],
            token: Some("remote-token".to_string()),
            expected_fingerprint: None,
            pin_store: temp_pin_store("redirect"),
        };

        match connect(&config).await.unwrap() {
            ConnectOutcome::Redirect { server_id, addr } => {
                assert_eq!(server_id, "a");
                assert_eq!(addr, "10.0.0.1:4433");
            }
            ConnectOutcome::Connected(..) => panic!("expected redirect"),
        }

        server.await.unwrap();
    }
}
