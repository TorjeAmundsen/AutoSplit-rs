//! Sends Molok its POSIX signals. Molok only runs on Linux, so this is only built there.

/// Sends `SIGRTMIN + offset` to every running `molok` process. One it can't signal, like another
/// user's or one that just quit, doesn't keep the others from getting it, so it only fails when
/// none did.
pub fn signal(offset: i32) -> Result<(), String> {
    let signal = libc::SIGRTMIN() + offset;
    let mut signaled = false;
    let mut error = None;
    for pid in molok_pids() {
        // SAFETY: kill only takes plain integers
        if unsafe { libc::kill(pid, signal) } == 0 {
            signaled = true;
        } else {
            let last = std::io::Error::last_os_error();
            // Quit since it was listed
            if last.raw_os_error() != Some(libc::ESRCH) {
                error.get_or_insert(last);
            }
        }
    }
    match error {
        _ if signaled => Ok(()),
        Some(error) => Err(format!("Couldn't signal Molok: {error}")),
        None => Err("Molok isn't running".into()),
    }
}

/// The processes named `molok`, from `/proc/<pid>/comm`.
fn molok_pids() -> Vec<libc::pid_t> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .filter(|pid: &libc::pid_t| {
            std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|name| name.trim_end() == "molok")
        })
        .collect()
}
