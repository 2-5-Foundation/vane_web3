use log::{Level, Log, Metadata, Record};
use wasm_bindgen::prelude::*;
use std::sync::OnceLock;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["hostFunctions", "hostLogging"], js_name = "log")]
    fn js_log(
        level: u32,
        target: &str,
        message: &str,
        module_path: Option<&str>,
        file: Option<&str>,
        line: Option<u32>,
        identifier: Option<&str>,
    );

    #[wasm_bindgen(js_namespace = ["hostFunctions", "hostLogging"], js_name = "setLogLevel")]
    fn js_set_log_level(level: u32);
}

pub struct WasmLogger{
    identifier: Option<String>,
}

impl WasmLogger {
    /// Initialize the WASM logger with smart filtering
    pub fn init(identifier: Option<String>) -> Result<(), log::SetLoggerError> {
        static LOGGER: OnceLock<WasmLogger> = OnceLock::new();
        let logger_ref: &'static WasmLogger = LOGGER.get_or_init(|| WasmLogger{identifier});
        log::set_logger(logger_ref)?;
        // Set a reasonable default - Info level to reduce noise
        log::set_max_level(log::LevelFilter::Info);
        Ok(())
    }

    pub fn init_debug(identifier: Option<String>) -> Result<(), log::SetLoggerError> {
        static LOGGER: OnceLock<WasmLogger> = OnceLock::new();
        let logger_ref: &'static WasmLogger = LOGGER.get_or_init(|| WasmLogger{identifier});
        log::set_logger(logger_ref)?;
        log::set_max_level(log::LevelFilter::Debug);
        Ok(())
    }

    pub fn set_log_level(level: Level) {
        let level_num = match level {
            Level::Error => 1,
            Level::Warn => 2,
            Level::Info => 3,
            Level::Debug => 4,
            Level::Trace => 5,
        };
        unsafe {
            js_set_log_level(level_num);
        }
    }
}

impl Log for WasmLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let target = metadata.target();

        match metadata.level() {
            Level::Error => true, // Always show errors
            Level::Warn => {
                target.starts_with("wasm_node")
                    || target.starts_with("p2p")
                    || target.starts_with("vane")
                    || target.contains("connection")
                    || target.contains("failed")
            }
            Level::Info => {
                target.starts_with("wasm_node")
                    || target.starts_with("p2p")
                    || target.starts_with("vane")
                    || target.contains("connection")
                    || target.contains("established")
                    || target.contains("dial")
            }
            Level::Debug | Level::Trace => {
                target.starts_with("wasm_node")
                    || target.starts_with("p2p")
                    || target.starts_with("vane")
            }
        }
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level_num = match record.level() {
                Level::Error => 1,
                Level::Warn => 2,
                Level::Info => 3,
                Level::Debug => 4,
                Level::Trace => 5,
            };

            let target = record.target();
            let message = record.args().to_string();
            let module_path = record.module_path();
            let file = record.file();
            let line = record.line();
            let identifier = self.identifier.as_deref();
            unsafe {
                js_log(level_num, target, &message, module_path, file, line, identifier);
            }
        }
    }

    fn flush(&self) {}
}

#[macro_export]
macro_rules! wasm_log_error {
    ($($arg:tt)*) => {
        log::error!($($arg)*)
    };
}

#[macro_export]
macro_rules! wasm_log_warn {
    ($($arg:tt)*) => {
        log::warn!($($arg)*)
    };
}

#[macro_export]
macro_rules! wasm_log_info {
    ($($arg:tt)*) => {
        log::info!($($arg)*)
    };
}

#[macro_export]
macro_rules! wasm_log_debug {
    ($($arg:tt)*) => {
        log::debug!($($arg)*)
    };
}

#[macro_export]
macro_rules! wasm_log_trace {
    ($($arg:tt)*) => {
        log::trace!($($arg)*)
    };
}

pub fn init_wasm_logging(identifier: Option<String>) -> Result<(), log::SetLoggerError> {
    WasmLogger::init(identifier)
}

pub fn init_clean_logging(identifier: Option<String>) -> Result<(), log::SetLoggerError> {
    console_error_panic_hook::set_once();
    WasmLogger::init(identifier)
}

pub fn init_debug_logging(identifier: Option<String>) -> Result<(), log::SetLoggerError> {
    console_error_panic_hook::set_once();
    WasmLogger::init_debug(identifier)
}

pub fn set_wasm_log_level(level: Level) {
    WasmLogger::set_log_level(level);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logger_creation() {
        let logger = WasmLogger;
        assert!(logger.enabled(
            &log::Metadata::builder()
                .level(Level::Info)
                .target("test")
                .build()
        ));
    }
}
