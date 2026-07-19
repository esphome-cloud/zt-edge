/// ESPHome Native API — Noise transport helpers.
///
/// The ESPHome Noise path uses a different wire framing from the cleartext path:
///   `0x01 | uint16_be(frame_len) | noise_payload`
///
/// The Noise protocol is `Noise_NNpsk0_25519_ChaChaPoly_SHA256`:
/// - NN  = no static key for either party (ephemeral-only DH)
/// - psk0 = pre-shared key mixed in at handshake step 0
/// - 25519 = X25519 Diffie-Hellman
/// - ChaChaPoly = ChaCha20-Poly1305 AEAD cipher
/// - SHA256 = BLAKE2s hash (Noise spec maps SHA256 label to BLAKE2s in this context)
///
/// ## Handshake flow (initiator = rshome-ha, responder = ESPHome device)
///
/// 1. Initiator sends `ClientHello`: `0x01 | uint16_be(len) | noise_msg_1`
/// 2. Responder sends `ServerHello`: `0x01 | uint16_be(len) | noise_msg_2`
/// 3. Handshake complete; all subsequent frames are Noise-encrypted application data.
///
/// ## Application frame format (post-handshake)
///
/// Each frame: `0x01 | uint16_be(ciphertext_len) | ciphertext`
/// Decrypted plaintext: `uint16_be(msg_type) | protobuf_payload`
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

const NOISE_PREAMBLE: u8 = 0x01;
const NOISE_PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_SHA256";

/// Shared Noise transport state used by both the reader task and the writer.
pub type SharedNoise = Arc<Mutex<snow::TransportState>>;

/// Perform the Noise NNpsk0 handshake as the **initiator** (client side).
///
/// Reads the server preamble to detect whether the server is using Noise or cleartext.
/// Returns the shared `TransportState` ready for application-level message exchange,
/// or an `io::Error` if the handshake failed (wrong PSK, protocol mismatch, I/O error).
pub async fn noise_handshake(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    psk: &[u8; 32],
) -> io::Result<SharedNoise> {
    let builder = snow::Builder::new(
        NOISE_PATTERN
            .parse()
            .map_err(|e| io::Error::other(format!("Noise pattern: {e}")))?,
    )
    .psk(0, psk);

    let mut handshake = builder
        .build_initiator()
        .map_err(|e| io::Error::other(format!("Noise init: {e}")))?;

    // ── Message 1: Initiator → Responder (e, psk) ─────────────────────────────
    let mut msg_buf = vec![0u8; 65535];
    let msg1_len = handshake
        .write_message(&[], &mut msg_buf)
        .map_err(|e| io::Error::other(format!("Noise write msg1: {e}")))?;

    // ClientHello: 0x01 | uint16_be(len) | message
    writer.write_u8(NOISE_PREAMBLE).await?;
    writer.write_u16(msg1_len as u16).await?;
    writer.write_all(&msg_buf[..msg1_len]).await?;
    writer.flush().await?;

    // ── Message 2: Responder → Initiator (e, ee) ──────────────────────────────
    let preamble = reader.read_u8().await?;
    if preamble != NOISE_PREAMBLE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected Noise preamble 0x{NOISE_PREAMBLE:02x}, got 0x{preamble:02x}"),
        ));
    }

    let msg2_len = reader.read_u16().await? as usize;
    let mut msg2 = vec![0u8; msg2_len];
    reader.read_exact(&mut msg2).await?;

    let mut out_buf = vec![0u8; 65535];
    handshake.read_message(&msg2, &mut out_buf).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Noise handshake failed (wrong PSK?): {e}"),
        )
    })?;

    // ── Enter transport mode ───────────────────────────────────────────────────
    let transport = handshake
        .into_transport_mode()
        .map_err(|e| io::Error::other(format!("Noise transport mode: {e}")))?;

    Ok(Arc::new(Mutex::new(transport)))
}

/// Encrypt and send one Native API frame over a Noise-encrypted TCP connection.
///
/// Plaintext layout: `uint16_be(msg_type) | protobuf_payload`
/// Wire layout: `0x01 | uint16_be(ciphertext_len) | ciphertext`
pub async fn noise_send_frame(
    writer: &mut OwnedWriteHalf,
    noise: &SharedNoise,
    msg_type: u32,
    payload: &[u8],
) -> io::Result<()> {
    // Build plaintext: 2-byte msg_type (big-endian) + payload
    let mut plaintext = Vec::with_capacity(2 + payload.len());
    plaintext.extend_from_slice(&(msg_type as u16).to_be_bytes());
    plaintext.extend_from_slice(payload);

    // Allocate space for AEAD ciphertext (plaintext + 16-byte Poly1305 tag)
    let mut ciphertext = vec![0u8; plaintext.len() + 16];
    let ciphertext_len = noise
        .lock()
        .await
        .write_message(&plaintext, &mut ciphertext)
        .map_err(|e| io::Error::other(format!("Noise encrypt: {e}")))?;

    // Wire: 0x01 | uint16_be(len) | ciphertext
    writer.write_u8(NOISE_PREAMBLE).await?;
    writer.write_u16(ciphertext_len as u16).await?;
    writer.write_all(&ciphertext[..ciphertext_len]).await?;
    writer.flush().await
}

