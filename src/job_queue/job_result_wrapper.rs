use std::any::Any;
use std::ops::{Deref, DerefMut};
use super::traits::JobResult;
use std::fmt::Debug;
use crate::job_queue::errors::JobResultWrapperError;

#[derive(PartialEq, Clone)]
pub struct JobResultWrapper<T: 'static + Send + Sync + Clone>(T);

impl<T: 'static + Send + Sync + Clone> JobResult for JobResultWrapper<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl<T: 'static + Send + Sync + Clone> Deref for JobResultWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: 'static + Send + Sync + Clone> DerefMut for JobResultWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: 'static + Send + Sync + Clone> From<T> for JobResultWrapper<T> {
    fn from(value: T) -> Self {
        JobResultWrapper(value)
    }
}

impl<T: 'static + Send + Sync + Clone> From<JobResultWrapper<T>> for Box<dyn JobResult> {
    fn from(wrapper: JobResultWrapper<T>) -> Self {
        Box::new(wrapper) as Box<dyn JobResult>
    }
}

impl<T: 'static + Send + Sync + Clone + 'static> TryFrom<Box<dyn JobResult>> for JobResultWrapper<T> {
    type Error = JobResultWrapperError;

    fn try_from(boxed_trait_object: Box<dyn JobResult>) -> Result<Self, Self::Error> {
        let any = boxed_trait_object.as_any();
        if let Some(wrapper) = any.downcast_ref::<JobResultWrapper<T>>() {
            Ok(wrapper.clone())
        } else {
            Err(JobResultWrapperError {
                from: std::any::type_name::<dyn JobResult>(),
                to: std::any::type_name::<JobResultWrapper<u64>>(),
            })
        }
    }
}

impl<T: 'static + Send + Sync + Clone + Debug> Debug for JobResultWrapper<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("JobResultWrapper")
            .field(&self.0)
            .finish()
    }
}

impl<T: 'static + Send + Sync + Clone + 'static> JobResultWrapper<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}