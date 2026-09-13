//! ORES OpenTelemetry-compatible service lifecycle logging.
//!
//! Transfer identifiers, pairing material, capabilities, tickets, filenames,
//! request URLs, remote addresses, and file bytes are prohibited here.
//!
//! Builders returned here carry only a constant event name. Each call site
//! attaches its own static `ores-trace-` literal and the enclosing function's
//! `ores-routine-` constant before calling `send()`.

use std::sync::Arc;

use next_loggers::{
    json, Event, JsonObject, Logger, LoggerError, OpenTelemetryTransport, Options, Value,
};

pub fn logger() -> Logger {
    let transport = Arc::new(OpenTelemetryTransport::new(|record| {
        let encoded = serde_json::to_string(&record)
            .map_err(|error| LoggerError(format!("cannot encode OTEL log record: {error}")))?;
        eprintln!("{encoded}");
        Ok(())
    }));
    let mut options = Options::default().with_transport(transport);
    options.app_name = "ftnl-backend-api".into();
    options.name = Some("backend".into());
    options.console = false;
    Logger::new(options)
}

/// Build an informational, metadata-free lifecycle event.
#[must_use]
pub fn event(logger: &Logger, name: &'static str) -> Event {
    metadata_free(logger.info(vec![Value::String(name.into())]), name)
}

/// Build a warning, metadata-free lifecycle failure event.
///
/// The underlying error is intentionally not attached: socket and server
/// errors can embed addresses.
#[must_use]
pub fn failure(logger: &Logger, name: &'static str) -> Event {
    metadata_free(logger.warn(vec![Value::String(name.into())]), name)
}

fn metadata_free(event: Event, name: &'static str) -> Event {
    event
        .add_fields(JsonObject::from_iter([
            ("event.name".into(), json!(name)),
            ("data.classification".into(), json!("metadata-free")),
        ]))
        .add_tags(["file-tunnel", "backend"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logger_accepts_a_constant_metadata_free_event() {
        let logger = logger();
        let _ = event(&logger, "backend.test").send();
        let _ = failure(&logger, "backend.test.failed").send();
        logger.close().unwrap();
    }
}
