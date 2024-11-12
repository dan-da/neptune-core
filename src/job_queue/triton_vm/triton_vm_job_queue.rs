use super::super::JobQueue;

// todo: maybe we want to have more levels or just make it an integer eg u8.
// or maybe name the levels by type/usage of job/proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum TritonVmJobPriority {
    Lowest = 1,
    Low = 2,
    #[default]
    Normal = 3,
    High = 4,
    Highest = 5,
}

#[derive(Clone, Copy, Debug)]
pub struct TritonVmProofJobOptions {
    pub job_priority: TritonVmJobPriority,
    pub max_log2_padded_height_for_proofs: Option<u8>,
}
impl From<TritonVmJobPriority> for TritonVmProofJobOptions {
    fn from(job_priority: TritonVmJobPriority) -> Self {
        Self {
            job_priority,
            max_log2_padded_height_for_proofs: None,
        }
    }
}
impl From<(TritonVmJobPriority, Option<u8>)> for TritonVmProofJobOptions {
    fn from(v: (TritonVmJobPriority, Option<u8>)) -> Self {
        let (job_priority, max_log2_padded_height_for_proofs) = v;
        Self {
            job_priority,
            max_log2_padded_height_for_proofs,
        }
    }
}

/// provides type safety and clarity in case we implement multiple job queues.
pub type TritonVmJobQueue = JobQueue<TritonVmJobPriority>;
