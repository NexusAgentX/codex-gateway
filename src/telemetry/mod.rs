use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init(default_filter: &str) -> Result<(), tracing_subscriber::util::TryInitError> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_filter))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .try_init()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_has_a_fallible_contract() {
        let _init: fn(&str) -> Result<(), tracing_subscriber::util::TryInitError> = init;
    }
}
