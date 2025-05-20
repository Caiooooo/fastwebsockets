[![Crates.io](https://img.shields.io/crates/v/fastwebsockets-monoio.svg)](https://crates.io/crates/fastwebsockets-monoio)

[Documentation](https://docs.rs/fastwebsockets-monoio) | [Benchmarks](benches/)

_fastwebsockets-monoio_ is a fast WebSocket protocol implementation based on the Monoio runtime.

Passes the
Autobahn|TestSuite<sup><a href="https://denoland.github.io/fastwebsockets/servers/">1</a></sup>
and fuzzed with LLVM's libfuzzer.

You can use it as a raw websocket frame parser and deal with spec compliance
yourself, or you can use it as a full-fledged websocket client/server.

```rust
use fastwebsockets_monoio::{Frame, OpCode, WebSocket};
use monoio::net::TcpStream;

async fn handle_client(
  mut socket: TcpStream,
) -> Result<(), WebSocketError> {
  handshake(&mut socket).await?;

  let mut ws = WebSocket::after_handshake(socket);
  ws.set_writev(true);
  ws.set_auto_close(true);
  ws.set_auto_pong(true);

  loop {
    let frame = ws.read_frame().await?;

    match frame.opcode {
      OpCode::Close => break,
      OpCode::Text | OpCode::Binary => {
        let frame = Frame::new(true, frame.opcode, None, frame.payload);
        ws.write_frame(frame).await?;
      }
      _ => {}
    }
  }

  Ok(())
}
```

**Fragmentation**

By default, fastwebsockets will give the application raw frames with FIN set.
Other crates like tungstenite which will give you a single message with all the
frames concatenated.

For concanated frames, use `FragmentCollector`:

```rust
let mut ws = WebSocket::after_handshake(socket);
let mut ws = FragmentCollector::new(ws);

let incoming = ws.read_frame().await?;
// Always returns full messages
assert!(incoming.fin);
```

> permessage-deflate is not supported yet.

**HTTP Upgrade**

Enable the `upgrade` feature to do server-side upgrades and client-side
handshakes.

This feature is powered by [hyper](https://docs.rs/hyper).

```rust
use fastwebsockets_monoio::upgrade::upgrade;
use hyper::{Request, Body, Response};
use bytes::Bytes;
use http_body_util::Empty;
use anyhow::Result;

async fn server_upgrade(
  mut req: Request<Incoming>,
) -> Result<Response<Empty<Bytes>>> {
  let (response, fut) = upgrade::upgrade(&mut req)?;

  monoio::spawn(async move {
    if let Err(e) = handle_client(fut).await {
      eprintln!("Error in websocket connection: {}", e);
    }
  });

  Ok(response)
}
```

Use the `handshake` module for client-side handshakes.

```rust
use fastwebsockets::handshake;
use fastwebsockets::WebSocket;
use hyper::{Request, Body, upgrade::Upgraded, header::{UPGRADE, CONNECTION}};
use tokio::net::TcpStream;
use std::future::Future;
use std::sync::Arc;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::rustls::OwnedTrustAnchor;
use tokio_rustls::rustls::Certificate;
use tokio_rustls::TlsConnector;
use monoio::io::IntoPollIo;

#[allow(deprecated)]
fn tls_connector() -> Result<TlsConnector> {
  static CERT: &[u8] = include_bytes!("./localhost.crt");
  let mut root_store = tokio_rustls::rustls::RootCertStore::empty();
  let local_certs: Vec<Certificate> = rustls_pemfile::certs(&mut &*CERT)
    .map(|mut certs| certs.drain(..).map(Certificate).collect())
    .unwrap();

  root_store.add_server_trust_anchors(
    webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
      OwnedTrustAnchor::from_subject_spki_name_constraints(
        ta.subject,
        ta.spki,
        ta.name_constraints,
      )
    }),
  );
  for cert in local_certs {
      root_store.add(&cert)?;
  }
  
  let config = ClientConfig::builder()
    .with_safe_defaults()
    .with_root_certificates(root_store)
    .with_no_client_auth();

  Ok(TlsConnector::from(Arc::new(config)))
}

async fn handle_websocket_upgrade(
  uri: Uri,
  port: u16,
) -> Result<(), WebSocketError> {
  // 1. 创建HTTP客户端
  let host = uri.host().expect("uri has no host");
  let port = uri.port_u16().unwrap_or(port);
  let addr = format!("{}:{}", host, port);
  let stream = TcpStream::connect(&addr).await?;
  let tcp_stream = HyperConnection(stream.into_poll_io()?);
  let domain =
    tokio_rustls::rustls::ServerName::try_from(uri.to_string().as_str())
      .map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid dnsname")
      })?;

  let tls_connector = tls_connector().unwrap();
  let tls_stream = tls_connector.connect(domain, tcp_stream).await.unwrap();

  let req = Request::builder()
    .method("GET")
    .uri(&addr)
    .header("Host", &addr)
    .header("Upgrade", "websocket")
    .header("Connection", "Upgrade")
    .header(
      "Sec-WebSocket-Key",
      fastwebsockets_monoio::handshake::generate_key(),
    )
    .header("Sec-WebSocket-Version", "13")
    .body(Body::empty())?;

  let (mut ws, _) =
    fastwebsockets_monoio::handshake::client(&HyperExecutor, req, tls_stream).await?;
  loop {
    let msg = match ws.read_frame().await {
      Ok(msg) => msg,
      Err(e) => {
        println!("Error: {}", e);
        ws.write_frame(Frame::close_raw(vec![].into())).await?;
        break;
      }
    };

    match msg.opcode {
      OpCode::Text => {
        let payload =
          String::from_utf8(msg.payload.to_vec()).expect("Invalid UTF-8 data");
        // Normally deserialise from json here, print just to show it works
        println!("{:?}", payload);
      }
      OpCode::Close => {
        break;
      }
      _ => {}
    }
  }
  Ok(())
}

#[monoio::main]
async fn main() {
  let uri: Uri = "127.0.0.1".parse::<hyper::Uri>().unwrap();
  let port = 8080;
  handle_websocket_upgrade(uri, port).await.unwrap();
}

#[derive(Clone)]
struct HyperExecutor;

impl<F> hyper::rt::Executor<F> for HyperExecutor
where
  F: Future + 'static,
  F::Output: 'static,
{
  fn execute(&self, fut: F) {
    monoio::spawn(fut);
  }
}

use std::pin::Pin;
struct HyperConnection(monoio::net::tcp::stream_poll::TcpStreamPoll);

impl tokio::io::AsyncRead for HyperConnection {
  #[inline]
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    buf: &mut tokio::io::ReadBuf<'_>,
  ) -> std::task::Poll<std::io::Result<()>> {
    Pin::new(&mut self.0).poll_read(cx, buf)
  }
}

impl tokio::io::AsyncWrite for HyperConnection {
  #[inline]
  fn poll_write(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    buf: &[u8],
  ) -> std::task::Poll<Result<usize, std::io::Error>> {
    Pin::new(&mut self.0).poll_write(cx, buf)
  }

  #[inline]
  fn poll_flush(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Result<(), std::io::Error>> {
    Pin::new(&mut self.0).poll_flush(cx)
  }

  #[inline]
  fn poll_shutdown(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Result<(), std::io::Error>> {
    Pin::new(&mut self.0).poll_shutdown(cx)
  }
}

unsafe impl Send for HyperConnection {}
```

**Usage with Axum**

Enable the Axum integration with `features = ["upgrade", "with_axum"]` in Cargo.toml.

```rust
use axum::{response::IntoResponse, routing::get, Router};
use fastwebsockets::upgrade;
use fastwebsockets::OpCode;
use fastwebsockets::WebSocketError;

#[tokio::main]
async fn main() {
  let app = Router::new().route("/", get(ws_handler));

  let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
  axum::serve(listener, app).await.unwrap();
}

async fn handle_client(fut: upgrade::UpgradeFut) -> Result<(), WebSocketError> {
  let mut ws = fastwebsockets::FragmentCollector::new(fut.await?);

  loop {
    let frame = ws.read_frame().await?;
    match frame.opcode {
      OpCode::Close => break,
      OpCode::Text | OpCode::Binary => {
        ws.write_frame(frame).await?;
      }
      _ => {}
    }
  }

  Ok(())
}

async fn ws_handler(ws: upgrade::IncomingUpgrade) -> impl IntoResponse {
  let (response, fut) = ws.upgrade().unwrap();

  tokio::task::spawn(async move {
    if let Err(e) = handle_client(fut).await {
      eprintln!("Error in websocket connection: {}", e);
    }
  });

  response
}
```


