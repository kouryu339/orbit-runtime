//! Bound browser calls without Native-only Reqwest options.
pub(crate) async fn deadline<T, F>(future: F) -> crate::error::Result<T>
where
    F: std::future::Future<Output = crate::error::Result<T>>,
{
    #[cfg(target_arch = "wasm32")]
    {
        corework::platform::time::timeout(crate::stream_control::TOTAL_TIMEOUT, future)
            .await
            .map_err(|_| crate::error::ApiError::Retryable {
                status: None,
                msg: "Browser request deadline exceeded".into(),
                retry_after: None,
                attempts: 1,
            })?
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        future.await
    }
}
