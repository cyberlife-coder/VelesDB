use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::serve::Listener;
use velesdb_core::metrics::OperationalMetrics;

use super::{CountingListener, OpenConnection};

fn open_count(metrics: &OperationalMetrics) -> u64 {
    metrics.active_connections.load(Ordering::Relaxed)
}

#[test]
fn open_connection_counts_while_alive() {
    let metrics = OperationalMetrics::new_arc();
    let first = OpenConnection::new(Arc::clone(&metrics));
    let second = OpenConnection::new(Arc::clone(&metrics));
    assert_eq!(open_count(&metrics), 2);
    drop(first);
    assert_eq!(open_count(&metrics), 1);
    drop(second);
    assert_eq!(open_count(&metrics), 0);
}

#[tokio::test]
async fn counting_listener_counts_an_accepted_stream_until_it_is_dropped() {
    let metrics = OperationalMetrics::new_arc();
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test: bind");
    let mut listener = CountingListener::new(tcp, Arc::clone(&metrics));
    let addr = listener.local_addr().expect("test: local addr");

    let client = tokio::net::TcpStream::connect(addr)
        .await
        .expect("test: connect");
    let (accepted, _) = listener.accept().await;
    assert_eq!(open_count(&metrics), 1);

    drop(accepted);
    assert_eq!(open_count(&metrics), 0);
    drop(client);
}
