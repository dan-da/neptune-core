//! This module implements a prioritized, heterogenous job queue that sends
//! completed job results of arbitrary type to the initiator/caller.
//!
//! This is intended for running heavy multi-threaded jobs that should be run
//! one at a time to avoid resource contention.  By using this queue, multiple
//! (async) tasks can initiate these tasks and wait for results without need
//! of any other synchronization.
//!
//! note: Other rust job queues investigated cerca 2024 either did not support
//! waiting for job results or else were overly complicated, requiring backend
//! database, etc.
//!
//! Both blocking and non-blocking (async) jobs are supported.  Non-blocking jobs
//! are called inside spawn_blocking() in order to execute on tokio's blocking
//! thread-pool.  Async jobs are simply awaited.
//!
//! It supports prioritizing Jobs. The order of job execution is not a simple
//! FIFO or LIFO but rather depends on the assigned priority of each job.
//! Job priority level can be specified via any type that implements [Ord]
//! such as a custom enum.
//!
//! There is no upper limit on the number of jobs. (except RAM).
//!
//! Jobs may be of mixed (heterogenous) types in a single [JobQueue] instance.
//! Any type that implements the [Job](traits::Job) trait may be a job.
//!
//! Job results also may be of any type.  Typically each type of Job will return
//! a single concrete result type.  A [JobResultWrapper] is provided to
//! facilitate this usage pattern.
//!
//! Each Job has an associated [JobHandle] that is used to await or cancel the
//! job.  If the `JobHandle` is dropped, the job will be cancelled.
//!
//! Example:
//!
//! ```
//! use neptune_cash::job_queue::JobResultWrapper;
//! use neptune_cash::job_queue::JobQueue;
//! use neptune_cash::job_queue::traits::*;
//!
//! type FindPrimesJobResult = JobResultWrapper<Vec<u128>>;
//!
//! // represents a custom job.  implements Job.
//! #[derive(Debug)]
//! struct MyJob {
//!     data: u64,
//! }
//!
//! #[async_trait::async_trait]
//! impl Job for MyJob {
//!     fn is_async(&self) -> bool {
//!         true
//!     }
//!
//!     async fn run_async(&self) -> Box<dyn JobResult> {
//!         tokio::time::sleep(self.duration).await;
//!         MyJobResult::from((self.data, self.data * 2, Instant::now())).into()
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result() {
//!
//!     let job_queue = JobQueue::start();
//!     let job = MyJob {
//!         data: 15,
//!         duration: std::time::Duration::from_secs(5),
//!     };
//!     let job_handle = job_queue.add_job(job, 10usize)?;
//!     let job_result: MyJobResult = job_handle.await?.result()?.try_into()?;
//!     let answer = job_result.into_inner();
//!
//!     assert_eq!(answer.0 * 2, answer.1);
//!
//!     Ok(())
//! }
//! ```

// please note that the job_queue module has zero neptune-core specific
// code in it.  It is intended/planned to move job_queue into its own
// crate in the (near) future.

pub mod channels;
pub mod errors;
mod job_completion;
mod job_handle;
mod job_id;
mod job_result_wrapper;
mod queue;
pub mod traits;

pub use job_completion::JobCompletion;
pub use job_handle::JobHandle;
pub use job_id::JobId;
pub use job_result_wrapper::JobResultWrapper;
pub use queue::JobQueue;
