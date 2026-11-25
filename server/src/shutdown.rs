//! Graceful process termination using the existing Tokio signal runtime.
use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct ShutdownSignal(Arc<AtomicBool>);
impl ShutdownSignal {
    pub(crate) fn install() -> io::Result<Self> {
        let requested = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&requested);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        #[cfg(unix)]
        let (mut terminate, mut interrupt) = {
            let _entered = runtime.enter();
            use tokio::signal::unix::{SignalKind, signal};
            (
                signal(SignalKind::terminate())?,
                signal(SignalKind::interrupt())?,
            )
        };
        std::thread::Builder::new()
            .name("termination-signal".into())
            .spawn(move || {
                runtime.block_on(async move {
                    #[cfg(unix)]
                    tokio::select! { _ = terminate.recv() => {}, _ = interrupt.recv() => {} }
                    #[cfg(not(unix))]
                    let _ = tokio::signal::ctrl_c().await;
                    flag.store(true, Ordering::Release);
                });
            })?;
        Ok(Self(requested))
    }
    pub(crate) fn requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
