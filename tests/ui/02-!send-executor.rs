use fastwebsockets_monoio::WebSocket;
use hyper::header::CONNECTION;
use hyper::header::UPGRADE;
use hyper::upgrade::Upgraded;
use hyper::body::Bytes;
use http_body_util::Empty;
use hyper::Request;
use std::future::Future;
use tokio::net::TcpStream;
use hyper_util::rt::tokio::TokioIo;
use anyhow::Result;

struct SpawnLocalExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnLocalExecutor
where
  Fut: Future + 'static,
  Fut::Output: 'static,
{
  fn execute(&self, fut: Fut) {
    tokio::task::spawn_local(fut);
  }
}

async fn connect(
  path: &str,
) -> Result<WebSocket<TokioIo<Upgraded>>> {
  let stream = TcpStream::connect("localhost:9001").await?;

  let req = Request::builder()
    .method("GET")
    .uri(format!("http://localhost:9001/{}", path))
    .header("Host", "localhost:9001")
    .header(UPGRADE, "websocket")
    .header(CONNECTION, "upgrade")
    .header(
      "Sec-WebSocket-Key",
      fastwebsockets_monoio::handshake::generate_key(),
    )
    .header("Sec-WebSocket-Version", "13")
    .body(Empty::<Bytes>::new())?;

  let (ws, _) =
    fastwebsockets_monoio::handshake::client(&SpawnLocalExecutor, req, stream).await?;
  Ok(ws)
}

fn main() {}