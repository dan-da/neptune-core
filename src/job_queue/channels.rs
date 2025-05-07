use std::ops::Deref;
use std::ops::DerefMut;

use tokio::sync::oneshot;
use tokio::sync::watch;

use super::job_completion::JobCompletion;

//pub type JobCancelReceiver = watch::Receiver<()>; // used in pub trait
//pub(super) type JobCancelSender = watch::Sender<()>;
pub type JobCancelReceiver = LogWhenDropped<watch::Receiver<()>>; // used in pub trait
pub(super) type JobCancelSender = LogWhenDropped<watch::Sender<()>>;

pub(super) type JobResultReceiver = oneshot::Receiver<JobCompletion>;
pub(super) type JobResultSender = oneshot::Sender<JobCompletion>;

pub struct LogWhenDropped<T>(pub T);

impl<T> Deref for LogWhenDropped<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for LogWhenDropped<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> Drop for LogWhenDropped<T> {
    fn drop(&mut self) {
        tracing::info!("LogWhenDropped<{}> dropped!", std::any::type_name::<T>());
    }
}

impl<T: Clone> Clone for LogWhenDropped<T> {
    fn clone(&self) -> Self {
        LogWhenDropped(self.0.clone())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for LogWhenDropped<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LogWhenDropped").field(&self.0).finish()
    }
}
