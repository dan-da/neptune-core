use futures::future::BoxFuture;
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};

type AcquiredCallbackFn = fn(is_mut: bool, name: Option<&str>);

/// An `Arc<Mutex<T>>` wrapper to make data thread-safe and easy to work with.
///
/// # Examples
/// ```
/// # use neptune_core::util_types::sync::tokio::AtomicMutex;
/// struct Car {
///     year: u16,
/// };
/// # tokio_test::block_on(async {
/// let atomic_car = AtomicMutex::from(Car{year: 2016});
/// atomic_car.lock(|c| {println!("year: {}", c.year)}).await;
/// atomic_car.lock_mut(|mut c| {c.year = 2023}).await;
/// # })
/// ```
///
/// It is also possible to provide a name and callback fn
/// during instantiation.  In this way, the application
/// can easily trace lock acquisitions.
///
/// # Examples
/// ```
/// # use neptune_core::util_types::sync::tokio::AtomicMutex;
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
/// let atomic_car = AtomicMutex::<Car>::from((Car{year: 2016}, Some("car"), Some(LOG_LOCK_ACQUIRED_CB)));
/// atomic_car.lock(|c| {println!("year: {}", c.year)}).await;
/// atomic_car.lock_mut(|mut c| {c.year = 2023}).await;
/// # })
/// ```
///
/// results in:
#[derive(Debug, Default)]
pub struct AtomicMutex<T> {
    inner: Arc<Mutex<T>>,
    name: Option<String>,
    acquired_callback: Option<AcquiredCallbackFn>,
}
impl<T> From<T> for AtomicMutex<T> {
    #[inline]
    fn from(t: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(t)),
            name: None,
            acquired_callback: None,
        }
    }
}
impl<T> From<(T, Option<String>, Option<AcquiredCallbackFn>)> for AtomicMutex<T> {
    /// Create from an optional name and an optional callback function, which
    /// is called when a lock is acquired.
    #[inline]
    fn from(v: (T, Option<String>, Option<AcquiredCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(Mutex::new(v.0)),
            name: v.1,
            acquired_callback: v.2,
        }
    }
}
impl<T> From<(T, Option<&str>, Option<AcquiredCallbackFn>)> for AtomicMutex<T> {
    /// Create from a name ref and an optional callback function, which
    /// is called when a lock is acquired.
    #[inline]
    fn from(v: (T, Option<&str>, Option<AcquiredCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(Mutex::new(v.0)),
            name: v.1.map(|s| s.to_owned()),
            acquired_callback: v.2,
        }
    }
}

impl<T> Clone for AtomicMutex<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
            acquired_callback: None,
        }
    }
}

impl<T> From<Mutex<T>> for AtomicMutex<T> {
    #[inline]
    fn from(t: Mutex<T>) -> Self {
        Self {
            name: None,
            inner: Arc::new(t),
            acquired_callback: None,
        }
    }
}
impl<T> From<(Mutex<T>, Option<String>, Option<AcquiredCallbackFn>)> for AtomicMutex<T> {
    /// Create from an Mutex<T> plus an optional name
    /// and an optional callback function, which is called
    /// when a lock is acquired.
    #[inline]
    fn from(v: (Mutex<T>, Option<String>, Option<AcquiredCallbackFn>)) -> Self {
        Self {
            inner: Arc::new(v.0),
            name: v.1,
            acquired_callback: v.2,
        }
    }
}

impl<T> TryFrom<AtomicMutex<T>> for Mutex<T> {
    type Error = Arc<Mutex<T>>;
    fn try_from(t: AtomicMutex<T>) -> Result<Mutex<T>, Self::Error> {
        Arc::<Mutex<T>>::try_unwrap(t.inner)
    }
}

impl<T> From<Arc<Mutex<T>>> for AtomicMutex<T> {
    #[inline]
    fn from(t: Arc<Mutex<T>>) -> Self {
        Self {
            name: None,
            inner: t,
            acquired_callback: None,
        }
    }
}
impl<T> From<(Arc<Mutex<T>>, Option<String>, Option<AcquiredCallbackFn>)> for AtomicMutex<T> {
    /// Create from an `Arc<Mutex<T>>` plus an optional name and
    /// an optional callback function, which is called when a lock
    /// is acquired.
    #[inline]
    fn from(v: (Arc<Mutex<T>>, Option<String>, Option<AcquiredCallbackFn>)) -> Self {
        Self {
            inner: v.0,
            name: v.1,
            acquired_callback: v.2,
        }
    }
}

