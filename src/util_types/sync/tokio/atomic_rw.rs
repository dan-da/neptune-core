use futures::future::BoxFuture;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

type AcquiredCallbackFn = fn(is_mut: bool, name: Option<&str>);

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
#[derive(Debug, Default)]
pub struct AtomicRw<T> {
    inner: Arc<RwLock<T>>,
    name: Option<String>,
    acquired_callback: Option<AcquiredCallbackFn>,
}
impl<T> From<T> for AtomicRw<T> {
    #[inline]
    fn from(t: T) -> Self {
        Self {
            inner: Arc::new(RwLock::new(t)),
            name: None,
            acquired_callback: None,
        }
    }
}
impl<T> From<(T, Option<String>, Option<AcquiredCallbackFn>)> for AtomicRw<T> {
    /// Create from an optional name and an optional callback function, which
    /// is called when a lock is acquired.
    #[inline]
    fn from(v: (T, Option<String>, Option<AcquiredCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(RwLock::new(v.0)),
            name: v.1,
            acquired_callback: v.2,
        }
    }
}
impl<T> From<(T, Option<&str>, Option<AcquiredCallbackFn>)> for AtomicRw<T> {
    /// Create from a name ref and an optional callback function, which
    /// is called when a lock is acquired.
    #[inline]
    fn from(v: (T, Option<&str>, Option<AcquiredCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(RwLock::new(v.0)),
            name: v.1.map(|s| s.to_owned()),
            acquired_callback: v.2,
        }
    }
}

impl<T> Clone for AtomicRw<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
            acquired_callback: None,
        }
    }
}

impl<T> From<RwLock<T>> for AtomicRw<T> {
    #[inline]
    fn from(t: RwLock<T>) -> Self {
        Self {
            name: None,
            inner: Arc::new(t),
            acquired_callback: None,
        }
    }
}
impl<T> From<(RwLock<T>, Option<String>, Option<AcquiredCallbackFn>)> for AtomicRw<T> {
    /// Create from an RwLock<T> plus an optional name
    /// and an optional callback function, which is called
    /// when a lock is acquired.
    #[inline]
    fn from(v: (RwLock<T>, Option<String>, Option<AcquiredCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(v.0),
            name: v.1,
            acquired_callback: v.2,
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
            name: None,
            inner: t,
            acquired_callback: None,
        }
    }
}
impl<T> From<(Arc<RwLock<T>>, Option<String>, Option<AcquiredCallbackFn>)> for AtomicRw<T> {
    /// Create from an `Arc<RwLock<T>>` plus an optional name and
    /// an optional callback function, which is called when a lock
    /// is acquired.
    #[inline]
    fn from(v: (Arc<RwLock<T>>, Option<String>, Option<AcquiredCallbackFn>)) -> Self {
        Self {
            inner: v.0,
            name: v.1,
            acquired_callback: v.2,
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
    pub async fn lock_guard(&self) -> RwLockReadGuard<T> {
        let guard = self.inner.read().await;
        if let Some(cb) = self.acquired_callback {
            cb(false, self.name.as_deref());
        }
        guard
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
    pub async fn lock_guard_mut(&self) -> RwLockWriteGuard<T> {
        let guard = self.inner.write().await;
        if let Some(cb) = self.acquired_callback {
            cb(true, self.name.as_deref());
        }
        guard
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
        let lock = self.inner.read().await;
        if let Some(cb) = self.acquired_callback {
            cb(false, self.name.as_deref());
        }
        f(&lock)
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
        let mut lock = self.inner.write().await;
        if let Some(cb) = self.acquired_callback {
            cb(true, self.name.as_deref());
        }
        f(&mut lock)
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
        let lock = self.inner.read().await;
        if let Some(cb) = self.acquired_callback {
            cb(false, self.name.as_deref());
        }
        f(&lock).await
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
        let mut lock = self.inner.write().await;
        if let Some(cb) = self.acquired_callback {
            cb(true, self.name.as_deref());
        }
        f(&mut lock).await
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
