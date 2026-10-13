//! The license and third-party notices built into the program.

#[test]
fn licenses_hold_both_files() {
    // Act
    let text = autosplit::licenses::text(false);

    // Assert
    assert!(text.starts_with("                    GNU GENERAL PUBLIC LICENSE"));
    assert!(text.contains("Third-party notices"));
}

#[test]
fn licenses_for_notepad_have_windows_line_endings() {
    // Arrange
    let text = autosplit::licenses::text(false);

    // Act
    let crlf = autosplit::licenses::text(true);

    // Assert
    assert!(!text.contains('\r'));
    assert_eq!(crlf.matches("\r\n").count(), text.matches('\n').count());
}
