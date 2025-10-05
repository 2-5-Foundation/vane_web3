import { hostCryptography } from "./cryptography";
import { hostNetworking } from "./networking";
import { hostLogging as _hostLogging } from "./logging";

// Re-export if you still want consumers to access the type directly
// (optional, not required for the fix)
// export type { LogEntry } from "./logging";

const DHT_URL = "http://[::1]:8787";

export const hostDHT = {
  async set(
    key: string,
    value: string
  ): Promise<{ success: boolean; message?: string; error?: string; random?: number }> {
    try {
      // First check if the key already exists
      const getRes = await fetch(`${DHT_URL}/get?key=${encodeURIComponent(key)}`, {
        method: "GET",
        headers: { "Content-Type": "application/json" },
      });
      
      if (getRes.ok) {
        const existingData = await getRes.json();
        if (existingData.success && existingData.value) {
          // Key already exists, no-op
          const randArr = new Uint32Array(1);
          crypto.getRandomValues(randArr);
          const random = randArr[0];
          return { success: true, message: "Key already exists, no-op", random };
        }
      }
      
      const res = await fetch(`${DHT_URL}/set`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ key, value }),
      });
      const data = await res.json();
      return data;
    } catch (error) {
      const randArr = new Uint32Array(1);
      crypto.getRandomValues(randArr);
      const random = randArr[0];
      return { success: false, error: `Network error: ${error}`, random };
    }
  },

  async get(
    key: string
  ): Promise<{ success: boolean; value?: string | null; error?: string; random?: number }> {
    // ensure Promise ALWAYS settles (timeout + robust parse)
    const randArr = new Uint32Array(1);
    crypto.getRandomValues(randArr);
    const random = randArr[0];

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(new Error("fetch timeout")), 12000);

    try {
      const res = await fetch(`${DHT_URL}/get?key=${encodeURIComponent(key)}`, {
        method: "GET",
        signal: controller.signal,
        headers: { Accept: "application/json" },
      });

      const text = await res.text(); // read as text first
      let data: any;
      try {
        data = JSON.parse(text);
      } catch {
        console.error("🔍 DHT GET: Bad JSON:", text);
        return { success: false, value: null, error: "Bad JSON from DHT server", random };
      }
      console.log("🔍 DHT GET: Response data:", data);

      // normalize shape so Rust can always deserialize
      return {
        success: !!data.success,
        value: data.value ?? null,
        error: data.error ?? (res.ok ? undefined : `HTTP ${res.status}`),
        random: typeof data.random === "number" ? data.random : random,
      };
    } catch (error: any) {
      console.error("🔍 DHT GET: Error:", error);
      const err = error?.name === "AbortError" ? "Network timeout" : `Network error: ${error}`;
      return { success: false, value: null, error: err, random };
    } finally {
      clearTimeout(timer);
    }
  },
};

/**
 * Public-facing wrapper type for logging that deliberately avoids mentioning `LogEntry`
 * so declaration emit doesn't need to name external types.
 */
export type PublicHostLogging = {
  log: (
    level: number,
    target: string,
    message: string,
    module_path?: string,
    file?: string,
    line?: number
  ) => void;
  setLogLevel: (level: number) => void;
  /** Kept for parity, but typed as unknown[] to avoid LogEntry in the public surface */
  getLogHistory: () => unknown[];
  clearHistory: () => void;
  exportLogs: () => string;
  /** Also keep callback but typed with unknown */
  setLogCallback: (callback: (entry: unknown) => void) => void;
  /** Optional passthrough if you need the singleton (typed as unknown to avoid leaking types) */
  getLogInstance: () => unknown;
  LogLevel: {
    Error: number;
    Warn: number;
    Info: number;
    Debug: number;
    Trace: number;
  };
};

/** Concrete wrapper that delegates to the real hostLogging, but hides LogEntry in types */
const hostLogging: PublicHostLogging = {
  log: _hostLogging.log,
  setLogLevel: _hostLogging.setLogLevel,
  getLogHistory: () => _hostLogging.getLogHistory() as unknown[],
  clearHistory: _hostLogging.clearHistory,
  exportLogs: _hostLogging.exportLogs,
  setLogCallback: (cb: (entry: unknown) => void) => _hostLogging.setLogCallback(cb as any),
  getLogInstance: () => _hostLogging.getLogInstance(),
  LogLevel: _hostLogging.LogLevel,
};

export type HostFunctions = {
  hostNetworking: typeof hostNetworking;
  hostCryptography: typeof hostCryptography;
  hostLogging: PublicHostLogging;
  hostDHT: typeof hostDHT;
};

// ✅ Explicit annotation prevents TS from inferring a type that mentions LogEntry
export const hostFunctions: HostFunctions = {
  hostNetworking,
  hostCryptography,
  hostLogging,
  hostDHT,
};

export default hostFunctions;
