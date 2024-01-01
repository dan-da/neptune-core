use futures::future::BoxFuture;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use super::{LockEvent, LockType, LockCallbackFn, LockCallbackInfo, LockAcquisition};
use std::ops::{Deref, DerefMut};

/// An `Arc<RwLock<T>>` wrapper to make data thread-safe and easy to work with.
///
/// # Examples
/// ```
/// # use neptune_core::util_types::sync::tokio::AtomicRw;
/// struct Car {
///     year: u16,
/// };
/// # tokio_test::block_on(async {
/// let atomic_car = AtomicRw::from(Car{year: 2016});
/// atomic_car.lock(|c| {println!("year: {}", c.year)}).await;
/// atomic_car.lock_mut(|mut c| {c.year = 2023}).await;
/// # })
/// ```
///
/// It is also possible to provide a name and callback fn/// during instantiation.  In this way, the application
/// can easily trace lock acquisitions.
///
/// # Examples
/// ```
/// # use neptune_core::util_types::sync::tokio::AtomicRw;
/// struct Car {
///     year: u16,
/// };
///
/// pub fn log_lock_acquired(is_mut: bool, name: Option<&str>) {
///     let tokio_id = match tokio::task::try_id() {
///         Some(id) => format!("{}", id),
///         None => "[None]".to_string(),
///     };
///     println!(
///         "thread {{name: `{}`, id: {:?}}}, tokio task {} acquired lock `{}` for {}",
///         std::thread::current().name().unwrap_or("?"),
///         std::thread::current().id(),
///         tokio_id,
///         name.unwrap_or("?"),
///         if is_mut { "write" } else { "read" }
///     );
/// }
/// const LOG_LOCK_ACQUIRED_CB: fn(is_mut: bool, name: Option<&str>) = log_lock_acquired;
///
/// # tokio_test::block_on(async {
/// let atomic_car = AtomicRw::<Car>::from((Car{year: 2016}, Some("car"), Some(LOG_LOCK_ACQUIRED_CB)));
/// atomic_car.lock(|c| {println!("year: {}", c.year)}).await;
/// atomic_car.lock_mut(|mut c| {c.year = 2023}).await;
/// # })
/// ```
///
/// results in:
#[derive(Debug)]
pub struct AtomicRw<T> {
    inner: Arc<RwLock<T>>,
    lock_callback_info: LockCallbackInfo,
}

impl<T: Default> Default for AtomicRw<T> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
            lock_callback_info: LockCallbackInfo::new(LockType::RwLock, None, None),
        }
    }
}

impl<T> From<T> for AtomicRw<T> {
    #[inline]
    fn from(t: T) -> Self {
        Self {
            inner: Arc::new(RwLock::new(t)),
            lock_callback_info: LockCallbackInfo::new(LockType::Mutex, None, None),
        }
    }
}
impl<T> From<(T, Option<String>, Option<LockCallbackFn>)> for AtomicRw<T> {
    /// Create from an optional name and an optional callback function, which
    /// is called when a lock is acquired.
    #[inline]
    fn from(v: (T, Option<String>, Option<LockCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(RwLock::new(v.0)),
            lock_callback_info: LockCallbackInfo::new(LockType::Mutex, v.1, v.2),
        }
    }
}
impl<T> From<(T, Option<&str>, Option<LockCallbackFn>)> for AtomicRw<T> {
    /// Create from a name ref and an optional callback function, which
    /// is called when a lock is acquired.
    #[inline]
    fn from(v: (T, Option<&str>, Option<LockCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(RwLock::new(v.0)),
            lock_callback_info: LockCallbackInfo::new(LockType::Mutex, v.1.map(|s| s.to_owned()), v.2),
        }
    }
}

impl<T> Clone for AtomicRw<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            lock_callback_info: self.lock_callback_info.clone(),
        }
    }
}

impl<T> From<RwLock<T>> for AtomicRw<T> {
    #[inline]
    fn from(t: RwLock<T>) -> Self {
        Self {
            inner: Arc::new(t),
            lock_callback_info: LockCallbackInfo::new(LockType::Mutex, None, None),
        }
    }
}
impl<T> From<(RwLock<T>, Option<String>, Option<LockCallbackFn>)> for AtomicRw<T> {
    /// Create from an RwLock<T> plus an optional name
    /// and an optional callback function, which is called
    /// when a lock is acquired.
    #[inline]
    fn from(v: (RwLock<T>, Option<String>, Option<LockCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(v.0),
            lock_callback_info: LockCallbackInfo::new(LockType::Mutex, v.1, v.2),
        }
    }
}

