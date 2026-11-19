//! The client side of WebSocket (RFC 6455) that a pane's Codex server asks for.
//!
//! `codex app-server --listen unix://PATH` does not speak JSON lines on its socket: it accepts a
//! WebSocket on it and carries one JSON-RPC message per text frame
//! (`app-server-transport/src/transport/unix_socket.rs`, `websocket.rs` at rust-v0.170.0, and the
//! same at rust-v0.157.0). `codex app-server proxy --sock PATH`, which a WSL pane is reached
//! through, relays bytes as they are (`stdio-to-uds/src/lib.rs`), so its stdio carries the same
//! WebSocket. This is the little of the protocol that takes: the upgrade, text frames out (masked,
//! as a client's must be), text, ping and close frames in, fragments put back together. Hangar
//! never asks for compression (`x-codex-websocket-compression`), so Codex sends no binary frame.

use std::io;

use base64::Engine;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
};

/// The largest message Hangar reads, as Codex's own limit (64 MiB, tungstenite's default).
pub(super) const MAX_MESSAGE: usize = 64 << 20;

pub(super) const TEXT: u8 = 0x1;
const BINARY: u8 = 0x2;
const CONTINUATION: u8 = 0x0;
pub(super) const CLOSE: u8 = 0x8;
const PING: u8 = 0x9;
pub(super) const PONG: u8 = 0xA;

/// The longest line of the server's upgrade answer Hangar reads.
const MAX_HEADER_LINE: usize = 8 * 1024;

/// Upgrades the connection: Hangar's request, then the server's `101 Switching Protocols`.
///
/// The `Sec-WebSocket-Accept` hash is not checked: it needs SHA-1, which core does not carry, and
/// it guards against a server that does not speak WebSocket at all; the peer is the pane's own
/// Codex, on a socket in Hangar's runtime folder. The status and the `Upgrade` header are checked.
pub(super) async fn handshake<R, W>(read: &mut R, write: &mut W) -> io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let key = base64::engine::general_purpose::STANDARD.encode(uuid::Uuid::new_v4().as_bytes());
    // No `Origin`: Codex refuses an upgrade that carries one (`websocket.rs`,
    // `reject_requests_with_origin_header`). The path is any but `/daemon/shutdown`.
    let request = format!(
        "GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    write.write_all(request.as_bytes()).await?;
    write.flush().await?;

    let mut status = None;
    let mut upgraded = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        match read_line(read, &mut line, MAX_HEADER_LINE).await? {
            Line::Eof => return Err(refused("the socket closed during the WebSocket upgrade")),
            Line::TooLong => return Err(refused("an overlong line in the WebSocket upgrade")),
            Line::Read => {}
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches('\r');
        if status.is_none() {
            status = Some(text.to_string());
            continue;
        }
        if text.is_empty() {
            break;
        }
        if let Some((name, value)) = text.split_once(':') {
            if name.trim().eq_ignore_ascii_case("upgrade")
                && value.trim().eq_ignore_ascii_case("websocket")
            {
                upgraded = true;
            }
        }
    }
    let status = status.unwrap_or_default();
    if status.split_whitespace().nth(1) != Some("101") {
        return Err(refused(&format!(
            "the socket refused the WebSocket upgrade: {status}"
        )));
    }
    if !upgraded {
        return Err(refused(
            "the socket's answer to the upgrade is not a WebSocket",
        ));
    }
    Ok(())
}

fn refused(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// A frame from Hangar: final, masked with a fresh key.
pub(super) fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        out.push(0x80 | len as u8);
    } else if len <= usize::from(u16::MAX) {
        out.push(0x80 | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    // The first four bytes of a v4 UUID are random (its version and variant bits are later).
    let mut mask = [0u8; 4];
    mask.copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..4]);
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// What [`Reader::next`] read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Received {
    /// A whole text message.
    Text(Vec<u8>),
    /// A ping, whose payload goes back in a pong.
    Ping(Vec<u8>),
    /// A message that was read and dropped, and why.
    Skipped(String),
    /// The peer closed (a close frame, or the end of the stream).
    Closed,
}

/// Reads messages from the frames of one connection, keeping a fragmented message between frames.
#[derive(Debug, Default)]
pub(super) struct Reader {
    /// The opcode and bytes of a fragmented message so far.
    partial: Option<(u8, Vec<u8>)>,
    /// A fragmented message being thrown away (too large), until its last frame.
    discarding: bool,
}

