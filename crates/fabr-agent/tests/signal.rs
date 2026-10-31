unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[test]
fn hangup_interrupt_and_terminate_all_stop_the_runner() {
    let stop = fabr_agent::runner::install_signals();
    assert!(!fabr_agent::runner::stopping());
    // SAFETY: sends SIGHUP to this test process, whose handler was installed above.
    unsafe {
        kill(std::process::id() as i32, 1);
    }
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !fabr_agent::runner::stopping() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(stop.load(std::sync::atomic::Ordering::SeqCst));
}
