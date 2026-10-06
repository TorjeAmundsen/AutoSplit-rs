//! The few places where the host needs the OS directly.

/// Decodes stdin bytes that aren't UTF-8. LiveSplit writes AutoSplit's stdin in the ANSI code
/// page, so a profile path with é or ø arrives that way on Windows.
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

/// When a console program that LiveSplit started (an older build, or autosplit.com) runs
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
/// late. The precision costs a little power, so it's only asked for while comparing.
#[derive(Default)]
pub struct PreciseTimer {
    #[cfg_attr(not(windows), allow(dead_code))]
    active: bool,
}

impl PreciseTimer {
    pub fn set(&mut self, precise: bool) {
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
        #[cfg(not(windows))]
        let _ = precise;
    }
}

impl Drop for PreciseTimer {
    fn drop(&mut self) {
        self.set(false);
    }
}

/// On Windows, where `autosplit.exe` is a GUI program, writes to the console of the terminal it
/// was started from: autosplit.com's, or the terminal's when started directly.
pub fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use std::os::windows::io::IntoRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE,
            STD_OUTPUT_HANDLE, SetStdHandle,
        };
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
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
    }
}

/// Shows an error in a message box on Windows, where a GUI program has no console to print it in.
pub fn show_error(message: &str) {
    #[cfg(windows)]
    {
        let _ = rfd::MessageDialog::new()
            .set_title("AutoSplit")
            .set_description(message)
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    #[cfg(not(windows))]
    let _ = message;
}