impl Reader {
    pub(super) async fn next<R: AsyncRead + Unpin>(
        &mut self,
        read: &mut R,
    ) -> io::Result<Received> {
        loop {
            let mut head = [0u8; 2];
            match read.read_exact(&mut head).await {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(Received::Closed),
                Err(e) => return Err(e),
            }
            let fin = head[0] & 0x80 != 0;
            let opcode = head[0] & 0x0f;
            let masked = head[1] & 0x80 != 0;
            let len = match head[1] & 0x7f {
                126 => {
                    let mut b = [0u8; 2];
                    read.read_exact(&mut b).await?;
                    u64::from(u16::from_be_bytes(b))
                }
                127 => {
                    let mut b = [0u8; 8];
                    read.read_exact(&mut b).await?;
                    u64::from_be_bytes(b)
                }
                short => u64::from(short),
            };
            let mask = if masked {
                let mut m = [0u8; 4];
                read.read_exact(&mut m).await?;
                Some(m)
            } else {
                None
            };
            let so_far = self
                .partial
                .as_ref()
                .map_or(0, |(_, bytes)| bytes.len() as u64);
            if opcode < 0x8 && so_far.saturating_add(len) > MAX_MESSAGE as u64 {
                // Too large to hold: read past it, and past the rest of its message.
                let mut rest = (&mut *read).take(len);
                tokio::io::copy(&mut rest, &mut tokio::io::sink()).await?;
                self.partial = None;
                self.discarding = !fin;
                if fin {
                    return Ok(Received::Skipped(format!(
                        "a message larger than {} MiB",
                        MAX_MESSAGE >> 20
                    )));
                }
                continue;
            }
            if opcode >= 0x8 && len > 125 {
                return Err(refused("a WebSocket control frame longer than 125 bytes"));
            }
            let mut payload = vec![0u8; len as usize];
            read.read_exact(&mut payload).await?;
            if let Some(mask) = mask {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }
            match opcode {
                CLOSE => return Ok(Received::Closed),
                PING => return Ok(Received::Ping(payload)),
                PONG => continue,
                TEXT | BINARY => {
                    self.discarding = false;
                    if fin {
                        self.partial = None;
                        return Ok(whole(opcode, payload));
                    }
                    self.partial = Some((opcode, payload));
                }
                CONTINUATION => {
                    if self.discarding {
                        if fin {
                            self.discarding = false;
                            return Ok(Received::Skipped(format!(
                                "a message larger than {} MiB",
                                MAX_MESSAGE >> 20
                            )));
                        }
                        continue;
                    }
                    let Some((first, mut bytes)) = self.partial.take() else {
                        return Err(refused("a WebSocket continuation without a message"));
                    };
                    bytes.extend_from_slice(&payload);
                    if fin {
                        return Ok(whole(first, bytes));
                    }
                    self.partial = Some((first, bytes));
                }
                other => {
                    return Err(refused(&format!("an unknown WebSocket opcode {other:#x}")));
                }
            }
        }
    }
}

fn whole(opcode: u8, bytes: Vec<u8>) -> Received {
    if opcode == TEXT {
        Received::Text(bytes)
    } else {
        Received::Skipped("a binary message (Hangar asks for no compression)".into())
    }
}

/// What [`read_line`] read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Line {
    /// A line (without its `\n`), or the last bytes before the end of the stream.
    Read,
    /// A line longer than the limit, read past and dropped.
    TooLong,
    /// The end of the stream, nothing read.
    Eof,
}

/// Reads one line into `buf` (cleared by the caller), up to `limit` bytes; a longer one is read
/// past, to its end, and reported as [`Line::TooLong`] so the next one can be read.
pub(super) async fn read_line<R: AsyncBufRead + Unpin>(
    read: &mut R,
    buf: &mut Vec<u8>,
    limit: usize,
) -> io::Result<Line> {
    let mut too_long = false;
    loop {
        let available = read.fill_buf().await?;
        if available.is_empty() {
            return Ok(if too_long {
                Line::TooLong
            } else if buf.is_empty() {
                Line::Eof
            } else {
                Line::Read
            });
        }
        let (chunk, found) = match available.iter().position(|b| *b == b'\n') {
            Some(at) => (&available[..at], Some(at)),
            None => (available, None),
        };
        if !too_long {
            if buf.len() + chunk.len() > limit {
                too_long = true;
                buf.clear();
            } else {
                buf.extend_from_slice(chunk);
            }
        }
        let used = found.map_or(available.len(), |at| at + 1);
        read.consume(used);
        if found.is_some() {
            return Ok(if too_long { Line::TooLong } else { Line::Read });
        }
    }
}

