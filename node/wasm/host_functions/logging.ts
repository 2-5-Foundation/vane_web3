    /**
     * Host functions for logging from WASM to JavaScript
     * This enables the Rust log crate to output logs through the browser console
     */

    export enum LogLevel {
        Error = 1,
        Warn = 2,
        Info = 3,
        Debug = 4,
        Trace = 5,
    }

    interface LogEntry {
        level: LogLevel;
        target: string;
        message: string;
        module_path?: string;
        file?: string;
        line?: number;
    }

    class Logger {
        private logHistory: LogEntry[] = [];
        private maxHistorySize: number = 1000;
        private logLevelFilter: LogLevel = LogLevel.Info;
        private logCallback: ((entry: LogEntry) => void) | null = null;
        
        constructor() {
            // Initialize with info level by default
            this.setLogLevel(LogLevel.Info);
        }

        setLogCallback(callback: (entry: LogEntry) => void) {
            this.logCallback = callback;
        }

        setLogLevel(level: LogLevel) {
            this.logLevelFilter = level;
        }

        private shouldLog(level: LogLevel): boolean {
            return level <= this.logLevelFilter;
        }

        private formatLogMessage(entry: LogEntry): string {
            
            // Extract just the filename from the full path for cleaner logs
            let fileInfo = '';
            if (entry.file && entry.line) {
                const filename = entry.file.split('/').pop() || entry.file;
                fileInfo = `📄 ${filename}:${entry.line}`;
            } else if (entry.module_path) {
                fileInfo = `📦 ${entry.module_path}`;
            }
            
            const locationStr = fileInfo ? ` [${fileInfo}]` : '';
            
            return  `${entry.target}${locationStr}: ${entry.message}`;
        }

        private addToHistory(entry: LogEntry) {
            this.logHistory.push(entry);
            if (this.logHistory.length > this.maxHistorySize) {
                this.logHistory.shift();
            }
        }

        log(
            level: LogLevel,
            target: string,
            message: string,
            module_path?: string,
            file?: string,
            line?: number,

        ) {
            if (!this.shouldLog(level)) {
                return;
            }

            const entry: LogEntry = {
                level,
                target,
                message,
                module_path,
                file,
                line,
            };

            this.addToHistory(entry);

            // Call the callback if set
            if (this.logCallback) {
                this.logCallback(entry);
            }

            let fileInfo = '';
            if (entry.file && entry.line) {
                const filename = entry.file.split('/').pop() || entry.file;
                fileInfo = `📄${filename}:${entry.line}`;
            }
            
            const cleanMessage = fileInfo 
                ? `${entry.target} [${fileInfo}] ${entry.message}`
                : `${entry.target} ${entry.message}`;

            switch (level) {
                case LogLevel.Error:
                    console.error(`🔴 ${cleanMessage}`);
                    break;
                case LogLevel.Warn:
                    console.warn(`🟡 ${cleanMessage}`);
                    break;
                case LogLevel.Info:
                    console.info(`🔵 ${cleanMessage}`);
                    break;
                case LogLevel.Debug:
                    console.debug(`🟢 ${cleanMessage}`);
                    break;
                case LogLevel.Trace:
                    console.debug(`⚪ ${cleanMessage}`);
                    break;
                default:
                    console.log(`❓ ${cleanMessage}`);
            }
        }

        getLogHistory(): LogEntry[] {
            return [...this.logHistory];
        }

        clearHistory() {
            this.logHistory = [];
            console.log("🧹 Log history cleared");
        }

        exportLogs(): string {
            return this.logHistory
                .map(entry => this.formatLogMessage(entry))
                .join('\n');
        }
    }

    // Global logger instance
    const logger = new Logger();

    // Host functions exposed to WASM
    export const hostLogging = {
        /**
         * Main logging function called from Rust
         */
        log(
            level: number,
            target: string,
            message: string,
            module_path?: string,
            file?: string,
            line?: number,

        ) {
            logger.log(level as LogLevel, target, message, module_path, file, line);
        },

        /**
         * Set the minimum log level to display
         */
        setLogLevel(level: number) {
            logger.setLogLevel(level as LogLevel);
        },

        /**
         * Get log history as JSON string
         */
        getLogHistory(): string {
            return JSON.stringify(logger.getLogHistory());
        },

        /**
         * Clear log history
         */
        clearHistory() {
            logger.clearHistory();
        },

        /**
         * Export logs as text
         */
        exportLogs(): string {
            return logger.exportLogs();
        },

        /**
         * Set a callback function to receive log entries
         */
        setLogCallback(callback: (entry: LogEntry) => void) {
            logger.setLogCallback(callback);
        },

        /**
         * Log level constants for JavaScript usage
         */
        LogLevel,
    };

    // For development: expose logger globally for debugging
    if (typeof window !== 'undefined') {
        (window as any).vaneLogger = {
            setLogLevel: (level: LogLevel) => logger.setLogLevel(level),
            getHistory: () => logger.getLogHistory(),
            clear: () => logger.clearHistory(),
            export: () => logger.exportLogs(),
            LogLevel,
        };
    }
