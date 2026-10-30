//! Open-connection accounting behind `velesdb_active_connections`.
//!
//! A connection counts from the moment it is accepted until its socket is
//! dropped — the end of the last request on it, for a keep-alive connection
//! — on both serving paths: [`CountingListener`] wraps the plain-HTTP
//! listener `axum::serve` drives, and the HTTPS accept loop holds an
//! [`OpenConnection`] for each connection it spawns.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use velesdb_core::metrics::OperationalMetrics;

/// One open connection: counted in `velesdb_active_connections` while alive.
#[must_use = "the connection is counted only while this guard is alive"]
pub struct OpenConnection {
    metrics: Arc<OperationalMetrics>,
}

impl OpenConnection {
    /// Counts a newly accepted connection.
    pub fn new(metrics: Arc<OperationalMetrics>) -> Self {
        metrics.inc_connections();
        Self { metrics }
    }
}

impl Drop for OpenConnection {
    fn drop(&mut self) {
        self.metrics.dec_connections();
    }
}

/// A TCP listener whose every accepted stream is counted while it lives.
pub struct CountingListener {
    listener: TcpListener,
    metrics: Arc<OperationalMetrics>,
}

impl CountingListener {
    /// Wraps `listener`, counting its connections into `metrics`.
    pub fn new(listener: TcpListener, metrics: Arc<OperationalMetrics>) -> Self {
        Self { listener, metrics }
    }
}

impl axum::serve::Listener for CountingListener {
    type Io = CountedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        // The `TcpListener` impl logs and retries accept errors itself.
        let (stream, addr) = axum::serve::Listener::accept(&mut self.listener).await;
        let open = OpenConnection::new(Arc::clone(&self.metrics));
        (
            CountedStream {
                stream,
                _open: open,
            },
            addr,
        )
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

/// A [`TcpStream`] that holds its connection's [`OpenConnection`]; every I/O
/// call is the stream's own.
pub struct CountedStream {
    stream: TcpStream,
    _open: OpenConnection,
}

impl AsyncRead for CountedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for CountedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }
}

#[cfg(test)]
#[path = "connections_tests.rs"]
mod tests;
