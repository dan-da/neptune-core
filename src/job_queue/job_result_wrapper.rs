//! This module provides type `JobResultWrapper` to enhance the ergonomics of
//! working with job-specific result types which must implement the `JobResult`
//! trait.
//!
//! It is useful for:
//!
//! 1. returning job results of type T as Box<dyn JobResults> when implementing
//!    the `Job` trait.
//!
//! 2. converting the Box<dyn JobResults> from a completed `Job` back into `T`.
//!
//! See [module docs](super) for usage examples.
use std::any::Any;
use std::fmt::Debug;
use std::ops::Deref;
use std::ops::DerefMut;

use super::traits::JobResult;
use crate::job_queue::errors::JobHandleError;

/// A generic wrapper around a job-specific result type `T` that implements the
/// [`JobResult`](super::traits::JobResult) trait.
///
/// This wrapper simplifies the process of:
///
/// * Returning concrete job results (`T`) as trait objects (`Box<dyn JobResult>`).
/// * Attempting to convert a `Box<dyn JobResult>` back into the original concrete type `T`.
///
/// # Type Parameters
///
/// * `T`: The specific type of the job result being wrapped. This type must be
///   `'static`, `Send`, and `Sync`. For convenient conversion back from
///   `Box<dyn JobResult>`, it is recommended that `T` also implements
///   [`Debug`](std::fmt::Debug).
pub struct JobResultWrapper<T>(T);

impl<T: 'static + Send + Sync> JobResult for JobResultWrapper<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl<T> Deref for JobResultWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for JobResultWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> From<T> for JobResultWrapper<T> {
    fn from(value: T) -> Self {
        JobResultWrapper(value)
    }
}

impl<T: 'static + Send + Sync> From<JobResultWrapper<T>> for Box<dyn JobResult> {
    fn from(wrapper: JobResultWrapper<T>) -> Self {
        Box::new(wrapper) as Box<dyn JobResult>
    }
}

/// The Debug trait bound avoids a conflict with TryFrom implementation in crate
/// `core`.
///
/// If `T` does not impl `Debug` [JobResultWrapper::try_from_boxed_job_result]
/// can be used instead.
impl<T: 'static + Debug> TryFrom<Box<dyn JobResult>> for JobResultWrapper<T> {
    type Error = JobHandleError;

    fn try_from(boxed_trait_object: Box<dyn JobResult>) -> Result<Self, JobHandleError> {
        Self::try_from_boxed_job_result(boxed_trait_object)
    }
}

impl<'a, T: 'static + Debug> TryFrom<&'a dyn JobResult> for &'a JobResultWrapper<T> {
    type Error = JobHandleError;

    fn try_from(boxed_trait_object: &'a dyn JobResult) -> Result<Self, Self::Error> {
        JobResultWrapper::try_from_boxed_job_result_ref(boxed_trait_object)
    }
}

impl<T: Debug> Debug for JobResultWrapper<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("JobResultWrapper").field(&self.0).finish()
    }
}

impl<T: Clone> Clone for JobResultWrapper<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> JobResultWrapper<T> {
    /// convert into inner `T`
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: 'static> JobResultWrapper<T> {

    /// fallibly convert a boxed dyn JobResult into a JobResultWrapper<T>.
    pub fn try_from_boxed_job_result(
        boxed_trait_object: Box<dyn JobResult>,
    ) -> Result<Self, JobHandleError> {
        let any = boxed_trait_object.into_any(); // Convert Box<dyn JobResult> to Box<dyn Any>
        if let Ok(concrete_wrapper) = any.downcast::<JobResultWrapper<T>>() {
            Ok(*concrete_wrapper) // Dereference the Box to get JobResultWrapper<T>
        } else {
            Err(JobHandleError::JobResultWrapperError {
                from: std::any::type_name::<dyn JobResult>(),
                to: std::any::type_name::<JobResultWrapper<T>>(),
            })
        }
    }

    /// fallibly convert a boxed dyn JobResult reference into a JobResultWrapper<T>.
    pub fn try_from_boxed_job_result_ref(
        boxed_trait_object: &dyn JobResult,
    ) -> Result<&Self, JobHandleError> {
        let any = boxed_trait_object.as_any();
        if let Some(concrete_wrapper) = any.downcast_ref::<JobResultWrapper<T>>() {
            Ok(concrete_wrapper)
        } else {
            Err(JobHandleError::JobResultWrapperError {
                from: std::any::type_name::<dyn JobResult>(),
                to: std::any::type_name::<JobResultWrapper<T>>(),
            })
        }
    }
}
