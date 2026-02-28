use std::future::Future;
use std::sync::OnceLock;

pub struct AiBridge;

impl AiBridge {
    fn runtime() -> &'static tokio::runtime::Runtime {
        static TOKIO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        TOKIO_RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("openmango-ai")
                .build()
                .expect("failed to initialize ai tokio runtime")
        })
    }

    pub fn block_on<F>(future: F) -> F::Output
    where
        F: Future,
    {
        Self::runtime().block_on(future)
    }

    pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        Self::runtime().spawn(future)
    }
}
