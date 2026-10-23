import http from "node:http";
import { mkdirSync } from "node:fs";
import { applyProjection, caseView, createWorld, METERLESS_COMMIT, type Projection } from "./project.ts";

const port = Number(process.env.METERLESS_PORT ?? 8091);
const directory = process.env.WORLD_DIR ?? new URL("../data", import.meta.url).pathname;
mkdirSync(directory, { recursive: true });
const world = createWorld("local", directory);

function readBody(req: http.IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (size > 1_000_000) {
        reject(new Error("body too large"));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url ?? "/", "http://127.0.0.1");
  const send = (status: number, body: unknown) => {
    res.writeHead(status, { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" });
    res.end(JSON.stringify(body));
  };
  try {
    if (req.method === "GET" && url.pathname === "/health") {
      send(200, { ok: true, engine: "world-model", commit: METERLESS_COMMIT, events: world.events().length });
      return;
    }
    if (req.method === "POST" && url.pathname === "/project") {
      const body = JSON.parse(await readBody(req)) as Projection;
      const result = await applyProjection(world, body);
      send(200, result);
      return;
    }
    const match = url.pathname.match(/^\/project\/([a-z0-9-]+)$/);
    if (req.method === "GET" && match) {
      send(200, caseView(world, match[1]));
      return;
    }
    send(404, { error: "not found" });
  } catch (err) {
    send(400, { error: err instanceof Error ? err.message : "projection failed" });
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`extrawork world-model listening on 127.0.0.1:${port} commit ${METERLESS_COMMIT}`);
});