impl<T> TryFrom<AtomicRw<T>> for RwLock<T> {
    type Error = Arc<RwLock<T>>;
    fn try_from(t: AtomicRw<T>) -> Result<RwLock<T>, Self::Error> {
        Arc::<RwLock<T>>::try_unwrap(t.inner)
    }
}

impl<T> From<Arc<RwLock<T>>> for AtomicRw<T> {
    #[inline]
    fn from(t: Arc<RwLock<T>>) -> Self {
        Self {
            inner: t,
            lock_callback_info: LockCallbackInfo::new(LockType::Mutex, None, None),
        }
    }
}
impl<T> From<(Arc<RwLock<T>>, Option<String>, Option<LockCallbackFn>)> for AtomicRw<T> {
    /// Create from an `Arc<RwLock<T>>` plus an optional name and
    /// an optional callback function, which is called when a lock
    /// is acquired.
    #[inline]
    fn from(v: (Arc<RwLock<T>>, Option<String>, Option<LockCallbackFn>)) -> Self {
        Self {
            inner: v.0,
            lock_callback_info: LockCallbackInfo::new(LockType::Mutex, v.1, v.2),
        }
    }
}

impl<T> From<AtomicRw<T>> for Arc<RwLock<T>> {
    #[inline]
    fn from(t: AtomicRw<T>) -> Self {
        t.inner
    }
}

// note: we impl the Atomic trait methods here also so they
// can be used without caller having to use the trait.
impl<T> AtomicRw<T> {
    /// Acquire read lock and return an `AtomicRwReadGuard`
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicRw;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicRw::from(Car{year: 2016});
    /// let year = atomic_car.lock_guard().await.year;
    /// # })
    /// ```
    pub async fn lock_guard(&self) -> AtomicRwReadGuard<T> {
        let guard = self.inner.read().await;
        AtomicRwReadGuard::new(guard, &self.lock_callback_info)
    }

    /// Acquire write lock and return an `AtomicRwWriteGuard`
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicRw;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicRw::from(Car{year: 2016});
    /// atomic_car.lock_guard_mut().await.year = 2022;
    /// # })
    /// ```
    pub async fn lock_guard_mut(&self) -> AtomicRwWriteGuard<T> {
        let guard = self.inner.write().await;
        AtomicRwWriteGuard::new(guard, &self.lock_callback_info)
    }

    /// Immutably access the data of type `T` in a closure and possibly return a result of type `R`
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicRw;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicRw::from(Car{year: 2016});
    /// atomic_car.lock(|c| println!("year: {}", c.year)).await;
    /// let year = atomic_car.lock(|c| c.year).await;
    /// })
    /// ```
    pub async fn lock<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let inner_guard = self.inner.read().await;
        let guard = AtomicRwReadGuard::new(inner_guard, &self.lock_callback_info);
        f(&guard)
    }

    /// Mutably access the data of type `T` in a closure and possibly return a result of type `R`
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicRw;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicRw::from(Car{year: 2016});
    /// atomic_car.lock_mut(|mut c| c.year = 2022).await;
    /// let year = atomic_car.lock_mut(|mut c| {c.year = 2023; c.year}).await;
    /// })
    /// ```
    pub async fn lock_mut<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let inner_guard = self.inner.write().await;
        let mut guard = AtomicRwWriteGuard::new(inner_guard, &self.lock_callback_info);
        f(&mut guard)
    }

    /// Immutably access the data of type `T` in an async closure and possibly return a result of type `R`
    ///
    /// The async callback uses dynamic dispatch and it is necessary to call
    /// `.boxed()` on the closure's async block and have [`FutureExt`](futures::future::FutureExt) in scope.
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicRw;
    /// # use futures::future::FutureExt;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicRw::from(Car{year: 2016});
    /// atomic_car.lock_async(|c| async {println!("year: {}", c.year)}.boxed()).await;
    /// let year = atomic_car.lock_async(|c| async {c.year}.boxed()).await;
    /// })
    /// ```
    // design background: https://stackoverflow.com/a/77657788/10087197
    pub async fn lock_async<R>(&self, f: impl FnOnce(&T) -> BoxFuture<'_, R>) -> R {
        let inner_guard = self.inner.read().await;
        let guard = AtomicRwReadGuard::new(inner_guard, &self.lock_callback_info);
        f(&guard).await
    }

    /// Mutably access the data of type `T` in an async closure and possibly return a result of type `R`
    ///
    /// The async callback uses dynamic dispatch and it is necessary to call
    /// `.boxed()` on the closure's async block and have [`FutureExt`](futures::future::FutureExt) in scope.
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicRw;
    /// # use futures::future::FutureExt;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicRw::from(Car{year: 2016});
    /// atomic_car.lock_mut_async(|mut c| async {c.year = 2022}.boxed()).await;
    /// let year = atomic_car.lock_mut_async(|mut c| async {c.year = 2023; c.year}.boxed()).await;
    /// })
    /// ```
    // design background: https://stackoverflow.com/a/77657788/10087197
    pub async fn lock_mut_async<R>(&self, f: impl FnOnce(&mut T) -> BoxFuture<'_, R>) -> R {
        let inner_guard = self.inner.write().await;
        let mut guard = AtomicRwWriteGuard::new(inner_guard, &self.lock_callback_info);
        f(&mut guard).await
    }
}

