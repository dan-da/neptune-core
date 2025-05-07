use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use super::channels::JobCancelSender;
use super::channels::JobResultReceiver;
use super::errors::JobHandleError;
use super::job_completion::JobCompletion;
use super::job_id::JobId;

/// A job-handle enables cancelling a job and awaiting results
///
/// A JobHandle can be awaited directly.  It returns a
/// `Result<JobCompletion, JobHandleError>`
///
/// See [JobCompletion] and [JobHandleError] for details.
///
/// When the `JobHandle` is dropped a cancellation message is sent to the job
/// task.
#[derive(Debug)]
pub struct JobHandle {
    job_id: JobId,
    result_rx: JobResultReceiver,
    cancel_tx: JobCancelSender,
}
impl JobHandle {
    // private instantiation fn.  only for use by JobQueue
    pub(super) fn new(
        job_id: JobId,
        result_rx: JobResultReceiver,
        cancel_tx: JobCancelSender,
    ) -> Self {
        Self {
            job_id,
            result_rx,
            cancel_tx,
        }
    }

    /// sends cancel message to job and returns immediately.
    ///
    /// note: await the JobHandle after calling `cancel()` to ensure the job has
    /// ended and obtain a [JobCompletion]
    pub fn cancel(&self) -> Result<(), JobHandleError> {
        Ok(self.cancel_tx.send(())?)
    }

    /// obtain randomly generated job identifier
    pub fn job_id(&self) -> JobId {
        self.job_id
    }
}

// we implement Future for JobHandle so that a JobHandle can be
// directly awaited (like a tokio JoinHandle).
impl Future for JobHandle {
    type Output = Result<JobCompletion, JobHandleError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Directly poll the underlying result_rx
        let result_rx = &mut self.get_mut().result_rx;
        Pin::new(result_rx).poll(cx).map_err(|e| e.into())
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        tracing::debug!("JobHandle dropping for job: {}", self.job_id);
        if !self.cancel_tx.is_closed() {
            if let Err(e) = self.cancel_tx.send(()) {
                tracing::error!("job-cancel message could not be sent. {}", e);
            } else {
                tracing::debug!("Sent job-cancel msg to job: {}", self.job_id);
            }
        }
    }
}
