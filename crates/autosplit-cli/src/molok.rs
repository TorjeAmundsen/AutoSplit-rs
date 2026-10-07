//! Sends Molok its POSIX signals. Molok only runs on Linux, so this is only built there.

/// Sends `SIGRTMIN + offset` to every running `molok` process.
pub fn signal(offset: i32) -> Result<(), String> {
    let signal = libc::SIGRTMIN() + offset;
    let mut found = false;
    for pid in molok_pids() {
        found = true;
        // SAFETY: kill only takes plain integers
        if unsafe { libc::kill(pid, signal) } != 0 {
            return Err(format!("Couldn't signal Molok: {}", std::io::Error::last_os_error()));
        }
    }
    if found { Ok(()) } else { Err("Molok isn't running".into()) }
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
