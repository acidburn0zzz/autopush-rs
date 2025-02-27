//! A future to figure out where we're going to dispatch a TCP socket.
//!
//! When the websocket server receives a TCP connection it may be a websocket
//! request or a general HTTP request. Right now the websocket library we're
//! using, Tungstenite, doesn't have built-in support for handling this
//! situation, so we roll our own.
//!
//! The general idea here is that we're going to read just enough data off the
//! socket to parse an initial HTTP request. This request will be parsed by the
//! `httparse` crate. Once we've got a request we take a look at the headers and
//! if we find a websocket upgrade we classify it as a websocket request. If
//! it's otherwise a `/status` request, we return that we're supposed to get the
//! status, and finally after all that if it doesn't match we return an error.
//!
//! This is basically a "poor man's" HTTP router and while it should be good
//! enough for now it should probably be extended/refactored in the future!
//!
//! Note that also to implement this we buffer the request that we read in
//! memory and then attach that to a socket once we've classified what kind of
//! socket this is. That's done to replay the bytes we read again for the
//! tungstenite library, which'll duplicate header parsing but we don't have
//! many other options for now!

use bytes::BytesMut;
use futures::{try_ready, Future, Poll};
use tokio_core::net::TcpStream;
use tokio_io::AsyncRead;

use autopush_common::errors::{ApcError, ApcErrorKind};

use crate::server::tls::MaybeTlsStream;
use crate::server::webpush_io::WebpushIo;

pub struct Dispatch {
    socket: Option<MaybeTlsStream<TcpStream>>,
    data: BytesMut,
}

pub enum RequestType {
    Websocket,
    Status,
    LogCheck,
    LBHeartBeat,
    Version,
}

impl Dispatch {
    pub fn new(socket: MaybeTlsStream<TcpStream>) -> Self {
        Self {
            socket: Some(socket),
            data: BytesMut::new(),
        }
    }
}

impl Future for Dispatch {
    type Item = (WebpushIo, RequestType);
    type Error = ApcError;

    fn poll(&mut self) -> Poll<(WebpushIo, RequestType), ApcError> {
        loop {
            if self.data.len() == self.data.capacity() {
                self.data.reserve(16); // get some extra space
            }
            if try_ready!(self.socket.as_mut().unwrap().read_buf(&mut self.data)) == 0 {
                return Err(ApcErrorKind::GeneralError("early eof".into()).into());
            }
            let ty = {
                let mut headers = [httparse::EMPTY_HEADER; 32];
                let mut req = httparse::Request::new(&mut headers);
                match req.parse(&self.data)? {
                    httparse::Status::Complete(_) => {}
                    httparse::Status::Partial => continue,
                }

                if req.headers.iter().any(|h| h.name == "Upgrade") {
                    RequestType::Websocket
                } else {
                    match req.path {
                        Some(path) if path.starts_with("/status") || path == "/__heartbeat__" => {
                            RequestType::Status
                        }
                        Some(path) if path == "/__lbheartbeat__" => RequestType::LBHeartBeat,
                        Some(path) if path == "/__version__" => RequestType::Version,
                        // legacy:
                        Some(path) if path.starts_with("/v1/err/crit") => RequestType::LogCheck,
                        // standardized:
                        Some(path) if path == ("/__error__") => RequestType::LogCheck,
                        _ => {
                            debug!("unknown http request {:?}", req);
                            return Err(
                                ApcErrorKind::GeneralError("unknown http request".into()).into()
                            );
                        }
                    }
                }
            };

            let tcp = self.socket.take().unwrap();
            return Ok((WebpushIo::new(tcp, self.data.take()), ty).into());
        }
    }
}
