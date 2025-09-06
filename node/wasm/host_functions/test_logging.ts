/**
 * Simple test to verify WASM logging functionality
 * Run this in a browser console after loading your WASM module
 */

import { hostFunctions } from './main';

export function testWasmLogging() {
    console.log("🧪 Testing WASM Logging System...");
    
    try {
        // Test setting log level
        console.log("1. Testing log level setting...");
        hostFunctions.hostLogging.setLogLevel(hostFunctions.hostLogging.LogLevel.Debug);
        
        // Test getting log history (should be empty initially)
        console.log("2. Testing log history retrieval...");
        const initialHistory = hostFunctions.hostLogging.getLogHistory();
        const parsedHistory = JSON.parse(initialHistory);
        console.log(`   Initial log history length: ${parsedHistory.length}`);
        
        // Test clearing history
        console.log("3. Testing log history clearing...");
        hostFunctions.hostLogging.clearHistory();
        
        // Test exporting logs
        console.log("4. Testing log export...");
        const exportedLogs = hostFunctions.hostLogging.exportLogs();
        console.log(`   Exported logs length: ${exportedLogs.length} characters`);
        
        // Test manual logging (if you want to test from JS side)
        console.log("5. Testing manual log entry...");
        hostFunctions.hostLogging.log(
            hostFunctions.hostLogging.LogLevel.Info,
            "TestTarget",
            "This is a test log message from JavaScript",
            "test_module",
            "test_logging.ts",
            42,
            "NODE 1",
        );
        
        // Check if the log was recorded
        const newHistory = hostFunctions.hostLogging.getLogHistory();
        const newParsedHistory = JSON.parse(newHistory);
        console.log(`   New log history length: ${newParsedHistory.length}`);
        
        if (newParsedHistory.length > 0) {
            console.log("   Last log entry:", newParsedHistory[newParsedHistory.length - 1]);
        }
        
        console.log("✅ WASM Logging tests completed successfully!");
        return true;
        
    } catch (error) {
        console.error("❌ WASM Logging test failed:", error);
        return false;
    }
}

// Auto-run test if in browser and WASM is loaded
if (typeof window !== 'undefined') {
    // Wait a bit for WASM to load, then test
    setTimeout(() => {
        testWasmLogging();
    }, 1000);
}
