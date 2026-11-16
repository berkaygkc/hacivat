//! Boxed byte streams and the factories that open them (HV-CONN-01). Working since Task 1.

use std::fmt;
use std::io;
use std::sync::Arc;

use futures::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncWrite};

/// A bidirectional byte stream a driver can speak its protocol over (HV-CONN-01).
pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> AsyncStream for T {}

/// A stream of any transport (TCP, Unix socket, tunnel channel) (HV-CONN-01).
pub type BoxedStream = Box<dyn AsyncStream>;

/// Opens a fresh stream to the same endpoint on every call (HV-CONN-01). Transport wrappers
/// (SSH, proxies) wrap one factory in another.
#[derive(Clone)]
pub struct StreamFactory(
    Arc<dyn Fn() -> BoxFuture<'static, io::Result<BoxedStream>> + Send + Sync>,
);

impl StreamFactory {
    pub fn new(
        f: impl Fn() -> BoxFuture<'static, io::Result<BoxedStream>> + Send + Sync + 'static,
    ) -> Self {
        StreamFactory(Arc::new(f))
    }

    /// Opens one stream.
    pub fn open(&self) -> BoxFuture<'static, io::Result<BoxedStream>> {
        (self.0)()
    }
}

impl fmt::Debug for StreamFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StreamFactory")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn a_factory_opens_a_new_stream_per_call() {
        let opened = Arc::new(AtomicUsize::new(0));
        let count = opened.clone();
        let factory = StreamFactory::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                let (a, mut b) = tokio::io::duplex(64);
                b.write_all(b"hv").await?;
                Ok(Box::new(a) as BoxedStream)
            })
        });
        let copy = factory.clone();
        let mut first = factory.open().await.unwrap();
        let _second = copy.open().await.unwrap();
        let mut buf = [0u8; 2];
        first.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hv");
        assert_eq!(opened.load(Ordering::SeqCst), 2);
        assert_eq!(format!("{factory:?}"), "StreamFactory");
    }
}