/// A frame as a server writes it: not masked.
#[cfg(test)]
pub(super) fn server_frame(opcode: u8, fin: bool, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![if fin { 0x80 | opcode } else { opcode }];
    let len = payload.len();
    if len < 126 {
        out.push(len as u8);
    } else if len <= usize::from(u16::MAX) {
        out.push(126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn read_all(bytes: Vec<u8>) -> Vec<Received> {
        let mut reader = Reader::default();
        let mut input = bytes.as_slice();
        let mut out = Vec::new();
        loop {
            let got = reader.next(&mut input).await.unwrap();
            let end = got == Received::Closed;
            out.push(got);
            if end {
                return out;
            }
        }
    }

    #[tokio::test]
    async fn hangar_frames_are_masked_and_read_back() {
        for size in [0usize, 5, 125, 126, 65_535, 65_536, 200_000] {
            let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let bytes = frame(TEXT, &payload);
            assert_eq!(bytes[0], 0x81, "final text frame");
            assert_ne!(bytes[1] & 0x80, 0, "a client's frames are masked");
            let got = read_all(bytes).await;
            assert_eq!(
                got,
                vec![Received::Text(payload), Received::Closed],
                "{size}"
            );
        }
    }

    #[tokio::test]
    async fn fragments_pings_and_close_are_read_in_order() {
        let mut bytes = Vec::new();
        bytes.extend(server_frame(TEXT, false, b"{\"method\":"));
        // A control frame may come between the fragments of a message.
        bytes.extend(server_frame(PING, true, b"hi"));
        bytes.extend(server_frame(CONTINUATION, false, b"\"warn"));
        bytes.extend(server_frame(CONTINUATION, true, b"ing\"}"));
        bytes.extend(server_frame(PONG, true, b""));
        bytes.extend(server_frame(BINARY, true, b"\x28\xb5\x2f\xfd"));
        bytes.extend(server_frame(TEXT, true, b"{}"));
        bytes.extend(server_frame(CLOSE, true, &1000u16.to_be_bytes()));
        bytes.extend(server_frame(TEXT, true, b"never read"));
        let got = read_all(bytes).await;
        assert_eq!(
            got,
            vec![
                Received::Ping(b"hi".to_vec()),
                Received::Text(b"{\"method\":\"warning\"}".to_vec()),
                Received::Skipped("a binary message (Hangar asks for no compression)".into()),
                Received::Text(b"{}".to_vec()),
                Received::Closed,
            ]
        );
    }

    #[tokio::test]
    async fn the_end_of_the_stream_is_a_close() {
        assert_eq!(read_all(Vec::new()).await, vec![Received::Closed]);
    }

    #[tokio::test]
    async fn the_upgrade_needs_a_101_with_the_websocket_header() {
        let ok = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: x\r\nx-codex-websocket-max-unfragmented-message-bytes: 67108864\r\n\r\n";
        let mut answer = ok.to_vec();
        // The first frame can come in the same read as the answer: it stays for the reader.
        answer.extend(server_frame(TEXT, true, b"{}"));
        let mut read = tokio::io::BufReader::new(answer.as_slice());
        let mut sent = Vec::new();
        handshake(&mut read, &mut sent).await.unwrap();
        let request = String::from_utf8(sent).unwrap();
        assert!(request.starts_with("GET / HTTP/1.1\r\n"), "{request}");
        assert!(request.contains("\r\nUpgrade: websocket\r\n"), "{request}");
        assert!(
            request.contains("\r\nSec-WebSocket-Version: 13\r\n"),
            "{request}"
        );
        assert!(
            !request.to_ascii_lowercase().contains("origin:"),
            "{request}"
        );
        assert!(request.ends_with("\r\n\r\n"), "{request}");
        let mut reader = Reader::default();
        assert_eq!(
            reader.next(&mut read).await.unwrap(),
            Received::Text(b"{}".to_vec())
        );

        for refusal in [
            &b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n"[..],
            &b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\n\r\n"[..],
            &b"HTTP/1.1 101 Switching"[..],
        ] {
            let mut read = tokio::io::BufReader::new(refusal);
            let err = handshake(&mut read, &mut Vec::new()).await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
        }
    }

    #[tokio::test]
    async fn an_overlong_line_is_read_past_and_the_next_one_read() {
        let input = b"0123456789abcdef\nshort\nlast";
        let mut read = tokio::io::BufReader::with_capacity(4, &input[..]);
        let mut buf = Vec::new();
        assert_eq!(
            read_line(&mut read, &mut buf, 8).await.unwrap(),
            Line::TooLong
        );
        buf.clear();
        assert_eq!(read_line(&mut read, &mut buf, 8).await.unwrap(), Line::Read);
        assert_eq!(buf, b"short");
        buf.clear();
        assert_eq!(read_line(&mut read, &mut buf, 8).await.unwrap(), Line::Read);
        assert_eq!(buf, b"last");
        buf.clear();
        assert_eq!(read_line(&mut read, &mut buf, 8).await.unwrap(), Line::Eof);
    }
}
