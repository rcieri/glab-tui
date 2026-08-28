use anyhow::Result;
use std::sync::LazyLock;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{Duration, Instant};

/// Maximum concurrent CLI subprocesses allowed in flight across the entire application.
pub const MAX_CONCURRENT_REQUESTS: usize = 8;

/// Minimum delay between starting consecutive requests to prevent secondary / abuse rate limits.
pub const MIN_INTER_REQUEST_DELAY: Duration = Duration::from_millis(50);

/// Pacing interval for sequential bulk operations (bulk edit, bulk merge, bulk retry).
pub const BULK_PACING_DELAY: Duration = Duration::from_millis(100);

/// Maximum retry attempts when rate limited.
pub const MAX_RATE_LIMIT_RETRIES: u32 = 3;

/// Base backoff duration before retrying rate-limited calls.
pub const BASE_BACKOFF_DURATION: Duration = Duration::from_millis(500);

/// Centralized rate limiter and concurrency bounding for CLI subprocess calls.
pub struct ApiRateLimiter {
    semaphore: Semaphore,
    last_request_time: Mutex<Option<Instant>>,
}

impl ApiRateLimiter {
    pub const fn new() -> Self {
        Self {
            semaphore: Semaphore::const_new(MAX_CONCURRENT_REQUESTS),
            last_request_time: Mutex::const_new(None),
        }
    }

    /// Acquire a concurrency permit and enforce minimum inter-request burst delay.
    pub async fn pace_request(&self) -> tokio::sync::SemaphorePermit<'_> {
        let permit = self.semaphore.acquire().await.expect("semaphore closed");
        let mut last = self.last_request_time.lock().await;
        let now = Instant::now();
        if let Some(prev) = *last {
            let elapsed = now.saturating_duration_since(prev);
            if elapsed < MIN_INTER_REQUEST_DELAY {
                tokio::time::sleep(MIN_INTER_REQUEST_DELAY - elapsed).await;
            }
        }
        *last = Some(Instant::now());
        permit
    }
}

pub static RATE_LIMITER: LazyLock<ApiRateLimiter> = LazyLock::new(ApiRateLimiter::new);

/// Checks if an error string or stderr output indicates rate limiting, HTTP 429, or abuse detection.
pub fn is_rate_limit_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("ratelimit")
        || lower.contains("rate_limit")
        || lower.contains("rate-limit")
        || lower.contains("secondary rate limit")
        || lower.contains("abuse detection")
        || lower.contains("abuse-rate-limits")
        || lower.contains("too many requests")
        || lower.contains("retry-after")
        || lower.contains("try again later")
        || lower.contains("please wait")
}

fn calculate_jitter(attempt: u32) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(42);
    let jitter_ms = (nanos.wrapping_add(attempt * 7919) % 100) as u64;
    Duration::from_millis(jitter_ms)
}

/// Execute an async operation with rate limiter pacing and exponential backoff on rate limits.
pub async fn execute_with_retry<F, Fut, T>(mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;
    let mut backoff = BASE_BACKOFF_DURATION;

    loop {
        let _permit = RATE_LIMITER.pace_request().await;
        match op().await {
            Ok(res) => return Ok(res),
            Err(e) => {
                let err_msg = e.to_string();
                if attempt < MAX_RATE_LIMIT_RETRIES && is_rate_limit_error(&err_msg) {
                    attempt += 1;
                    let delay = backoff + calculate_jitter(attempt);
                    tokio::time::sleep(delay).await;
                    backoff *= 2;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

/// Paces sequential bulk operations to avoid triggering secondary rate limits.
pub async fn pace_bulk_operation() {
    tokio::time::sleep(BULK_PACING_DELAY).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_rate_limit_error() {
        assert!(is_rate_limit_error("HTTP 429: Too Many Requests"));
        assert!(is_rate_limit_error("API rate limit exceeded for user"));
        assert!(is_rate_limit_error(
            "You have exceeded a secondary rate limit. Please wait a few minutes."
        ));
        assert!(is_rate_limit_error("abuse detection mechanism triggered"));
        assert!(is_rate_limit_error("Please try again later."));
        assert!(is_rate_limit_error("retry-after: 60"));
        assert!(!is_rate_limit_error("404 Not Found"));
        assert!(!is_rate_limit_error("GraphQL error: syntax error"));
        assert!(!is_rate_limit_error("remote repository not found"));
    }

    #[tokio::test]
    async fn test_execute_with_retry_succeeds_immediately() {
        let count = std::sync::atomic::AtomicUsize::new(0);
        let res = execute_with_retry(|| async {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok::<_, anyhow::Error>(42)
        })
        .await;

        assert_eq!(res.unwrap(), 42);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_execute_with_retry_retries_on_rate_limit() {
        let count = std::sync::atomic::AtomicUsize::new(0);
        let res = execute_with_retry(|| async {
            let prev = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if prev == 0 {
                anyhow::bail!("HTTP 429: Too Many Requests");
            }
            Ok::<_, anyhow::Error>("success")
        })
        .await;

        assert_eq!(res.unwrap(), "success");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_execute_with_retry_fails_immediately_on_non_rate_limit() {
        let count = std::sync::atomic::AtomicUsize::new(0);
        let res: Result<()> = execute_with_retry(|| async {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            anyhow::bail!("Resource not found");
        })
        .await;

        assert!(res.is_err());
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
