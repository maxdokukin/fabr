use std::process::Command;

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn prctl(option: i32, ...) -> i32;
}

pub const TERM: i32 = 15;
pub const KILL: i32 = 9;

fn as_pid(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|p| *p > 0)
}

pub fn signal_group(pgid: u32, sig: i32) {
    #[cfg(unix)]
    if let Some(p) = as_pid(pgid) {
        // SAFETY: kill(2) with a negative pid signals a process group; no memory is shared.
        unsafe {
            kill(-p, sig);
        }
    }
    #[cfg(not(unix))]
    let _ = (pgid, sig);
}

fn exists(target: i32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the target exists and may be signalled.
        let rc = unsafe { kill(target, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(1)
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        false
    }
}

pub fn alive(pid: u32) -> bool {
    as_pid(pid).is_some_and(exists)
}

pub fn group_alive(pgid: u32) -> bool {
    as_pid(pgid).is_some_and(|p| exists(-p))
}

pub fn own_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: the hook runs between fork and exec and only calls prctl(2), which is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                prctl(1, TERM as std::ffi::c_ulong);
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    let _ = cmd;
}
