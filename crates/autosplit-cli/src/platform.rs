//! The few places where the host needs the OS directly.

/// Decodes stdin bytes that aren't UTF-8. LiveSplit writes AutoSplit's stdin in the ANSI code
/// page, so a profile path with letters outside ASCII arrives that way on Windows.
pub fn decode_ansi(bytes: &[u8]) -> String {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::{CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
        let length = unsafe { MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None) };
        let mut wide = vec![0; length.max(0) as usize];
        let written = unsafe { MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut wide)) };
        wide.truncate(written.max(0) as usize);
        String::from_utf16_lossy(&wide)
    }
    #[cfg(not(windows))]
    String::from_utf8_lossy(bytes).into_owned()
}

/// When a console program that LiveSplit started (an older build, or AutoSplit.com) runs
/// AutoSplit, Windows opens an empty console window that closes AutoSplit when closed. Detaches
/// from it when nothing else uses it, which leaves a console AutoSplit was started from alone.
pub fn leave_own_console() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
        let mut processes = [0_u32; 2];
        if GetConsoleProcessList(&mut processes) == 1 {
            let _ = FreeConsole();
        }
    }
}

/// Makes waits precise to about a millisecond while frames are compared. Windows rounds waits
/// up to its timer tick (15.6 ms by default), which would make comparisons uneven and splits
/// late, and macOS's App Nap stretches them while the window is hidden. The precision costs a
/// little power, so it's only asked for while comparing.
///
/// On macOS it's also kept while a split waits out a delay or pause, when nothing is compared:
/// App Nap would otherwise stretch the wait until just before it ends, from which precision is
/// asked for, by seconds, and the split or the next comparison would come late.
#[derive(Default)]
pub struct PreciseTimer {
    #[cfg_attr(not(windows), allow(dead_code))]
    active: bool,
    #[cfg(target_os = "macos")]
    activity: Option<Retained<ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>,
}

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::ProtocolObject;

impl PreciseTimer {
    /// `precise` while comparing and just before the run's next wakeup, `waiting` while the run
    /// waits for one at all, like during a delay or pause.
    pub fn set(&mut self, precise: bool, waiting: bool) {
        #[cfg(windows)]
        if precise != self.active {
            use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
            unsafe {
                if precise {
                    timeBeginPeriod(1);
                } else {
                    timeEndPeriod(1);
                }
            }
            self.active = precise;
        }
        #[cfg(target_os = "macos")]
        let precise = precise || waiting;
        #[cfg(not(target_os = "macos"))]
        let _ = waiting;
        #[cfg(target_os = "macos")]
        if precise != self.activity.is_some() {
            use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
            let process = NSProcessInfo::processInfo();
            match self.activity.take() {
                Some(activity) => unsafe { process.endActivity(&activity) },
                None => {
                    let reason = NSString::from_str("Comparing frames for splits");
                    self.activity = Some(process.beginActivityWithOptions_reason(
                        NSActivityOptions::UserInitiatedAllowingIdleSystemSleep | NSActivityOptions::LatencyCritical,
                        &reason,
                    ));
                }
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = precise;
    }
}

impl Drop for PreciseTimer {
    fn drop(&mut self) {
        self.set(false, false);
    }
}

/// On Windows, where `AutoSplit.exe` is a GUI program, writes to the console of the terminal it
/// was started from: AutoSplit.com's, or the terminal's when started directly. Returns whether
/// there was one; elsewhere, whether stderr is a terminal.
pub fn attach_console() -> bool {
    #[cfg(windows)]
    unsafe {
        use std::os::windows::io::IntoRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE,
            STD_OUTPUT_HANDLE, SetConsoleCtrlHandler, SetStdHandle,
        };
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return false;
        }
        // Ignoring Ctrl+C is inherited, and a launcher may have asked for it for itself
        let _ = SetConsoleCtrlHandler(None, false);
        // Handles the terminal didn't redirect go to the console
        let handles: [(STD_HANDLE, &str, bool); 3] = [
            (STD_INPUT_HANDLE, "CONIN$", false),
            (STD_OUTPUT_HANDLE, "CONOUT$", true),
            (STD_ERROR_HANDLE, "CONOUT$", true),
        ];
        for (which, device, write) in handles {
            if GetStdHandle(which).is_ok_and(|handle| !handle.is_invalid() && !handle.0.is_null()) {
                continue;
            }
            if let Ok(file) = std::fs::OpenOptions::new().read(!write).write(write).open(device) {
                let _ = SetStdHandle(which, HANDLE(file.into_raw_handle()));
            }
        }
        true
    }
    #[cfg(not(windows))]
    {
        use std::io::IsTerminal;
        std::io::stderr().is_terminal()
    }
}

