//! Verbosity handling and tracing initialization.

use std::io::IsTerminal;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

/// Maps a `-v` count to the maximum diagnostic level.
///
/// `0` shows errors and warnings only, `1` info, `2` debug, `3+` trace.
pub fn level_for(verbosity: u8) -> LevelFilter {
    match verbosity {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        2 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    }
}

/// Installs the global tracing subscriber.
///
/// All diagnostics go to stderr so stdout stays clean for data output.
/// `RUST_LOG` overrides the verbosity-derived filter; ANSI colors are
/// disabled when stderr is not a terminal.
pub fn init_tracing(verbosity: u8) {
    let filter = EnvFilter::builder()
        .with_default_directive(level_for(verbosity).into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

#[cfg(test)]
mod tests {
    use super::level_for;
    use tracing::level_filters::LevelFilter;

    #[test]
    fn verbosity_zero_maps_to_warnings() {
        assert_eq!(level_for(0), LevelFilter::WARN);
    }

    #[test]
    fn verbosity_one_maps_to_info() {
        assert_eq!(level_for(1), LevelFilter::INFO);
    }

    #[test]
    fn verbosity_two_maps_to_debug() {
        assert_eq!(level_for(2), LevelFilter::DEBUG);
    }

    #[test]
    fn verbosity_three_and_above_maps_to_trace() {
        assert_eq!(level_for(3), LevelFilter::TRACE);
        assert_eq!(level_for(4), LevelFilter::TRACE);
    }
}
