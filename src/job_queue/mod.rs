/// This module implements a prioritized job queue that sends completed
/// job results to the initiator/caller.
///
/// This is intended for running heavy multi-threaded jobs that should be run
/// one at a time to avoid resource contention.  By using this queue, multiple
/// (async) tasks can initiate these tasks and wait for results without need
/// of any other synchronization.
///
/// note: Other rust job queues I found either did not support waiting for job
/// results or else were overly complicated, requiring backend database, etc.
///
/// At present, only blocking (non-async) jobs are supported.  These are
/// called inside spawn_blocking() in order to execute on tokio's blocking
/// thread-pool.  Supporting async tasks would be trivial, if needed.
///
/// An async_priority_channel::unbounded is used for queueing the jobs.
/// This is much like tokio::sync::mpsc::unbounded except:
///  1. it supports prioritizing channel events (jobs)
///  2. order of events with same priority is undefined.
///     see: https://github.com/rmcgibbo/async-priority-channel/issues/75
///
/// Using an unbounded channel means that there is no backpressure and no
/// upper limit on the number of jobs. (except RAM).

pub mod triton_vm_job;

use async_priority_channel as mpsc;
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobPriority {
    Low = 1,
    Medium = 2,
    High = 3,
}

/// represents any kind of job
pub trait Job {
    type JobResult: std::fmt::Debug + Send;

    fn is_async(&self) -> bool;
    fn run(self) -> Self::JobResult;
    fn run_async(self) -> impl std::future::Future<Output = Self::JobResult> + Send;
}


/// implements a prioritized job queue that sends result of each job to a listener.
/// At present order of jobs with the same priority is undefined.
/// todo: fix it so that jobs with same priority execute FIFO.
#[derive(Debug, Clone)]
pub struct JobQueue<T: Job> {
    tx: mpsc::Sender<(T, oneshot::Sender<<T as Job>::JobResult>), JobPriority>,
}

impl<T: Job + Send + Sync + 'static> JobQueue<T> {
    /// creates job queue and starts it processing.  returns immediately.
    pub fn start() -> Self
    where
        <T as Job>::JobResult: std::fmt::Debug + Send,
    {
        let (tx, rx) =
            mpsc::unbounded::<(T, oneshot::Sender<<T as Job>::JobResult>), JobPriority>();

        // spawns background task that processes job-queue and runs jobs.
        tokio::spawn(async move {
            while let Ok(r) = rx.recv().await {
                let (job, otx) = r.0;

                let result = match job.is_async() {
                    true => job.run_async().await,
                    false => tokio::task::spawn_blocking(move || job.run())
                        .await
                        .unwrap()
                };
                let _ = otx.send(result);
            }
        });

        Self { tx }
    }

    /// adds job to job-queue and returns immediately.
    ///
    /// returns a [oneshot::Receiver] that can be awaited on to obtain
    /// the job's result.
    pub async fn add_job(
        &self,
        job: T,
        priority: JobPriority,
    ) -> anyhow::Result<oneshot::Receiver<<T as Job>::JobResult>>
    where
        <T as Job>::JobResult: Send,
    {
        let (otx, orx) = oneshot::channel();
        self.tx.send((job, otx), priority).await?;
        Ok(orx)
    }

    /// adds job to job-queue, waits for job completion, and returns job result.
    pub async fn add_and_await_job(
        &self,
        job: T,
        priority: JobPriority,
    ) -> anyhow::Result<<T as Job>::JobResult>
    where
        <T as Job>::JobResult: Send,
    {
        let (otx, orx) = oneshot::channel();
        self.tx.send((job, otx), priority).await?;
        Ok(orx.await?)
    }

    /// this is a convenience method for tests.
    /// it should not be used for real code without further thought
    /// because the 10 second polling frequency may not be acceptable
    /// depending on caller's use-case.
    #[cfg(test)]
    pub async fn wait_until_queue_empty(&self) {
        loop {
            if self.tx.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// represents a prover job.  implements Job.
    struct DoubleJob {
        data: u64,
    }

    impl Job for DoubleJob {
        type JobResult = (u64, u64);

        async fn run(&self) -> Self::JobResult {
            let r = (self.data, self.data * 2);

            println!("{} * 2 = {}", r.0, r.1);
            r
        }
    }

    #[tokio::test]
    async fn it_works() -> anyhow::Result<()> {
        // create a job queue
        let job_queue = JobQueue::start();

        // create 10 jobs
        for i in 0..10 {
            let job1 = DoubleJob { data: i };
            let job2 = DoubleJob { data: i * 100 };
            let job3 = DoubleJob { data: i * 1000 };

            // process job and print results.
            job_queue.add_job(job1, JobPriority::Low).await?;
            job_queue.add_job(job2, JobPriority::Medium).await?;
            job_queue.add_job(job3, JobPriority::High).await?;
        }

        job_queue.wait_until_queue_empty().await;

        Ok(())
    }
}
