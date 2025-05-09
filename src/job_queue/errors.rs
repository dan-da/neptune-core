use std::sync::Arc;
use std::sync::Mutex;

/// a job-handle error.
#[derive(Debug, thiserror::Error)]
pub enum JobHandleError {
    #[error("the job was cancelled")]
    JobCancelled,

    #[error("the job panicked during processing")]
    JobPanicked(Arc<Mutex<Box<dyn std::any::Any + Send + 'static>>>),

    #[error("channel send error cancelling job")]
    CancelJobError(#[from] tokio::sync::watch::error::SendError<()>),

    #[error("channel recv error waiting for job results: {0}")]
    JobResultError(#[from] tokio::sync::oneshot::error::RecvError),

    #[error("downcast failed converting '{from}' to '{to}'")]
    JobResultWrapperError {
        from: &'static str,
        to: &'static str,
    },
}

impl JobHandleError {
    pub fn panic_message(&self) -> Option<String> {
        match self {
            JobHandleError::JobPanicked(payload) => {
                let guard = payload.lock().unwrap();
                if let Some(s) = guard.downcast_ref::<&'static str>() {
                    Some((*s).to_string())
                } else {
                    guard.downcast_ref::<String>().cloned()
                }
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum AddJobError {
    #[error("channel send error adding job.  error: {0}")]
    SendError(#[from] tokio::sync::mpsc::error::SendError<()>),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StopQueueError {
    #[error("channel send error adding job.  error: {0}")]
    SendError(#[from] tokio::sync::watch::error::SendError<()>),

    #[error("join error while waiting for job-queue to stop.  error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}
