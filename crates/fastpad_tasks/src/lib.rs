use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn throw_if_cancelled(&self) -> Result<(), TaskError> {
        if self.is_cancelled() {
            Err(TaskError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskProgress {
    pub name: String,
    pub processed_bytes: u64,
    pub total_bytes: Option<u64>,
    pub message: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task cancelled")]
    Cancelled,
    #[error("task panicked")]
    Panicked,
}

pub struct TaskHandle<T> {
    name: String,
    token: CancellationToken,
    progress: Receiver<TaskProgress>,
    join: Option<JoinHandle<T>>,
}

impl<T: Send + 'static> TaskHandle<T> {
    pub fn spawn<F>(name: impl Into<String>, f: F) -> Self
    where
        F: FnOnce(CancellationToken, Sender<TaskProgress>) -> T + Send + 'static,
    {
        let name = name.into();
        let token = CancellationToken::new();
        let thread_token = token.clone();
        let (sender, progress) = unbounded();
        let thread_name = name.clone();
        let join = thread::Builder::new()
            .name(thread_name)
            .spawn(move || f(thread_token, sender))
            .expect("spawn background task");

        Self {
            name,
            token,
            progress,
            join: Some(join),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }

    pub fn cancel(&self) {
        self.token.cancel();
    }

    pub fn progress(&self) -> &Receiver<TaskProgress> {
        &self.progress
    }

    pub fn join(mut self) -> Result<T, TaskError> {
        self.join
            .take()
            .expect("join handle present")
            .join()
            .map_err(|_| TaskError::Panicked)
    }
}

#[derive(Debug, Clone)]
pub struct ProgressThrottle {
    last_emit: Instant,
    interval: Duration,
}

impl ProgressThrottle {
    pub fn new(interval: Duration) -> Self {
        Self {
            last_emit: Instant::now() - interval,
            interval,
        }
    }

    pub fn should_emit(&mut self) -> bool {
        if self.last_emit.elapsed() >= self.interval {
            self.last_emit = Instant::now();
            true
        } else {
            false
        }
    }
}
