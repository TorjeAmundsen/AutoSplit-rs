//! autosplit.com on Windows: a console program that runs autosplit.exe, a GUI program, inside
//! the terminal and waits for it. Terminals pick autosplit.com over autosplit.exe when you type
//! `autosplit`, so they show its output in order, wait for it, and pass Ctrl+C to it.

use std::process::Command;

fn main() {
    let Ok(launcher) = std::env::current_exe() else { std::process::exit(1) };
    let program = launcher.with_file_name(if cfg!(windows) { "autosplit.exe" } else { "autosplit" });
    // autosplit.exe attaches to this console, so Ctrl+C reaches it directly. This one waits for
    // it to finish instead of being stopped first.
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(None, true);
    }
    match Command::new(&program).args(std::env::args_os().skip(1)).status() {
        // Windows exit codes don't fit in ExitCode, like 0xC000013A after Ctrl+C
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("error: couldn't start {}: {error}", program.display());
            std::process::exit(1)
        }
    }
}
