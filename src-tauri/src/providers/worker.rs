//! Independent provider workers. Slow WSL/process/API queries cannot delay
//! another provider or the UI; refresh requests are bounded and coalesced.
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant, SystemTime};

use super::ProviderOutput;

pub struct Observation {
    pub output: ProviderOutput,
    pub completed_at: SystemTime,
    pub duration_ms: u64,
}

pub struct Worker {
    pub observations: Receiver<Observation>,
    refresh: SyncSender<()>,
}

impl Worker {
    pub fn spawn(
        interval: Duration,
        mut poll: impl FnMut(bool) -> ProviderOutput + Send + 'static,
    ) -> Self {
        let (tx, observations) = mpsc::sync_channel(8);
        let (refresh, requests) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut forced = false;
            loop {
                let started = Instant::now();
                let output = poll(forced);
                let observation = Observation {
                    output,
                    completed_at: SystemTime::now(),
                    duration_ms: started.elapsed().as_millis() as u64,
                };
                if tx.send(observation).is_err() {
                    break;
                }
                // Start-to-start cadence, rather than query duration + interval.
                let delay = interval.saturating_sub(started.elapsed());
                match requests.recv_timeout(delay) {
                    Ok(()) => forced = true,
                    Err(mpsc::RecvTimeoutError::Timeout) => forced = false,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self {
            observations,
            refresh,
        }
    }

    pub fn request_refresh(&self) {
        let _ = self.refresh.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_provider_does_not_block_peer_or_refresh_request() {
        let (release_tx, release_rx) = mpsc::channel();
        let slow = Worker::spawn(Duration::from_secs(60), move |_| {
            let _ = release_rx.recv_timeout(Duration::from_secs(3));
            ProviderOutput::default()
        });
        let fast = Worker::spawn(Duration::from_secs(60), |_| ProviderOutput::default());
        for _ in 0..100 {
            slow.request_refresh();
        }
        assert!(fast
            .observations
            .recv_timeout(Duration::from_secs(1))
            .is_ok());
        assert!(slow.observations.try_recv().is_err());
        release_tx.send(()).unwrap();
        assert!(slow
            .observations
            .recv_timeout(Duration::from_secs(1))
            .is_ok());
    }

    #[test]
    fn manual_refresh_wakes_worker_before_interval() {
        let worker = Worker::spawn(Duration::from_secs(60), |_| ProviderOutput::default());
        worker
            .observations
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        worker.request_refresh();
        worker
            .observations
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
    }
}
