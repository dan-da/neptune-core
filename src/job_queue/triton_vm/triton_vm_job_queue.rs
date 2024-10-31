use super::super::traits::Job;
use super::super::traits::JobResult;
use super::super::JobPriority;
use super::super::JobQueue;

use tokio::sync::oneshot;

/// newtype for JobQueue that is specific to triton-vm jobs.
/// provides type safety and clarity in case we implement multiple job queues.
#[derive(Debug, Clone)]
pub struct TritonVmJobQueue(JobQueue);

impl TritonVmJobQueue {
    pub fn start() -> Self {
        Self(JobQueue::start())
    }
    // alias of Self::start().
    // here for two reasons:
    //  1. backwards compat with existing tests
    //  2. if tests call dummy() instead of start(), then it is easier
    //     to find where start() is called for real.
    #[cfg(test)]
    pub fn dummy() -> Self {
        Self::start()
    }

    // adds job to job-queue and returns immediately.
    pub async fn add_job(
        &self,
        job: Box<dyn Job>,
        priority: JobPriority,
    ) -> anyhow::Result<oneshot::Receiver<Box<dyn JobResult>>> {
        self.0.add_job(job, priority).await
    }

    // adds job to job-queue, waits for job completion, and returns job result.
    pub async fn add_and_await_job(
        &self,
        job: Box<dyn Job>,
        priority: JobPriority,
    ) -> anyhow::Result<Box<dyn JobResult>> {
        self.0.add_and_await_job(job, priority).await
    }

    #[cfg(test)]
    pub async fn wait_until_queue_empty(&self) {
        self.0.wait_until_queue_empty().await
    }
}
