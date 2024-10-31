use super::super::JobQueue;

// todo: maybe we want to have more levels or just make it an integer eg u8.
// or maybe name the levels by type/usage of job/proof.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TritonVmJobPriority {
    Lowest = 1,
    Low = 2,
    Normal = 3,
    High = 4,
    Highest = 5,
}

/// provides type safety and clarity in case we implement multiple job queues.
pub type TritonVmJobQueue = JobQueue<TritonVmJobPriority>;