impl<T> From<AtomicMutex<T>> for Arc<Mutex<T>> {
    #[inline]
    fn from(t: AtomicMutex<T>) -> Self {
        t.inner
    }
}

// note: we impl the Atomic trait methods here also so they
// can be used without caller having to use the trait.
impl<T> AtomicMutex<T> {
    /// Acquire read lock and return an `RwLockReadGuard`
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicMutex;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicMutex::from(Car{year: 2016});
    /// let year = atomic_car.lock_guard().await.year;
    /// # })
    /// ```
    pub async fn lock_guard(&self) -> MutexGuard<T> {
        let guard = self.inner.lock().await;
        if let Some(cb) = self.acquired_callback {
            cb(false, self.name.as_deref());
        }
        guard
    }

    /// Acquire write lock and return an `RwLockWriteGuard`
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicMutex;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicMutex::from(Car{year: 2016});
    /// atomic_car.lock_guard_mut().await.year = 2022;
    /// # })
    /// ```
    pub async fn lock_guard_mut(&self) -> MutexGuard<T> {
        let guard = self.inner.lock().await;
        if let Some(cb) = self.acquired_callback {
            cb(true, self.name.as_deref());
        }
        guard
    }

    /// Immutably access the data of type `T` in a closure and possibly return a result of type `R`
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicMutex;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicMutex::from(Car{year: 2016});
    /// atomic_car.lock(|c| println!("year: {}", c.year)).await;
    /// let year = atomic_car.lock(|c| c.year).await;
    /// })
    /// ```
    pub async fn lock<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let lock = self.inner.lock().await;
        if let Some(cb) = self.acquired_callback {
            cb(false, self.name.as_deref());
        }
        f(&lock)
    }

    /// Mutably access the data of type `T` in a closure and possibly return a result of type `R`
    ///
    /// # Examples
    /// ```
    /// # use neptune_core::util_types::sync::tokio::AtomicMutex;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicMutex::from(Car{year: 2016});
    /// atomic_car.lock_mut(|mut c| c.year = 2022).await;
    /// let year = atomic_car.lock_mut(|mut c| {c.year = 2023; c.year}).await;
    /// })
    /// ```
    pub async fn lock_mut<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut lock = self.inner.lock().await;
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
    /// # use neptune_core::util_types::sync::tokio::AtomicMutex;
    /// # use futures::future::FutureExt;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicMutex::from(Car{year: 2016});
    /// atomic_car.lock_async(|c| async {println!("year: {}", c.year)}.boxed()).await;
    /// let year = atomic_car.lock_async(|c| async {c.year}.boxed()).await;
    /// })
    /// ```
    // design background: https://stackoverflow.com/a/77657788/10087197
    pub async fn lock_async<R>(&self, f: impl FnOnce(&T) -> BoxFuture<'_, R>) -> R {
        let lock = self.inner.lock().await;
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
    /// # use neptune_core::util_types::sync::tokio::AtomicMutex;
    /// # use futures::future::FutureExt;
    /// struct Car {
    ///     year: u16,
    /// };
    /// # tokio_test::block_on(async {
    /// let atomic_car = AtomicMutex::from(Car{year: 2016});
    /// atomic_car.lock_mut_async(|mut c| async {c.year = 2022}.boxed()).await;
    /// let year = atomic_car.lock_mut_async(|mut c| async {c.year = 2023; c.year}.boxed()).await;
    /// })
    /// ```
    // design background: https://stackoverflow.com/a/77657788/10087197
    pub async fn lock_mut_async<R>(&self, f: impl FnOnce(&mut T) -> BoxFuture<'_, R>) -> R {
        let mut lock = self.inner.lock().await;
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
impl<T> Atomic<T> for AtomicMutex<T> {
    #[inline]
    async fn lock<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        AtomicMutex::<T>:.lock(self, f).await
    }

    #[inline]
    async fn lock_mut<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        AtomicMutex::<T>:.lock_mut(self, f).await
    }
}
*/

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::FutureExt;

    #[tokio::test]
    // Verify (compile-time) that AtomicMutex:.lock() and :.lock_mut() accept mutable values.  (FnMut)
    async fn mutable_assignment() {
        let name = "Jim".to_string();
        let atomic_name = AtomicMutex::from(name);

        let mut new_name: String = Default::default();
        atomic_name.lock_mut(|n| *n = "Sally".to_string()).await;
        atomic_name.lock_mut(|n| new_name = n.to_string()).await;
    }

    #[tokio::test]
    async fn lock_async() {
        struct Car {
            year: u16,
        }

        let atomic_car = AtomicMutex::from(Car { year: 2016 });

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
