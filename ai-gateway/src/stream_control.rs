use std::time::Duration;

use futures_util::{Stream, StreamExt};

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const TOTAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub(crate) async fn next_with_idle_timeout<S>(stream: &mut S) -> Result<Option<S::Item>, ()>
where
    S: Stream + Unpin,
{
    next_with_timeout(stream, IDLE_TIMEOUT).await
}

async fn next_with_timeout<S>(stream: &mut S, timeout: Duration) -> Result<Option<S::Item>, ()>
where
    S: Stream + Unpin,
{
    tokio::time::timeout(timeout, stream.next())
        .await
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_policy_does_not_retain_the_old_two_minute_total_limit() {
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(15));
        assert_eq!(IDLE_TIMEOUT, Duration::from_secs(60));
        assert_eq!(TOTAL_TIMEOUT, Duration::from_secs(600));
        assert!(TOTAL_TIMEOUT > Duration::from_secs(120));
    }

    #[tokio::test]
    async fn idle_timeout_is_measured_between_stream_items() {
        let mut stream = futures_util::stream::pending::<usize>();
        let result = next_with_timeout(&mut stream, Duration::from_millis(5)).await;
        assert_eq!(result, Err(()));

        let mut stream = futures_util::stream::iter([1usize, 2usize]);
        assert_eq!(
            next_with_timeout(&mut stream, Duration::from_millis(5)).await,
            Ok(Some(1))
        );
        assert_eq!(
            next_with_timeout(&mut stream, Duration::from_millis(5)).await,
            Ok(Some(2))
        );
        assert_eq!(
            next_with_timeout(&mut stream, Duration::from_millis(5)).await,
            Ok(None)
        );
    }
}
