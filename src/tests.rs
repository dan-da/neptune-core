pub mod shared;

macro_rules! shared_tokio_runtime {
    (
        $(#[$fn_meta:meta])*
        $vis:vis async fn $fn_name:ident() $(-> $ret:ty)? {
            $($tt:tt)*
        }
    ) => {
        $(#[$fn_meta])*
        #[test]
        fn $fn_name() $(-> $ret)? { // Propagate the return type to the #[test] fn
            static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
            let runtime = RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().unwrap());

            runtime.block_on(async {
                async fn __inner() $(-> $ret)? {
                    $($tt)*
                }
                __inner().await // Return the awaited result
            })
        }
    };
}

pub(crate) use shared_tokio_runtime;

