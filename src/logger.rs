#[macro_export]
macro_rules! scrim_error {
    ($($arg:tt)*) => {
        eprintln!("[Scrim] Error: {}", format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! scrim_progress {
    ($($arg:tt)*) => {
        eprintln!("[Scrim] {}", format_args!($($arg)*));
    };
}
