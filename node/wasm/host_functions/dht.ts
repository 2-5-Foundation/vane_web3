export default {
    async fetch(request: Request, env: any): Promise<Response> {
      console.log(`🔍 DHT Service: ${request.method} ${request.url}`);
      const url = new URL(request.url);
      const randArr = new Uint32Array(1);
      crypto.getRandomValues(randArr);
      const random = randArr[0];

      // Handle CORS preflight requests
      if (request.method === "OPTIONS") {
        return new Response(null, {
          status: 200,
          headers: {
            "Access-Control-Allow-Origin": "*",
            "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
            "Access-Control-Allow-Headers": "Content-Type",
          },
        });
      }
  
      if (url.pathname === "/set" && request.method === "POST") {
        try {
          const { key, value } = await request.json();
          if (!key || !value) {
            return new Response(JSON.stringify({ 
              success: false, 
              value: null,
              error: "Missing key or value",
              random: random
            }), { 
              status: 400,
              headers: { 
                "Content-Type": "application/json",
                "Access-Control-Allow-Origin": "*"
              }
            });
          }
          await env.KV.put(key, value);
          return new Response(JSON.stringify({ 
            success: true, 
            value: null,
            error: null,
            random: random
          }), {
            headers: { 
              "Content-Type": "application/json",
              "Access-Control-Allow-Origin": "*"
            }
          });
        } catch (error) {
          return new Response(JSON.stringify({ 
            success: false, 
            value: null,
            error: `Failed to store: ${error}`,
            random: random
          }), { 
            status: 500,
            headers: { 
              "Content-Type": "application/json",
              "Access-Control-Allow-Origin": "*"
            }
          });
        }
      }
  
      if (url.pathname === "/get" && request.method === "GET") {
        console.log("🔍 DHT Service: Handling GET request for key:", url.searchParams.get("key"));

        try {
          const key = url.searchParams.get("key");
          if (!key) {
            console.log("🔍 DHT Service: Missing key, returning 400");
            return new Response(JSON.stringify({ 
              success: false, 
              value: null,
              error: "Missing key",
              random: random
            }), { 
              status: 400,
              headers: { 
                "Content-Type": "application/json",
                "Access-Control-Allow-Origin": "*"
              }
            });
          }
          console.log("🔍 DHT Service: Looking up key in KV:", key);
          const value = await env.KV.get(key);
          console.log("🔍 DHT Service: KV lookup result:", value);
          return new Response(JSON.stringify({ 
            success: true, 
            value: value || null,
            error: null,
            random: random
          }), {
            headers: { 
              "Content-Type": "application/json",
              "Access-Control-Allow-Origin": "*"
            }
          });
        } catch (error) {
          console.log("🔍 DHT Service: Error during GET:", error);
          return new Response(JSON.stringify({ 
            success: false, 
            value: null,
            error: `Failed to retrieve: ${error}`,
            random: random
          }), { 
            status: 500,
            headers: { 
              "Content-Type": "application/json",
              "Access-Control-Allow-Origin": "*"
            }
          });
        }
      }
  
      return new Response(JSON.stringify({ 
        success: false, 
        value: null,
        error: "Not found",
        random: random
      }), { 
        status: 404,
        headers: { 
          "Content-Type": "application/json",
          "Access-Control-Allow-Origin": "*"
        }
      });
    }
  };