/// Decrypt and receive one Native API frame from a Noise-encrypted TCP connection.
///
/// Returns `(msg_type, payload)` on success, or `None` on clean EOF.
/// Returns `Some(Err(_))` on I/O or decryption failure.
pub async fn noise_recv_frame(
    reader: &mut OwnedReadHalf,
    noise: &SharedNoise,
) -> Option<io::Result<(u32, Vec<u8>)>> {
    let preamble = match reader.read_u8().await {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return None,
        Err(e) => return Some(Err(e)),
    };
    if preamble != NOISE_PREAMBLE {
        return Some(Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected Noise preamble 0x01, got 0x{preamble:02x}"),
        )));
    }

    let frame_len = match reader.read_u16().await {
        Ok(n) => n as usize,
        Err(e) => return Some(Err(e)),
    };

    let mut ciphertext = vec![0u8; frame_len];
    if let Err(e) = reader.read_exact(&mut ciphertext).await {
        return Some(Err(e));
    }

    // Plaintext = ciphertext_len − 16 (AEAD tag)
    let mut plaintext = vec![0u8; frame_len.saturating_sub(16)];
    let plaintext_len = match noise.lock().await.read_message(&ciphertext, &mut plaintext) {
        Ok(n) => n,
        Err(e) => {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Noise decrypt: {e}"),
            )))
        }
    };

    let plaintext = &plaintext[..plaintext_len];
    if plaintext.len() < 2 {
        return Some(Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Noise plaintext too short (missing msg_type header)",
        )));
    }

    let msg_type = u16::from_be_bytes([plaintext[0], plaintext[1]]) as u32;
    let payload = plaintext[2..].to_vec();
    Some(Ok((msg_type, payload)))
}

/// Build a Noise **responder** handshake for use in tests and simulation helpers.
///
/// Accepts a client connection, performs the `Noise_NNpsk0` server-side handshake,
/// and returns the shared transport state.  Returns an error if the handshake fails
/// (e.g. the client used a wrong PSK).
pub async fn noise_server_handshake(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    psk: &[u8; 32],
) -> io::Result<SharedNoise> {
    let builder = snow::Builder::new(
        NOISE_PATTERN
            .parse()
            .map_err(|e| io::Error::other(format!("Noise pattern: {e}")))?,
    )
    .psk(0, psk);

    let mut handshake = builder
        .build_responder()
        .map_err(|e| io::Error::other(format!("Noise responder init: {e}")))?;

    // ── Message 1: Initiator → Responder ──────────────────────────────────────
    let preamble = reader.read_u8().await?;
    if preamble != NOISE_PREAMBLE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected Noise preamble 0x01, got 0x{preamble:02x}"),
        ));
    }
    let msg1_len = reader.read_u16().await? as usize;
    let mut msg1 = vec![0u8; msg1_len];
    reader.read_exact(&mut msg1).await?;

    let mut out_buf = vec![0u8; 65535];
    handshake.read_message(&msg1, &mut out_buf).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Noise server read msg1: {e}"),
        )
    })?;

    // ── Message 2: Responder → Initiator ──────────────────────────────────────
    let msg2_len = handshake
        .write_message(&[], &mut out_buf)
        .map_err(|e| io::Error::other(format!("Noise server write msg2: {e}")))?;

    writer.write_u8(NOISE_PREAMBLE).await?;
    writer.write_u16(msg2_len as u16).await?;
    writer.write_all(&out_buf[..msg2_len]).await?;
    writer.flush().await?;

    let transport = handshake
        .into_transport_mode()
        .map_err(|e| io::Error::other(format!("Noise server transport mode: {e}")))?;

    Ok(Arc::new(Mutex::new(transport)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::{TcpListener, TcpStream};

    async fn random_listener() -> (TcpListener, u16) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        (l, port)
    }

    /// Build a known 32-byte PSK from a seed byte (all bytes equal to `seed`).
    fn psk(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    #[tokio::test]
    async fn noise_handshake_succeeds_with_matching_psk() {
        let (listener, port) = random_listener().await;

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = stream.into_split();
            let noise = noise_server_handshake(&mut r, &mut w, &psk(1))
                .await
                .unwrap();

            // Server sends one test frame
            noise_send_frame(&mut w, &noise, 42, b"hello from server")
                .await
                .unwrap();
        });

        let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let (mut r, mut w) = stream.into_split();
        let noise = noise_handshake(&mut r, &mut w, &psk(1)).await.unwrap();

        let (msg_type, payload) = noise_recv_frame(&mut r, &noise).await.unwrap().unwrap();
        assert_eq!(msg_type, 42);
        assert_eq!(payload, b"hello from server");
    }

    #[tokio::test]
    async fn noise_handshake_fails_with_wrong_psk() {
        let (listener, port) = random_listener().await;

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = stream.into_split();
            // Server uses the "correct" PSK
            let _ = noise_server_handshake(&mut r, &mut w, &psk(1)).await;
        });

        let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let (mut r, mut w) = stream.into_split();
        // Client uses a "wrong" PSK — handshake must fail
        let result = noise_handshake(&mut r, &mut w, &psk(0xFF)).await;
        assert!(result.is_err(), "handshake must fail with wrong PSK");
    }

    #[tokio::test]
    async fn noise_bidirectional_roundtrip() {
        let (listener, port) = random_listener().await;

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = stream.into_split();
            let noise = noise_server_handshake(&mut r, &mut w, &psk(7))
                .await
                .unwrap();

            // Echo back what the client sends
            if let Some(Ok((t, p))) = noise_recv_frame(&mut r, &noise).await {
                noise_send_frame(&mut w, &noise, t + 100, &p).await.unwrap();
            }
        });

        let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let (mut r, mut w) = stream.into_split();
        let noise = noise_handshake(&mut r, &mut w, &psk(7)).await.unwrap();

        noise_send_frame(&mut w, &noise, 1, b"ping").await.unwrap();
        let (msg_type, payload) = noise_recv_frame(&mut r, &noise).await.unwrap().unwrap();
        assert_eq!(msg_type, 101); // 1 + 100
        assert_eq!(payload, b"ping");
    }
}
