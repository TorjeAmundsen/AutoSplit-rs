//! AutoSplit.com on Windows: a console program that runs AutoSplit.exe, a GUI program, inside
//! the terminal and waits for it. Terminals pick AutoSplit.com over AutoSplit.exe when you type
//! `autosplit`, so they show its output in order, wait for it, and pass Ctrl+C to it. Without
//! arguments it only opens the window, so it returns to the prompt at once.

use std::process::{Command, Stdio};

/// Lets Ctrl+C through to AutoSplit.exe, which shares the console, while this one keeps waiting
/// for it. A handler isn't inherited, unlike ignoring Ctrl+C with no handler.
#[cfg(windows)]
unsafe extern "system" fn keep_waiting(_event: u32) -> windows::core::BOOL {
    true.into()
}

fn main() {
    let Ok(launcher) = std::env::current_exe() else { std::process::exit(1) };
    let program = launcher.with_file_name(if cfg!(windows) { "AutoSplit.exe" } else { "autosplit" });
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let mut command = Command::new(&program);
    command.args(&arguments);
    if arguments.is_empty() {
        let started = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        if let Err(error) = started {
            eprintln!("error: couldn't start {}: {error}", program.display());
            std::process::exit(1)
        }
        return;
    }
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(Some(keep_waiting), true);
    }
    match command.status() {
        // Windows exit codes don't fit in ExitCode, like 0xC000013A after Ctrl+C
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("error: couldn't start {}: {error}", program.display());
            std::process::exit(1)
        }
    }
}
