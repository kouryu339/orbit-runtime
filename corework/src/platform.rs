//! Scheduling and clocks shared by native execution and a single browser Worker.
#[cfg(not(target_arch = "wasm32"))]
pub fn local_future<F: std::future::Future>(future: F) -> F {
    future
}
/// Browser futures stay on their originating Worker. The wrapper preserves
/// native Send bounds without ever transferring JS objects between threads.
#[cfg(target_arch = "wasm32")]
pub fn local_future<F: std::future::Future>(future: F) -> send_wrapper::SendWrapper<F> {
    send_wrapper::SendWrapper::new(future)
}
#[cfg(not(target_arch = "wasm32"))]
pub use tokio::spawn;
#[cfg(not(target_arch = "wasm32"))]
pub mod time {
    pub use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    pub use tokio::time::{sleep, timeout};
    pub async fn sleep_until(deadline: Instant) {
        tokio::time::sleep_until(deadline.into()).await;
    }
    pub fn system_time_now() -> SystemTime {
        SystemTime::now()
    }
}

#[cfg(target_arch = "wasm32")]
pub mod time {
    use futures::{future::Either, pin_mut};
    pub use std::time::{Duration, SystemTime, UNIX_EPOCH};
    pub use web_time::Instant;
    pub fn system_time_now() -> SystemTime {
        UNIX_EPOCH
            + web_time::SystemTime::now()
                .duration_since(web_time::UNIX_EPOCH)
                .unwrap_or_default()
    }

