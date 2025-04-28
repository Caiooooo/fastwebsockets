// Copyright 2023 Divy Srivastava <dj.srivastava23@gmail.com>
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use fastwebsockets::FragmentCollector;
use fastwebsockets::Frame;
use fastwebsockets::OpCode;
use fastwebsockets::WebSocketError;
use hyper::header::CONNECTION;
use hyper::header::UPGRADE;
use hyper::upgrade::Upgraded;
use hyper::Body;
use hyper::Request;
use monoio_compat::TcpStreamCompat;
use std::future::Future;

struct SpawnExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
where
  Fut: Future + Send + 'static,
  Fut::Output: Send + 'static,
{
  fn execute(&self, fut: Fut) {
    monoio::spawn(fut);
  }
}

async fn connect(addr: &str) -> Result<FragmentCollector<Upgraded>, WebSocketError> {
  let tcp_stream = monoio::net::TcpStream::connect(addr).await?;
  let tcp_stream = TcpStreamCompat::new(tcp_stream);

  let req = Request::builder()
    .method("GET")
    .uri(format!("ws://{}/", addr))
    .header("Host", addr)
    .header(UPGRADE, "websocket")
    .header(CONNECTION, "upgrade")
    .header(
      "Sec-WebSocket-Key",
      fastwebsockets::handshake::generate_key(),
    )
    .header("Sec-WebSocket-Version", "13")
    .body(Body::empty())?;

  let (ws, _) = fastwebsockets::handshake::client(&SpawnExecutor, req, tcp_stream).await?;
  Ok(FragmentCollector::new(ws))
}

#[monoio::main]
async fn main() -> Result<(), WebSocketError> {
  let addr = "127.0.0.1:8080";
  let mut ws = connect(addr).await?;

  // Send a test message
  let test_message = "Hello, Server!";
  ws.write_frame(Frame::text(test_message.as_bytes().to_vec().into()))
    .await?;

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
        let payload = String::from_utf8(msg.payload.to_vec()).expect("Invalid UTF-8 data");
        println!("Received: {}", payload);
      }
      OpCode::Close => {
        break;
      }
      _ => {}
    }
  }
  Ok(())
} 