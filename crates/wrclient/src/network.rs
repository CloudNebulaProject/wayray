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

use std::net::SocketAddr;

use quinn::rustls::pki_types::CertificateDer;
use tracing::info;
use wayray_protocol::codec;
use wayray_protocol::messages::{
    ClientHello, ControlMessage, DisplayMessage, InputMessage, ServerHello,
};

/// Configuration for the QUIC client connection.
pub struct ClientConfig {
    /// Server address to connect to.
    pub server_addr: SocketAddr,
    /// Client capabilities to advertise in the hello.
    pub capabilities: Vec<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_addr: "127.0.0.1:4433".parse().unwrap(),
            capabilities: vec!["display".to_string()],
        }
    }
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

/// Dummy certificate verifier that accepts any server cert.
///
/// Used during development when servers use self-signed certificates.
/// TODO: Replace with proper certificate pinning or CA verification.
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

/// Build a quinn client config that skips certificate verification.
fn build_client_config() -> quinn::ClientConfig {
    let provider = rustls::crypto::ring::default_provider();
    let crypto = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .expect("TLS protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(SkipServerVerification))
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
pub async fn connect(
    config: &ClientConfig,
) -> Result<(quinn::Endpoint, ServerConnection), Box<dyn std::error::Error>> {
    let client_config = build_client_config();

    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse::<SocketAddr>()?)?;
    endpoint.set_default_client_config(client_config);

    info!(server = %config.server_addr, "connecting to wrsrvd");
    let connection = endpoint.connect(config.server_addr, "localhost")?.await?;
    info!("QUIC connection established");

    // Open control stream (bidirectional) and immediately send ClientHello.
    // Writing data triggers the server's accept_bi().
    let (mut control_send, mut control_recv) = connection.open_bi().await?;
    info!("control stream opened");

    let client_hello = ControlMessage::ClientHello(ClientHello {
        version: wayray_protocol::PROTOCOL_VERSION,
        capabilities: config.capabilities.clone(),
    });
    write_message(&mut control_send, &client_hello).await?;
    info!("sent ClientHello");

    // Read ServerHello response.
    let response: ControlMessage = read_message(&mut control_recv).await?;
    let server_hello = match response {
        ControlMessage::ServerHello(hello) => {
            info!(
                version = hello.version,
                session_id = hello.session_id,
                width = hello.output_width,
                height = hello.output_height,
                "received ServerHello"
            );
            hello
        }
        other => {
            return Err(format!("expected ServerHello, got {other:?}").into());
        }
    };

    // Open input stream (unidirectional to server).
    // Data written later via send_input() triggers the server's accept_uni().
    let input_send = connection.open_uni().await?;
    info!("input stream opened");

    Ok((
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
    use wayray_protocol::messages::{FrameUpdate, KeyState, KeyboardEvent};

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
            capabilities: vec!["test".to_string()],
        };

        let (_endpoint, mut conn) = connect(&config).await.unwrap();
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
            });
            write_message(&mut control_send, &server_hello)
                .await
                .unwrap();

            // Send frame on display uni (triggers client's accept_uni).
            let mut display_send = connection.open_uni().await.unwrap();
            let frame = DisplayMessage::FrameUpdate(FrameUpdate {
                sequence: 7,
                regions: vec![],
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
            capabilities: vec![],
        };

        let (_endpoint, mut conn) = connect(&config).await.unwrap();

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
}
