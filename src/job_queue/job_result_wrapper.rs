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
use crate::job_queue::errors::JobResultWrapperError;

/// A generic wrapper around a job-specific result type `T` that implements the
/// [`JobResult`](super::traits::JobResult) trait.
///
/// This wrapper simplifies the process of:
///
/// * Returning concrete job results (`T`) as trait objects (`Box<dyn JobResult>`).
/// * Attempting to convert a `Box<dyn JobResult>` back into the original concrete type `T`.
///
/// The type `T` must be `'static`, `Send`, and `Sync` to be safely used across
/// threads and within the dynamic dispatch context of trait objects.
///
/// # Type Parameters
///
/// * `T`: The specific type of the job result being wrapped. This type must
///     be `'static`, `Send`, and `Sync`. For convenient conversion back from
///     `Box<dyn JobResult>`, it is recommended that `T` also implements
///     [`Debug`](std::fmt::Debug).
pub struct JobResultWrapper<T: 'static + Send + Sync>(T);

impl<T: 'static + Send + Sync> JobResult for JobResultWrapper<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl<T: 'static + Send + Sync> Deref for JobResultWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: 'static + Send + Sync> DerefMut for JobResultWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: 'static + Send + Sync> From<T> for JobResultWrapper<T> {
    fn from(value: T) -> Self {
        JobResultWrapper(value)
    }
}

impl<T: 'static + Send + Sync> From<JobResultWrapper<T>> for Box<dyn JobResult> {
    fn from(wrapper: JobResultWrapper<T>) -> Self {
        Box::new(wrapper) as Box<dyn JobResult>
    }
}

impl<T: 'static + Send + Sync + Debug> TryFrom<Box<dyn JobResult>> for JobResultWrapper<T> {
    type Error = JobResultWrapperError;

    fn try_from(boxed_trait_object: Box<dyn JobResult>) -> Result<Self, JobResultWrapperError> {
        Self::try_from_boxed_job_result(boxed_trait_object)
    }
}

impl<'a, T: 'static + Send + Sync + Debug> TryFrom<&'a Box<dyn JobResult>>
    for &'a JobResultWrapper<T>
{
    type Error = JobResultWrapperError;

    fn try_from(boxed_trait_object: &'a Box<dyn JobResult>) -> Result<Self, Self::Error> {
        let any = boxed_trait_object.as_any();
        if let Some(concrete_wrapper) = any.downcast_ref::<JobResultWrapper<T>>() {
            Ok(concrete_wrapper)
        } else {
            Err(JobResultWrapperError::DowncastError {
                from: std::any::type_name::<dyn JobResult>(),
                to: std::any::type_name::<JobResultWrapper<T>>(),
            })
        }
    }
}

/// The Debug trait bound avoids a conflict with TryFrom implementation in crate
/// `core`.
///
/// If `T` does not impl `Debug` [JobResultWrapper::try_from_boxed_job_result]
/// can be used instead.
impl<T: 'static + Send + Sync + Debug> Debug for JobResultWrapper<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("JobResultWrapper").field(&self.0).finish()
    }
}

impl<T: 'static + Send + Sync + Clone> Clone for JobResultWrapper<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static + Send + Sync + 'static> JobResultWrapper<T> {
    /// convert into inner `T`
    pub fn into_inner(self) -> T {
        self.0
    }

    /// fallibly convert a boxed dyn JobResult into a JobResultWrapper<T>.
    pub fn try_from_boxed_job_result(
        boxed_trait_object: Box<dyn JobResult>,
    ) -> Result<Self, JobResultWrapperError> {
        let any = boxed_trait_object.into_any(); // Convert Box<dyn JobResult> to Box<dyn Any>
        if let Ok(concrete_wrapper) = any.downcast::<JobResultWrapper<T>>() {
            Ok(*concrete_wrapper) // Dereference the Box to get JobResultWrapper<T>
        } else {
            Err(JobResultWrapperError::DowncastError {
                from: std::any::type_name::<dyn JobResult>(),
                to: std::any::type_name::<JobResultWrapper<T>>(),
            })
        }
    }

    /// fallibly convert a boxed dyn JobResult reference into a JobResultWrapper<T>.
    pub fn try_from_boxed_job_result_ref(
        boxed_trait_object: &Box<dyn JobResult>,
    ) -> Result<&Self, JobResultWrapperError> {
        let any = boxed_trait_object.as_any();
        if let Some(concrete_wrapper) = any.downcast_ref::<JobResultWrapper<T>>() {
            Ok(concrete_wrapper)
        } else {
            Err(JobResultWrapperError::DowncastError {
                from: std::any::type_name::<dyn JobResult>(),
                to: std::any::type_name::<JobResultWrapper<T>>(),
            })
        }
    }
}
