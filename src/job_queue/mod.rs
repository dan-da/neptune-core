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
use std::any::Any;
use tokio::sync::oneshot;

// todo: maybe we want to have more levels or just make it an integer eg u8.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobPriority {
    Low = 1,
    Medium = 2,
    High = 3,
}

pub trait JobResult: Any + Send + Sync + std::fmt::Debug {
    fn as_any(&self) -> &dyn Any;
}

// represents any kind of job
#[async_trait::async_trait]
pub trait Job: Send + Sync {
    fn is_async(&self) -> bool;

    // note: we provide unimplemented default methods for
    // run and run_async.  This is so that implementing types
    // only need to impl the appropriate method.

    fn run(&self) -> Box<dyn JobResult> {
        unimplemented!()
    }

    // fn run_async(&self) ->  std::future::Future<Output = Box<dyn JobResult>> + Send;
    async fn run_async(&self) -> Box<dyn JobResult> {
        unimplemented!()
    }
}

/// implements a prioritized job queue that sends result of each job to a listener.
/// At present order of jobs with the same priority is undefined.
/// todo: fix it so that jobs with same priority execute FIFO.

type JobResultOneShotChannel = oneshot::Sender<Box<dyn JobResult>>;

pub struct JobQueue {
    tx: mpsc::Sender<(Box<dyn Job>, JobResultOneShotChannel), JobPriority>,
}

impl std::fmt::Debug for JobQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobQueue")
            .field("tx", &"mpsc::Sender")
            .finish()
    }
}

impl Clone for JobQueue {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl JobQueue {
    // creates job queue and starts it processing.  returns immediately.
    pub fn start() -> Self {
        let (tx, rx) = mpsc::unbounded::<(Box<dyn Job>, JobResultOneShotChannel), JobPriority>();

        // spawns background task that processes job-queue and runs jobs.
        tokio::spawn(async move {
            while let Ok(r) = rx.recv().await {
                let (job, otx) = r.0;

                let result = match job.is_async() {
                    true => job.run_async().await,
                    false => tokio::task::spawn_blocking(move || job.run())
                        .await
                        .unwrap(),
                };
                let _ = otx.send(result);
            }
        });

        Self { tx }
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
        let (otx, orx) = oneshot::channel();
        self.tx.send((job, otx), priority).await?;
        Ok(orx)
    }