    pub fn sleep(duration: Duration) -> impl std::future::Future<Output = ()> + Send {
        send_wrapper::SendWrapper::new(gloo_timers::future::TimeoutFuture::new(
            duration.as_millis().min(u32::MAX as u128) as u32,
        ))
    }
    pub async fn sleep_until(deadline: Instant) {
        sleep(deadline.saturating_duration_since(Instant::now())).await
    }
    #[derive(Debug)]
    pub struct Elapsed;
    impl std::fmt::Display for Elapsed {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("deadline exceeded")
        }
    }
    impl std::error::Error for Elapsed {}
    pub async fn timeout<F: std::future::Future>(
        duration: Duration,
        future: F,
    ) -> Result<F::Output, Elapsed> {
        let timer = sleep(duration);
        pin_mut!(future, timer);
        match futures::future::select(future, timer).await {
            Either::Left((value, _)) => Ok(value),
            Either::Right(_) => Err(Elapsed),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod task {
    pub use tokio::task::{yield_now, JoinHandle, JoinSet};
}

#[cfg(target_arch = "wasm32")]
pub mod task {
    use futures::{
        channel::oneshot,
        future::{AbortHandle, Abortable},
        stream::{FuturesUnordered, StreamExt},
    };
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        task::{Context, Poll},
    };
    #[derive(Debug)]
    pub struct JoinError;
    impl JoinError {
        pub fn is_cancelled(&self) -> bool {
            true
        }
    }
    impl std::fmt::Display for JoinError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("task cancelled")
        }
    }
    impl std::error::Error for JoinError {}
    pub struct JoinHandle<T> {
        receiver: oneshot::Receiver<T>,
        abort: AbortHandle,
        finished: Arc<AtomicBool>,
    }
    impl<T> JoinHandle<T> {
        pub fn abort(&self) {
            self.abort.abort();
        }
        pub fn is_finished(&self) -> bool {
            self.finished.load(Ordering::Acquire)
        }
    }
    impl<T> Future for JoinHandle<T> {
        type Output = Result<T, JoinError>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut self.receiver)
                .poll(cx)
                .map(|result| result.map_err(|_| JoinError))
        }
    }
    pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let (abort, registration) = AbortHandle::new_pair();
        let finished = Arc::new(AtomicBool::new(false));
        let mark = finished.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(value) = Abortable::new(future, registration).await {
                let _ = sender.send(value);
            }
            mark.store(true, Ordering::Release);
        });
        JoinHandle {
            receiver,
            abort,
            finished,
        }
    }
    pub async fn yield_now() {
        super::time::sleep(std::time::Duration::ZERO).await;
    }
    pub struct JoinSet<T> {
        tasks: FuturesUnordered<JoinHandle<T>>,
    }
    impl<T: Send + 'static> Default for JoinSet<T> {
        fn default() -> Self {
            Self::new()
        }
    }
    impl<T: Send + 'static> JoinSet<T> {
        pub fn new() -> Self {
            Self {
                tasks: FuturesUnordered::new(),
            }
        }
        pub fn spawn<F>(&mut self, future: F)
        where
            F: Future<Output = T> + Send + 'static,
        {
            self.tasks.push(spawn(future));
        }
        pub fn len(&self) -> usize {
            self.tasks.len()
        }
        pub fn is_empty(&self) -> bool {
            self.tasks.is_empty()
        }
        pub fn abort_all(&mut self) {
            for task in self.tasks.iter() {
                task.abort();
            }
        }
        pub async fn join_next(&mut self) -> Option<Result<T, JoinError>> {
            self.tasks.next().await
        }
    }
    impl<T> Drop for JoinSet<T> {
        fn drop(&mut self) {
            for task in self.tasks.iter() {
                task.abort();
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use task::spawn;

#[cfg(not(target_arch = "wasm32"))]
pub use tokio::fs;

/// Legacy path APIs never acquire ambient filesystem authority in a browser.
/// Browser resources are provided through the content-store host contract.
#[cfg(target_arch = "wasm32")]
pub mod fs {
    use std::{
        io,
        path::{Path, PathBuf},
    };
    fn unavailable() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem paths are unavailable in the browser; use the content-store host adapter",
        )
    }
    pub async fn read(_: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        Err(unavailable())
    }
    pub async fn read_to_string(_: impl AsRef<Path>) -> io::Result<String> {
        Err(unavailable())
    }
    pub async fn write(_: impl AsRef<Path>, _: impl AsRef<[u8]>) -> io::Result<()> {
        Err(unavailable())
    }
    pub async fn create_dir_all(_: impl AsRef<Path>) -> io::Result<()> {
        Err(unavailable())
    }
    pub async fn remove_file(_: impl AsRef<Path>) -> io::Result<()> {
        Err(unavailable())
    }
    pub async fn remove_dir_all(_: impl AsRef<Path>) -> io::Result<()> {
        Err(unavailable())
    }
    pub async fn try_exists(_: impl AsRef<Path>) -> io::Result<bool> {
        Err(unavailable())
    }
    pub async fn metadata(_: impl AsRef<Path>) -> io::Result<std::fs::Metadata> {
        Err(unavailable())
    }
    pub struct OpenOptions;
    impl Default for OpenOptions {
        fn default() -> Self {
            Self::new()
        }
    }
    impl OpenOptions {
        pub fn new() -> Self {
            Self
        }
        pub fn create(&mut self, _: bool) -> &mut Self {
            self
        }
        pub fn append(&mut self, _: bool) -> &mut Self {
            self
        }
        pub fn write(&mut self, _: bool) -> &mut Self {
            self
        }
        pub async fn open(&self, _: impl AsRef<Path>) -> io::Result<tokio::io::Sink> {
            Err(unavailable())
        }
    }
    pub struct ReadDir;
    pub struct DirEntry;
    impl ReadDir {
        pub async fn next_entry(&mut self) -> io::Result<Option<DirEntry>> {
            Err(unavailable())
        }
    }
    impl DirEntry {
        pub fn path(&self) -> PathBuf {
            unreachable!("a browser cannot construct filesystem directory entries")
        }
        pub fn file_name(&self) -> std::ffi::OsString {
            unreachable!("a browser cannot construct filesystem directory entries")
        }
        pub async fn file_type(&self) -> io::Result<std::fs::FileType> {
            Err(unavailable())
        }
    }
    pub async fn read_dir(_: impl AsRef<Path>) -> io::Result<ReadDir> {
        Err(unavailable())
    }
}
