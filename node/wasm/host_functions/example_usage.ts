/**
 * Example usage of WASM logging host functions
 * This demonstrates how to use the logging system from JavaScript
 */

import { hostFunctions } from './main';

// Example: Setting up logging when initializing your WASM module
export function setupVaneLogging() {
    console.log("🚀 Setting up Vane WASM logging...");
    
    // Set log level to Debug to see more detailed logs
    hostFunctions.hostLogging.setLogLevel(hostFunctions.hostLogging.LogLevel.Debug);
    
    // You can also access the global logger for debugging
    if (typeof window !== 'undefined' && (window as any).vaneLogger) {
        const logger = (window as any).vaneLogger;
        
        // Example: Set to Info level
        logger.setLogLevel(logger.LogLevel.Info);
        
        console.log("📊 Available logging methods:");
        console.log("- setLogLevel(level): Set minimum log level");
        console.log("- getHistory(): Get all log entries");
        console.log("- clear(): Clear log history");
        console.log("- export(): Export logs as text");
    }
}

// Example: Function to monitor and export logs
export function exportVaneLogs(): string {
    try {
        return hostFunctions.hostLogging.exportLogs();
    } catch (error) {
        console.error("Failed to export logs:", error);
        return "";
    }
}

// Example: Function to get recent logs as JSON
export function getRecentLogs(): any[] {
    try {
        const historyJson = hostFunctions.hostLogging.getLogHistory();
        return JSON.parse(historyJson);
    } catch (error) {
        console.error("Failed to get log history:", error);
        return [];
    }
}

// Example: Log level management
export function setVaneLogLevel(level: 'error' | 'warn' | 'info' | 'debug' | 'trace') {
    const LogLevel = hostFunctions.hostLogging.LogLevel;
    const levelMap = {
        error: LogLevel.Error,
        warn: LogLevel.Warn,
        info: LogLevel.Info,
        debug: LogLevel.Debug,
        trace: LogLevel.Trace,
    };
    
    const numericLevel = levelMap[level];
    if (numericLevel) {
        hostFunctions.hostLogging.setLogLevel(numericLevel);
        console.log(`🔧 Vane log level set to: ${level}`);
    } else {
        console.error(`❌ Invalid log level: ${level}`);
    }
}

// Example: Periodic log monitoring (useful for debugging)
export function startLogMonitoring(intervalMs: number = 10000) {
    console.log(`🔍 Starting log monitoring every ${intervalMs}ms`);
    
    return setInterval(() => {
        const logs = getRecentLogs();
        const errorLogs = logs.filter(log => log.level === 1); // Error level
        
        if (errorLogs.length > 0) {
            console.warn(`⚠️ Found ${errorLogs.length} error logs in recent history`);
        }
        
        console.log(`📈 Total logs in history: ${logs.length}`);
    }, intervalMs);
}

// Example: Save logs to file (browser download)
export function downloadVaneLogs() {
    try {
        const logsText = exportVaneLogs();
        const blob = new Blob([logsText], { type: 'text/plain' });
        const url = URL.createObjectURL(blob);
        
        const a = document.createElement('a');
        a.href = url;
        a.download = `vane-logs-${new Date().toISOString().split('T')[0]}.log`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        
        URL.revokeObjectURL(url);
        console.log("📥 Vane logs downloaded successfully");
    } catch (error) {
        console.error("Failed to download logs:", error);
    }
}
