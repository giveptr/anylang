#[cfg(debug_assertions)]
pub fn listening() {
    use std::io::{self, IsTerminal};
    use tracing::info;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::format::FmtSpan;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(concat!(env!("CARGO_CRATE_NAME"), "=info"))),
        )
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false)
        .with_ansi(io::stdout().is_terminal())
        .init();

    info!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
}

#[cfg(not(debug_assertions))]
pub fn listening() {}
