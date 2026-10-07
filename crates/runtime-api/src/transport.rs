//! Audited HTTP/1 output ownership path: bytes::Bytes owner -> Hyper Queue ->
//! vectored Tokio TCP write. No TLS, compression, HTTP/2, or flattening adapter.
use crate::{ServiceShutdown, security::PeerEndpoints};
use axum::{Extension, Router};
use hyper::server::conn::http1;
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    net::TcpListener,
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
    time::{Sleep, sleep},
};

pub const WRITE_PROGRESS_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_CONNECTIONS: usize = 64;
/// LAN cannot consume the sixteen connection slots reserved for local control.
pub const MAX_LAN_CONNECTIONS: usize = 48;
// Separately bounded connection teardown scratch/traffic. This is not generated
// text and must never be counted as part of the core's 256 KiB output ledger.
// Allow a just-over-limit eager image upload to observe its complete 413.
// This only drains traffic: scratch stays 8 KiB and the total deadline stays 1s.
const LINGER_BYTE_LIMIT: usize = crate::config::MAX_IMAGE_BODY_BYTES + 64 * 1024;
const LINGER_TIMEOUT: Duration = Duration::from_secs(1);
const LINGER_SCRATCH_BYTES: usize = 8 * 1024;
#[derive(Debug, PartialEq, Eq)]
enum LingerEnd {
    Eof,
    ByteLimit,
    Deadline,
    IoError,
}
#[derive(Debug)]
struct LingerResult {
    discarded: usize,
    end: LingerEnd,
}
/// RFC 9112 section 9.6: flush HTTP first, half-close writes, then briefly drain
/// reads before full close. A peer never extends the total byte or time limits.
/// Already-buffered Hyper bytes count, and no bytes are parsed or dispatched.
async fn bounded_linger_close<T: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut T,
    buffered: usize,
) -> LingerResult {
    let mut discarded = buffered;
    let outcome = tokio::time::timeout(LINGER_TIMEOUT, async {
        socket.shutdown().await?;
        let mut scratch = [0u8; LINGER_SCRATCH_BYTES];
        while discarded < LINGER_BYTE_LIMIT {
            let maximum = scratch.len().min(LINGER_BYTE_LIMIT - discarded);
            let read = socket.read(&mut scratch[..maximum]).await?;
            if read == 0 {
                return Ok::<_, io::Error>(LingerEnd::Eof);
            }
            discarded += read;
        }
        Ok(LingerEnd::ByteLimit)
    })
    .await;
    let end = match outcome {
        Ok(Ok(end)) => end,
        Ok(Err(_)) => LingerEnd::IoError,
        Err(_) => LingerEnd::Deadline,
    };
    LingerResult { discarded, end }
}
/// Timer exists only after an actual socket write returns Pending. CPU work,
/// queueing, load, prepare and prefill cannot accidentally consume this deadline.
/// A positive write resets it; a terminal core event does not disable it.
struct ProgressIo<T> {
    inner: T,
    timeout: Duration,
    pending: Option<Pin<Box<Sleep>>>,
}
impl<T> ProgressIo<T> {
    fn new(inner: T, timeout: Duration) -> Self {
        Self {
            inner,
            timeout,
            pending: None,
        }
    }
    fn write_result(
        &mut self,
        cx: &mut Context<'_>,
        result: Poll<io::Result<usize>>,
    ) -> Poll<io::Result<usize>> {
        match result {
            Poll::Pending => {
                let timer = self
                    .pending
                    .get_or_insert_with(|| Box::pin(sleep(self.timeout)));
                if std::future::Future::poll(timer.as_mut(), cx).is_ready() {
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "local HTTP write stalled",
                    )))
                } else {
                    Poll::Pending
                }
            }
            Poll::Ready(Ok(n)) => {
                if n > 0 {
                    self.pending = None;
                }
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(e)) => {
                self.pending = None;
                Poll::Ready(Err(e))
            }
        }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for ProgressIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for ProgressIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        self.write_result(cx, result)
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        self.write_result(cx, result)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
pub async fn serve(
    listener: TcpListener,
    app: Router,
    shutdown: ServiceShutdown,
) -> io::Result<()> {
    serve_with_lan(listener, app, None, shutdown).await
}
/// Both sockets must already be bound before discovery is published. Any serving
/// failure stops the shared runtime and both listeners, then waits for cleanup.
pub async fn serve_with_lan(
    listener: TcpListener,
    app: Router,
    lan: Option<(TcpListener, Router, crate::LanApiConfig)>,
    shutdown: ServiceShutdown,
) -> io::Result<()> {
    let checked = (|| {
        if !crate::proof::normalize_endpoint(listener.local_addr()?)
            .ip()
            .is_loopback()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "only loopback management listeners are permitted",
            ));
        }
        match lan {
            Some((listener, app, config)) => {
                let policy = config.policy().map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "invalid LAN listener policy",
                    )
                })?;
                if listener.local_addr()? != policy.listen {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "LAN listener must match its configured private address",
                    ));
                }
                Ok(Some((listener, app, policy)))
            }
            None => Ok(None),
        }
    })();
    let lan = match checked {
        Ok(lan) => lan,
        Err(error) => {
            shutdown.begin();
            let _ = shutdown.wait().await;
            return Err(error);
        }
    };
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let local = serve_listener(listener, app, shutdown.clone(), slots.clone(), None);
    if let Some((listener, app, policy)) = lan {
        let remote = serve_listener(listener, app, shutdown, slots, Some(policy));
        let (local, remote) = tokio::join!(local, remote);
        local.and(remote)
    } else {
        local.await
    }
}
struct ConnectionPermits {
    _total: OwnedSemaphorePermit,
    _lan: Option<OwnedSemaphorePermit>,
}
fn take_connection_slots(
    total: &Arc<Semaphore>,
    lan: Option<&Arc<Semaphore>>,
) -> Option<ConnectionPermits> {
    let lan = match lan {
        Some(lan) => Some(lan.clone().try_acquire_owned().ok()?),
        None => None,
    };
    Some(ConnectionPermits {
        _total: total.clone().try_acquire_owned().ok()?,
        _lan: lan,
    })
}
async fn serve_listener(
    listener: TcpListener,
    app: Router,
    shutdown: ServiceShutdown,
    slots: Arc<Semaphore>,
    lan: Option<crate::lan::LanAccessPolicy>,
) -> io::Result<()> {
    let lan_slots = lan
        .as_ref()
        .map(|_| Arc::new(Semaphore::new(MAX_LAN_CONNECTIONS)));
    let mut connections = JoinSet::new();
    let serving_result = loop {
        tokio::select! {
            biased;
            _=shutdown.requested()=>break Ok(()),
            Some(_)=connections.join_next(),if !connections.is_empty()=>{},
            incoming=listener.accept()=>{
                let (stream,client)=match incoming { Ok(value)=>value, Err(error)=>break Err(error) };
                let server=match stream.local_addr() { Ok(value)=>value, Err(error)=>break Err(error) };
                let endpoints=PeerEndpoints{client,server};
                if let Some(policy)=&lan {
                    if !policy.allows(endpoints) { continue; }
                } else if !crate::proof::normalize_endpoint(client).ip().is_loopback() { continue; }
                let Some(permits)=take_connection_slots(&slots,lan_slots.as_ref()) else{continue;};
                if let Err(error)=stream.set_nodelay(true) { break Err(error); }
                let service=TowerToHyperService::new(app.clone().layer(Extension(endpoints)));
                let stop=shutdown.clone();
                connections.spawn(async move {
                    let _permits=permits;
                    let mut builder=http1::Builder::new();
                    builder.writev(true).pipeline_flush(false).half_close(false).max_buf_size(16*1024).max_headers(64).timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(10));
                    let mut connection=builder.serve_connection(TokioIo::new(ProgressIo::new(stream,WRITE_PROGRESS_TIMEOUT)),service);
                    let finished=tokio::select!{
                        result=std::future::poll_fn(|cx|connection.poll_without_shutdown(cx))=>Some(result),
                        _=stop.requested()=>{
                            Pin::new(&mut connection).graceful_shutdown();
                            // Runtime cancellation has its own confirmed-cleanup deadline.
                            // Preserve responses during cleanup, then bound idle/blocked sockets.
                            tokio::select!{result=std::future::poll_fn(|cx|connection.poll_without_shutdown(cx))=>Some(result),_=async{let _=stop.wait().await;tokio::time::sleep(WRITE_PROGRESS_TIMEOUT).await;}=>None}
                        }
                    };
                    if matches!(finished,Some(Ok(()))){
                        // poll_without_shutdown is successful only AFTER Hyper's
                        // HTTP write queue is flushed. Keep writev/Queue intact.
                        let parts=connection.into_parts();
                        let buffered=parts.read_buf.len();
                        drop(parts.read_buf);
                        drop(parts.service);
                        let mut socket=parts.io.into_inner().inner;
                        let result=bounded_linger_close(&mut socket,buffered).await;
                        debug_assert!(result.discarded<=LINGER_BYTE_LIMIT);
                        let _=result.end;
                    }
                });
            }
        }
    };
    drop(listener);
    // Also runs for accept/socket failures; the other listener observes this
    // immediately and retains its own JoinSet until connection cleanup completes.
    shutdown.begin();
    let result = shutdown
        .wait()
        .await
        .map_err(|_| io::Error::other("runtime cleanup failed"));
    while connections.join_next().await.is_some() {}
    result.and(serving_result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lan_connection_quota_preserves_local_reserve_and_all_permits_return() {
        let total = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let lan = Arc::new(Semaphore::new(MAX_LAN_CONNECTIONS));
        let remote: Vec<_> = (0..MAX_LAN_CONNECTIONS)
            .map(|_| take_connection_slots(&total, Some(&lan)).unwrap())
            .collect();
        assert!(take_connection_slots(&total, Some(&lan)).is_none());
        assert_eq!(total.available_permits(), 16);
        let local: Vec<_> = (0..16)
            .map(|_| take_connection_slots(&total, None).unwrap())
            .collect();
        assert!(take_connection_slots(&total, None).is_none());
        drop(remote);
        assert_eq!(total.available_permits(), 48);
        assert_eq!(lan.available_permits(), 48);
        drop(local);
        assert_eq!(total.available_permits(), 64);
        let local: Vec<_> = (0..64)
            .map(|_| take_connection_slots(&total, None).unwrap())
            .collect();
        assert!(take_connection_slots(&total, Some(&lan)).is_none());
        assert_eq!(lan.available_permits(), 48);
        drop(local);
        assert_eq!(total.available_permits(), 64);
    }
    #[tokio::test]
    async fn pending_writes_time_out_but_idle_prefill_does_not() {
        use tokio::io::AsyncWriteExt;
        let (writer, _reader) = tokio::io::duplex(1);
        let mut writer = ProgressIo::new(writer, Duration::from_millis(20));
        tokio::time::sleep(Duration::from_millis(30)).await;
        writer.write_all(b"x").await.unwrap();
        assert_eq!(
            writer.write_all(b"y").await.unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
    struct DropBytes {
        bytes: Vec<u8>,
        dropped: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl AsRef<[u8]> for DropBytes {
        fn as_ref(&self) -> &[u8] {
            &self.bytes
        }
    }
    impl Drop for DropBytes {
        fn drop(&mut self) {
            self.dropped
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    #[tokio::test]
    async fn audited_hyper_queue_preserves_bytes_owner_during_pending_and_partial_write() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut client, server) = tokio::io::duplex(64);
        let dropped = Arc::new(AtomicUsize::new(0));
        let provided = Arc::new(AtomicBool::new(false));
        let count = dropped.clone();
        let supplied = provided.clone();
        let service = hyper::service::service_fn(move |_| {
            let bytes = bytes::Bytes::from_owner(DropBytes {
                bytes: vec![b'x'; 4096],
                dropped: count.clone(),
            });
            supplied.store(true, Ordering::SeqCst);
            std::future::ready(Ok::<_, std::convert::Infallible>(hyper::Response::new(
                http_body_util::Full::new(bytes),
            )))
        });
        let connection = tokio::spawn(async move {
            let mut connection = http1::Builder::new()
                .writev(true)
                .pipeline_flush(false)
                .half_close(false)
                .serve_connection(
                    TokioIo::new(ProgressIo::new(server, Duration::from_secs(2))),
                    service,
                );
            let result = std::future::poll_fn(|cx| connection.poll_without_shutdown(cx)).await;
            if result.is_ok() {
                let mut io = connection.into_parts().io.into_inner();
                io.shutdown().await.unwrap();
            }
            result
        });
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        while !provided.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        let mut partial = [0u8; 128];
        client.read_exact(&mut partial).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.unwrap();
        connection.await.unwrap().unwrap();
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn completed_body_still_times_out_and_releases_owner_on_connection_drop() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncWriteExt;
        let (mut client, server) = tokio::io::duplex(64);
        let dropped = Arc::new(AtomicUsize::new(0));
        let count = dropped.clone();
        let service = hyper::service::service_fn(move |_| {
            let bytes = bytes::Bytes::from_owner(DropBytes {
                bytes: vec![b'x'; 4096],
                dropped: count.clone(),
            });
            std::future::ready(Ok::<_, std::convert::Infallible>(hyper::Response::new(
                http_body_util::Full::new(bytes),
            )))
        });
        let connection = tokio::spawn(async move {
            let mut connection = http1::Builder::new()
                .writev(true)
                .pipeline_flush(false)
                .half_close(false)
                .serve_connection(
                    TokioIo::new(ProgressIo::new(server, Duration::from_millis(20))),
                    service,
                );
            std::future::poll_fn(|cx| connection.poll_without_shutdown(cx)).await
        });
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), connection)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn lingering_discard_counts_hyper_buffer_and_stops_at_exact_byte_cap() {
        let (mut socket, mut peer) = tokio::io::duplex(256);
        peer.write_all(&[7u8; 128]).await.unwrap();
        let result = bounded_linger_close(&mut socket, LINGER_BYTE_LIMIT - 13).await;
        assert_eq!(result.end, LingerEnd::ByteLimit);
        assert_eq!(result.discarded, LINGER_BYTE_LIMIT);
        let mut remaining = [0u8; 115];
        socket.read_exact(&mut remaining).await.unwrap();
        assert_eq!(remaining, [7; 115]);
    }
    #[tokio::test]
    async fn endlessly_ready_sender_cannot_exceed_lingering_byte_cap() {
        let (mut socket, mut peer) = tokio::io::duplex(8192);
        let writer = tokio::spawn(async move {
            let bytes = [0u8; 8192];
            while peer.write_all(&bytes).await.is_ok() {}
        });
        let result =
            tokio::time::timeout(Duration::from_secs(2), bounded_linger_close(&mut socket, 0))
                .await
                .unwrap();
        assert_eq!(result.end, LingerEnd::ByteLimit);
        assert_eq!(result.discarded, LINGER_BYTE_LIMIT);
        drop(socket);
        tokio::time::timeout(Duration::from_secs(1), writer)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn slow_sender_does_not_reset_total_lingering_deadline() {
        let (mut socket, mut peer) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            loop {
                if peer.write_all(b"x").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let started = tokio::time::Instant::now();
        let result =
            tokio::time::timeout(Duration::from_secs(2), bounded_linger_close(&mut socket, 0))
                .await
                .unwrap();
        assert_eq!(result.end, LingerEnd::Deadline);
        assert!(result.discarded > 0 && result.discarded < 128);
        assert!(started.elapsed() >= LINGER_TIMEOUT);
        drop(socket);
        tokio::time::timeout(Duration::from_secs(1), writer)
            .await
            .unwrap()
            .unwrap();
    }
}
