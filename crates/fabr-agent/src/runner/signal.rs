use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::sync::atomic::Ordering;

static STOP: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
unsafe extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
}

#[cfg(unix)]
extern "C" fn on_signal(_: i32) {
    STOP.store(true, Ordering::SeqCst);
}

pub fn install() -> &'static AtomicBool {
    #[cfg(unix)]
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        for sig in [1, 2, 15] {
            signal(sig, on_signal as *const () as usize);
        }
    }
    &STOP
}

pub fn stopping() -> bool {
    STOP.load(std::sync::atomic::Ordering::SeqCst)
}
