import { appendFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";

const port = Number(process.argv[2]);
const outputPath = process.argv[3];
if (!Number.isInteger(port) || !outputPath) {
  throw new Error("usage: node workflow-webhook-sink.mjs <port> <output-path>");
}

writeFileSync(outputPath, "", "utf8");
const attempts = new Map();
const server = createServer((request, response) => {
  if (request.method === "GET" && request.url === "/health") {
    response.writeHead(200, { "content-type": "application/json" });
    response.end('{"ok":true}');
    return;
  }
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    const body = Buffer.concat(chunks).toString("utf8");
    const idempotencyKey = request.headers["idempotency-key"] ?? "";
    const attempt = (attempts.get(idempotencyKey) ?? 0) + 1;
    attempts.set(idempotencyKey, attempt);
    appendFileSync(
      outputPath,
      `${JSON.stringify({
        method: request.method,
        url: request.url,
        authorization: request.headers.authorization ?? null,
        idempotencyKey,
        attempt,
        body: body ? JSON.parse(body) : null,
      })}\n`,
      "utf8",
    );
    const failOnce = request.url === "/fail-once" && attempt === 1;
    response.writeHead(failOnce ? 503 : 200, {
      "content-type": "application/json",
    });
    response.end(JSON.stringify({ ok: !failOnce, attempt }));
  });
});

server.listen(port, "127.0.0.1");

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
