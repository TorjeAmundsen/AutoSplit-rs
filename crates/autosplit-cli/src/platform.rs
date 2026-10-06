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

/// LiveSplit starts AutoSplit without hiding its console, so Windows opens an empty console
/// window that closes AutoSplit when closed. Detaches from it when nothing else uses it, which
/// leaves a console AutoSplit was started from alone.
pub fn leave_own_console() {
    #[cfg(windows)]
    if owns_console() {
        let _ = unsafe { windows::Win32::System::Console::FreeConsole() };
    }
}

/// Whether Windows opened a console just for this program, like when it's double-clicked:
/// the window closes as soon as it exits.
pub fn owns_console() -> bool {
    #[cfg(windows)]
    {
        let mut processes = [0_u32; 2];
        unsafe { windows::Win32::System::Console::GetConsoleProcessList(&mut processes) == 1 }
    }
    #[cfg(not(windows))]
    false
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