/// A wrapper for [RwLockReadGuard](tokio::sync::RwLockReadGuard) that
/// can optionally call a callback to notify when the
/// lock is acquired or released.
pub struct AtomicRwReadGuard<'a, T> {
    guard: RwLockReadGuard<'a, T>,
    lock_callback_info: &'a LockCallbackInfo,
}

impl<'a, T> AtomicRwReadGuard<'a, T> {
    fn new(guard: RwLockReadGuard<'a, T>, lock_callback_info: &'a LockCallbackInfo) -> Self {
        if let Some(cb) = lock_callback_info.lock_callback_fn {
            cb(LockEvent::Acquire {
                info: lock_callback_info.lock_info_owned.as_lock_info(),
                acquired: LockAcquisition::Read,
            });
        }
        Self {
            guard,
            lock_callback_info,
        }
    }
}

impl<'a, T> Drop for AtomicRwReadGuard<'a, T> {
    fn drop(&mut self) {
        let lock_callback_info = self.lock_callback_info;
        if let Some(cb) = lock_callback_info.lock_callback_fn {
            cb(LockEvent::Release {
                info: lock_callback_info.lock_info_owned.as_lock_info(),
                acquired: LockAcquisition::Read,
            });
        }
    }
}

impl<'a, T> Deref for AtomicRwReadGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &(*self.guard)
    }
}

/// A wrapper for [RwLockWriteGuard](tokio::sync::RwLockWriteGuard) that
/// can optionally call a callback to notify when the
/// lock is acquired or released.
pub struct AtomicRwWriteGuard<'a, T> {
    guard: RwLockWriteGuard<'a, T>,
    lock_callback_info: &'a LockCallbackInfo,
}

impl<'a, T> AtomicRwWriteGuard<'a, T> {
    fn new(guard: RwLockWriteGuard<'a, T>, lock_callback_info: &'a LockCallbackInfo) -> Self {
        if let Some(cb) = lock_callback_info.lock_callback_fn {
            cb(LockEvent::Acquire {
                info: lock_callback_info.lock_info_owned.as_lock_info(),
                acquired: LockAcquisition::Write,
            });
        }
        Self {
            guard,
            lock_callback_info,
        }
    }
}

impl<'a, T> Drop for AtomicRwWriteGuard<'a, T> {
    fn drop(&mut self) {
        let lock_callback_info = self.lock_callback_info;
        if let Some(cb) = lock_callback_info.lock_callback_fn {
            cb(LockEvent::Release {
                info: lock_callback_info.lock_info_owned.as_lock_info(),
                acquired: LockAcquisition::Write,
            });
        }
    }
}

impl<'a, T> Deref for AtomicRwWriteGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &*self.guard
    }
}

impl<'a, T> DerefMut for AtomicRwWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.guard
    }
}

/*
note: commenting until async-traits are supported in stable rust.
      It is supposed to be available in 1.75.0 on Dec 28, 2023.
      See: https://releases.rs/docs/1.75.0/
impl<T> Atomic<T> for AtomicRw<T> {
    #[inline]
    async fn lock<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        AtomicRw::<T>:.lock(self, f).await
    }

    #[inline]
    async fn lock_mut<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        AtomicRw::<T>:.lock_mut(self, f).await
    }
}
*/

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::FutureExt;

    #[tokio::test]
    // Verify (compile-time) that AtomicRw:.lock() and :.lock_mut() accept mutable values.  (FnMut)
    async fn mutable_assignment() {
        let name = "Jim".to_string();
        let atomic_name = AtomicRw::from(name);

        let mut new_name: String = Default::default();
        atomic_name.lock_mut(|n| *n = "Sally".to_string()).await;
        atomic_name.lock_mut(|n| new_name = n.to_string()).await;
    }

    #[tokio::test]
    async fn lock_async() {
        struct Car {
            year: u16,
        }

        let atomic_car = AtomicRw::from(Car { year: 2016 });

        // access data without returning anything from closure
        atomic_car
            .lock_async(|c| {
                async {
                    assert_eq!(c.year, 2016);
                }
                .boxed()
            })
            .await;

        // test return from closure.
        let year = atomic_car.lock_async(|c| async { c.year }.boxed()).await;
        assert_eq!(year, 2016);
    }
}
