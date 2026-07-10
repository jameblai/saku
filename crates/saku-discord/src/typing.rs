//! Typing Indicator keep-alive for an active Run.

use std::future::Future;
use std::time::Duration;

use tokio::sync::oneshot;

/// Discord typing persists ~7–10s; refresh on this cadence (same as Serenity's `Typing`).
pub const TYPING_REFRESH: Duration = Duration::from_secs(7);

/// Keeps a Discord Typing Indicator alive until [`TypingIndicator::stop`] or drop.
pub struct TypingIndicator {
    stop: Option<oneshot::Sender<()>>,
}

impl TypingIndicator {
    /// Ping immediately, then on `interval`, until stopped. Ping errors are ignored.
    pub fn start<F, Fut>(interval: Duration, mut ping: F) -> Self
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), ()>> + Send + 'static,
    {
        let (tx, mut rx) = oneshot::channel();
        tokio::spawn(async move {
            loop {
                let _ = ping().await;
                tokio::select! {
                    _ = &mut rx => break,
                    _ = tokio::time::sleep(interval) => {}
                }
            }
        });
        Self { stop: Some(tx) }
    }

    pub fn stop(mut self) {
        self.signal_stop();
    }

    fn signal_stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for TypingIndicator {
    fn drop(&mut self) {
        self.signal_stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn counting_ping(
        pings: Arc<AtomicU32>,
    ) -> impl FnMut() -> std::pin::Pin<Box<dyn Future<Output = Result<(), ()>> + Send>> {
        move || {
            let pings = pings.clone();
            Box::pin(async move {
                pings.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn typing_indicator_pings_immediately() {
        let pings = Arc::new(AtomicU32::new(0));
        let _indicator = TypingIndicator::start(TYPING_REFRESH, counting_ping(pings.clone()));
        tokio::task::yield_now().await;
        assert_eq!(pings.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn typing_indicator_refreshes_on_interval() {
        let pings = Arc::new(AtomicU32::new(0));
        let _indicator = TypingIndicator::start(TYPING_REFRESH, counting_ping(pings.clone()));
        tokio::task::yield_now().await;
        assert_eq!(pings.load(Ordering::SeqCst), 1);

        tokio::time::advance(TYPING_REFRESH).await;
        tokio::task::yield_now().await;
        assert_eq!(pings.load(Ordering::SeqCst), 2);

        tokio::time::advance(TYPING_REFRESH).await;
        tokio::task::yield_now().await;
        assert_eq!(pings.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn typing_indicator_stops_refreshing_after_stop() {
        let pings = Arc::new(AtomicU32::new(0));
        let indicator = TypingIndicator::start(TYPING_REFRESH, counting_ping(pings.clone()));
        tokio::task::yield_now().await;
        assert_eq!(pings.load(Ordering::SeqCst), 1);

        indicator.stop();
        tokio::task::yield_now().await;

        tokio::time::advance(TYPING_REFRESH).await;
        tokio::task::yield_now().await;
        assert_eq!(pings.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn typing_indicator_keeps_going_after_ping_failure() {
        let pings = Arc::new(AtomicU32::new(0));
        let pings_c = pings.clone();
        let _indicator = TypingIndicator::start(TYPING_REFRESH, move || {
            let pings_c = pings_c.clone();
            async move {
                let n = pings_c.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 1 { Err(()) } else { Ok(()) }
            }
        });
        tokio::task::yield_now().await;
        assert_eq!(pings.load(Ordering::SeqCst), 1);

        tokio::time::advance(TYPING_REFRESH).await;
        tokio::task::yield_now().await;
        assert_eq!(pings.load(Ordering::SeqCst), 2);
    }
}
