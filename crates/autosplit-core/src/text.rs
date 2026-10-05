//! How numbers are shown in the UI.

/// Truncates to 3 decimals, so a similarity just under a threshold never shows as reaching it.
///
/// AutoSplit truncates to 2 decimals with `int(value * 100) / 100`, which float rounding
/// sometimes pushes one step too low (0.29 shows as 0.28). Truncating the shortest decimal form
/// of the value avoids that.
pub fn decimal(value: f64) -> String {
    let text = format!("{value}");
    if !value.is_finite() {
        return text;
    }
    let (integer, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let fraction: String = fraction.chars().chain(std::iter::repeat('0')).take(3).collect();
    let negative_zero = integer == "-0" && fraction == "000";
    format!("{}.{fraction}", if negative_zero { "0" } else { integer })
}

/// Pause and delay countdowns, with one decimal.
pub fn seconds_remaining_text(seconds: f64) -> String {
    let shown = format!("{seconds:.1}");
    let plural = if shown == "1.0" { "" } else { "s" };
    format!("{shown} second{plural} remaining")
}
