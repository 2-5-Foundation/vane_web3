# WASM Logging with Host Functions

This document explains how to use the logging system that exposes Rust's `log` crate functionality to JavaScript via host functions.

## Overview

The logging system consists of three main components:

1. **Rust logging module** (`src/logging.rs`) - Implements a custom logger that forwards logs to JavaScript
2. **JavaScript host functions** (`host_functions/logging.ts`) - Receives and processes logs from WASM
3. **Integration layer** - Connects everything together in your main WASM interface

## Features

- ✅ Full integration with Rust's `log` crate
- ✅ Log level filtering on both Rust and JavaScript sides
- ✅ Structured logging with metadata (file, line, module path)
- ✅ Log history storage and retrieval
- ✅ Export logs to text format
- ✅ Browser console output with proper log levels
- ✅ Global debugging interface for development

## Usage

### From Rust (WASM) Code

```rust
use log::{error, warn, info, debug, trace};

// Standard log crate macros work out of the box
info!("Starting transaction processing");
warn!("Low balance detected: {}", balance);
error!("Failed to connect to peer: {}", peer_id);

// With targets for better organization
info!(target: "MainServiceWorker", "Processing transaction: {}", tx_id);
error!(target: "P2P", "Connection failed: {}", error);

// Or use the convenience macros
use crate::logging::{wasm_log_info, wasm_log_error};
wasm_log_info!("This is an info message");
wasm_log_error!("This is an error message");
```

### From JavaScript

```typescript
import { hostFunctions } from './host_functions/main';

// Set log level
hostFunctions.hostLogging.setLogLevel(hostFunctions.hostLogging.LogLevel.Debug);

// Get log history
const logs = JSON.parse(hostFunctions.hostLogging.getLogHistory());

// Export logs as text
const logsText = hostFunctions.hostLogging.exportLogs();

// Clear history
hostFunctions.hostLogging.clearHistory();
```

### Development Debugging

When running in a browser, you can access the global logger:

```javascript
// Available in browser console
vaneLogger.setLogLevel(vaneLogger.LogLevel.Debug);
vaneLogger.getHistory();
vaneLogger.clear();
vaneLogger.export();
```

## Log Levels

| Level | Value | Description |
|-------|-------|-------------|
| Error | 1 | Error conditions |
| Warn  | 2 | Warning conditions |
| Info  | 3 | Informational messages |
| Debug | 4 | Debug messages |
| Trace | 5 | Trace messages |

## Configuration

### Setting Log Level from Rust

```rust
use crate::logging::{set_wasm_log_level};
use log::Level;

// Set to debug level
set_wasm_log_level(Level::Debug);
```

### Setting Log Level from JavaScript

```typescript
import { setVaneLogLevel } from './host_functions/example_usage';

// Set to debug level
setVaneLogLevel('debug');
```

## Integration with Your WASM Module

The logging system is automatically initialized when you call `start_vane_web3()`. No additional setup is required in most cases.

If you need to initialize logging manually:

```rust
use crate::logging::init_wasm_logging;

// Initialize early in your WASM module
if let Err(e) = init_wasm_logging() {
    // Handle initialization error
}
```

## Examples

### Basic Logging

```rust
// In your Rust WASM code
use log::{info, error};

pub fn process_transaction(tx: &Transaction) -> Result<(), Error> {
    info!(target: "TxProcessor", "Processing transaction {}", tx.id);
    
    match validate_transaction(tx) {
        Ok(_) => {
            info!(target: "TxProcessor", "Transaction {} validated successfully", tx.id);
            Ok(())
        }
        Err(e) => {
            error!(target: "TxProcessor", "Transaction {} validation failed: {}", tx.id, e);
            Err(e)
        }
    }
}
```

### JavaScript Log Monitoring

```typescript
import { startLogMonitoring, downloadVaneLogs } from './host_functions/example_usage';

// Monitor logs every 5 seconds
const monitoringInterval = startLogMonitoring(5000);

// Download logs for debugging
downloadVaneLogs();

// Stop monitoring
clearInterval(monitoringInterval);
```

### Error Handling and Debugging

```rust
use log::{error, debug};
use anyhow::Result;

pub async fn connect_to_peer(peer_id: &str) -> Result<()> {
    debug!(target: "P2P", "Attempting to connect to peer: {}", peer_id);
    
    match establish_connection(peer_id).await {
        Ok(connection) => {
            debug!(target: "P2P", "Successfully connected to peer: {}", peer_id);
            Ok(())
        }
        Err(e) => {
            error!(target: "P2P", "Failed to connect to peer {}: {}", peer_id, e);
            Err(e.into())
        }
    }
}
```

## Best Practices

1. **Use targets** to organize logs by component (e.g., "P2P", "TxProcessor", "DB")
2. **Set appropriate log levels** for different environments (Debug for development, Info for production)
3. **Include context** in log messages (transaction IDs, peer IDs, etc.)
4. **Monitor error logs** in production for debugging
5. **Export logs periodically** for analysis
6. **Clear log history** periodically to prevent memory issues

## Troubleshooting

### Logs not appearing in console

1. Check that logging is initialized: `init_wasm_logging()` should be called
2. Verify log level is set appropriately
3. Ensure host functions are properly imported

### Performance considerations

1. The log history is limited to 1000 entries by default
2. Consider setting higher log levels in production
3. Clear log history periodically if needed

### Memory usage

The logging system stores logs in memory. In long-running applications, consider:
- Setting appropriate log levels
- Clearing history periodically
- Exporting and clearing logs for analysis