/// Shows an error in a message box, for when there's no terminal to print it in.
pub fn show_error(message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_title("Error")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

/// Opens a file or link with its default app, like AutoSplit's `open_file` (`os.startfile` on
/// Windows). Paths may have forward slashes, like folders picked in AutoSplit's window.
#[cfg(windows)]
pub fn open(target: impl AsRef<std::ffi::OsStr>) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{PCWSTR, w};

    let target = target.as_ref();
    let link = target.to_string_lossy().contains("://");
    let wide: Vec<u16> = target
        .encode_wide()
        .map(|unit| if !link && unit == u16::from(b'/') { u16::from(b'\\') } else { unit })
        .chain([0])
        .collect();
    // SAFETY: the strings are null-terminated and outlive the call
    let result = unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    // Values above 32 mean it worked
    if result.0 as usize <= 32 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Opens a file or link with its default app, like AutoSplit's `open_file`, without waiting
/// for the app.
#[cfg(not(windows))]
pub fn open(target: impl AsRef<std::ffi::OsStr>) -> std::io::Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let mut child = std::process::Command::new(opener)
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    // Reaped in the background, so it doesn't stay a zombie
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Whether AutoSplit is the app in front on macOS, where showing one of its windows with the
/// focus would bring it there. Elsewhere, where the system decides whether a window may take
/// the focus, always true.
pub fn app_in_front() -> bool {
    #[cfg(target_os = "macos")]
    if let Some(main_thread) = objc2::MainThreadMarker::new() {
        return objc2_app_kit::NSApplication::sharedApplication(main_thread).isActive();
    }
    true
}

/// Lets the menus that open from the menu bar follow the system's dark mode on Windows. The bar
/// itself already does, but a menu is light unless the program allows dark mode for itself,
/// which Windows only offers through uxtheme.dll's unnamed `SetPreferredAppMode`.
pub fn allow_dark_menus() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        use windows::core::{PCSTR, w};
        // Windows 10 1903 and later. In 1809 it was AllowDarkModeForApp(true), which 1 also means.
        const SET_PREFERRED_APP_MODE: usize = 135;
        const FLUSH_MENU_THEMES: usize = 136;
        const ALLOW_DARK: i32 = 1;
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        if let Some(set_preferred_app_mode) = GetProcAddress(uxtheme, PCSTR(SET_PREFERRED_APP_MODE as *const u8)) {
            let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 =
                std::mem::transmute(set_preferred_app_mode);
            set_preferred_app_mode(ALLOW_DARK);
        }
        if let Some(flush_menu_themes) = GetProcAddress(uxtheme, PCSTR(FLUSH_MENU_THEMES as *const u8)) {
            let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(flush_menu_themes);
            flush_menu_themes();
        }
    }
}

/// Keeps Windows from slowing the session down while the window is minimized or covered, which
/// it does to windowed programs but not to console ones: Windows 11 drops the precise timer
/// [`PreciseTimer`] asks for, and power throttling can move comparisons to slower cores.
pub fn keep_running_when_hidden() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling,
            SetProcessInformation,
        };
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION | PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            std::ptr::from_ref(&state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}
