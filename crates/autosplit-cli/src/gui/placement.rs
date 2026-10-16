//! Where the settings, log, about and message windows open: over the main window, on its screen.
//! The Live Tester opens beside it, so the capture region stays in view.

/// A rectangle on the desktop, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The top left corner for a window of `width` by `height`: centered over `main`, then moved
/// inside `screen` if it would stick out of it. A window bigger than the screen keeps its top left
/// corner on the screen, so its title bar stays reachable.
pub fn centered_over(main: Area, width: u32, height: u32, screen: Option<Area>) -> (i32, i32) {
    let mut x = main.x + (main.width as i32 - width as i32) / 2;
    let mut y = main.y + (main.height as i32 - height as i32) / 2;
    if let Some(screen) = screen {
        x = x.min(screen.x + screen.width as i32 - width as i32).max(screen.x);
        y = y.min(screen.y + screen.height as i32 - height as i32).max(screen.y);
    }
    (x, y)
}

/// The top left corner for a window of `width` by `height` beside `main`: on its right, or on its
/// left when that doesn't fit on `screen`, or [`centered_over`] it when neither does.
pub fn beside(main: Area, width: u32, height: u32, screen: Option<Area>) -> (i32, i32) {
    let Some(screen) = screen else { return (main.x + main.width as i32, main.y) };
    let y = main.y.min(screen.y + screen.height as i32 - height as i32).max(screen.y);
    let right = main.x + main.width as i32;
    if right + width as i32 <= screen.x + screen.width as i32 {
        return (right, y);
    }
    let left = main.x - width as i32;
    if left >= screen.x {
        return (left, y);
    }
    centered_over(main, width, height, Some(screen))
}
