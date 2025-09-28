import { hostCryptography } from "./cryptography";
import { hostNetworking } from "./networking";
import { hostLogging } from "./logging";


const DHT_URL = "http://[::1]:8787";

export const hostDHT = {
  async set(key: string, value: string): Promise<{ success: boolean; message?: string; error?: string; random?: number }> {
    try {
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

  async get(key: string): Promise<{ success: boolean; value?: string | null; error?: string; random?: number }> {
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
        headers: { "Accept": "application/json" },
      });

      const text = await res.text();            // read as text first
      let data: any;
      try { data = JSON.parse(text); }
      catch {
        console.error("🔍 DHT GET: Bad JSON:", text);
        return { success: false, value: null, error: "Bad JSON from DHT server", random };
      }
      console.log("🔍 DHT GET: Response data:", data);

      // normalize shape so Rust can always deserialize
      return {
        success: !!data.success,
        value:  data.value ?? null,
        error:  data.error ?? (res.ok ? undefined : `HTTP ${res.status}`),
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


export const hostFunctions = {
    hostNetworking: hostNetworking,
    hostCryptography: hostCryptography,
    hostLogging: hostLogging,
    hostDHT: hostDHT,
}

export default hostFunctions;