    // adds job to job-queue, waits for job completion, and returns job result.
    pub async fn add_and_await_job(
        &self,
        job: Box<dyn Job>,
        priority: JobPriority,
    ) -> anyhow::Result<Box<dyn JobResult>> {
        let (otx, orx) = oneshot::channel();
        self.tx.send((job, otx), priority).await?;
        Ok(orx.await?)
    }

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

// use async_priority_channel as mpsc;
// use tokio::sync::oneshot;

// #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
// pub enum JobPriority {
//     Low = 1,
//     Medium = 2,
//     High = 3,
// }

// /// represents any kind of job
// pub trait Job {
//     type JobResult: std::fmt::Debug + Send;

//     fn is_async(&self) -> bool;

//     // note: we provide unimplemented default methods for
//     // run.  This is so that implementing types only need to impl
//     // if they need a blocking run().
//     // We would *like* to do the same for run_async, however rust
//     // doesn't allow unimplemented!() in a fn that returns impl trait.
//     // see: https://github.com/rust-lang/rust/issues/69882

//     fn run(self) -> Self::JobResult
//     where
//         Self: Sized,
//     {
//         unimplemented!()
//     }

//     fn run_async(self) -> impl std::future::Future<Output = Self::JobResult> + Send;
// }

// /// implements a prioritized job queue that sends result of each job to a listener.
// /// At present order of jobs with the same priority is undefined.
// /// todo: fix it so that jobs with same priority execute FIFO.
// #[derive(Debug)]
// pub struct JobQueue<T: Job> {
//     tx: mpsc::Sender<(T, oneshot::Sender<<T as Job>::JobResult>), JobPriority>,
// }

// impl<T: Job> Clone for JobQueue<T> {
//     fn clone(&self) -> Self {
//         Self {
//             tx: self.tx.clone(),
//         }
//     }
// }

// impl<T: Job + Send + Sync + 'static> JobQueue<T> {
//     /// creates job queue and starts it processing.  returns immediately.
//     pub fn start() -> Self
//     where
//         <T as Job>::JobResult: std::fmt::Debug + Send,
//     {
//         let (tx, rx) =
//             mpsc::unbounded::<(T, oneshot::Sender<<T as Job>::JobResult>), JobPriority>();

//         // spawns background task that processes job-queue and runs jobs.
//         tokio::spawn(async move {
//             while let Ok(r) = rx.recv().await {
//                 let (job, otx) = r.0;

//                 let result = match job.is_async() {
//                     true => job.run_async().await,
//                     false => tokio::task::spawn_blocking(move || job.run())
//                         .await
//                         .unwrap(),
//                 };
//                 let _ = otx.send(result);
//             }
//         });

//         Self { tx }
//     }

//     // alias of Self::start().
//     // here for two reasons:
//     //  1. backwards compat with existing tests
//     //  2. if tests call dummy() instead of start(), then it is easier
//     //     to find where start() is called for real.
//     #[cfg(test)]
//     pub fn dummy() -> Self {
//         Self::start()
//     }

//     /// adds job to job-queue and returns immediately.
//     ///
//     /// returns a [oneshot::Receiver] that can be awaited on to obtain
//     /// the job's result.
//     pub async fn add_job(
//         &self,
//         job: T,
//         priority: JobPriority,
//     ) -> anyhow::Result<oneshot::Receiver<<T as Job>::JobResult>>
//     where
//         <T as Job>::JobResult: Send,
//     {
//         let (otx, orx) = oneshot::channel();
//         self.tx.send((job, otx), priority).await?;
//         Ok(orx)
//     }

//     /// adds job to job-queue, waits for job completion, and returns job result.
//     pub async fn add_and_await_job(
//         &self,
//         job: T,
//         priority: JobPriority,
//     ) -> anyhow::Result<<T as Job>::JobResult>
//     where
//         <T as Job>::JobResult: Send,
//     {
//         let (otx, orx) = oneshot::channel();
//         self.tx.send((job, otx), priority).await?;
//         Ok(orx.await?)
//     }

//     /// this is a convenience method for tests.
//     /// it should not be used for real code without further thought
//     /// because the 10 second polling frequency may not be acceptable
//     /// depending on caller's use-case.
//     #[cfg(test)]
//     pub async fn wait_until_queue_empty(&self) {
//         loop {
//             if self.tx.is_empty() {
//                 break;
//             }
//             tokio::time::sleep(std::time::Duration::from_millis(10)).await;
//         }
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(PartialEq, Debug)]
    struct DoubleJobResult(u64, u64);
    impl JobResult for DoubleJobResult {
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    // a job that doubles the input value.  implements Job.
    struct DoubleJob {
        data: u64,
    }

    impl Job for DoubleJob {
        fn is_async(&self) -> bool {
            false
        }

        fn run(&self) -> Box<dyn JobResult> {
            let r = DoubleJobResult(self.data, self.data * 2);

            println!("{} * 2 = {}", r.0, r.1);
            Box::new(r)
        }
    }

    // todo: make test(s) for async jobs.

    /// todo: this should verify the priority order of jobs.
    ///       presently each job just prints result and
    ///       human can manually verify output.
    #[tokio::test]
    async fn run_jobs_by_priority() -> anyhow::Result<()> {
        // create a job queue
        let job_queue = JobQueue::start();

        // create 10 jobs
        for i in 0..10 {
            let job1 = Box::new(DoubleJob { data: i });
            let job2 = Box::new(DoubleJob { data: i * 100 });
            let job3 = Box::new(DoubleJob { data: i * 1000 });

            // process job and print results.
            job_queue.add_job(job1, JobPriority::Low).await?;
            job_queue.add_job(job2, JobPriority::Medium).await?;
            job_queue.add_job(job3, JobPriority::High).await?;
        }

        job_queue.wait_until_queue_empty().await;

        Ok(())
    }

    #[tokio::test]
    async fn get_result() -> anyhow::Result<()> {
        // create a job queue
        let job_queue = JobQueue::start();

        // create 10 jobs
        for i in 0..10 {
            let job = Box::new(DoubleJob { data: i });

            let result = job_queue.add_and_await_job(job, JobPriority::Low).await?;
            assert_eq!(
                Some(&DoubleJobResult(i, i * 2)),
                result.as_any().downcast_ref::<DoubleJobResult>()
            );
        }

        Ok(())
    }
}
