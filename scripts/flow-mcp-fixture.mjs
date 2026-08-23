import readline from "node:readline";

const lines = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});
function respond(id, result) {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
}

function fail(id, code, message) {
  process.stdout.write(
    `${JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } })}\n`,
  );
}

lines.on("line", (line) => {
  let request;
  try {
    request = JSON.parse(line);
  } catch {
    return;
  }

  if (request.id === undefined || request.id === null) {
    return;
  }

  switch (request.method) {
    case "initialize":
      respond(request.id, {
        protocolVersion: request.params?.protocolVersion ?? "2024-11-05",
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: "opentopia-flow-mcp-fixture", version: "1.0.0" },
      });
      break;
    case "ping":
      respond(request.id, {});
      break;
    case "tools/list":
      respond(request.id, {
        tools: [
          {
            name: "lookup_customer",
            description: "Look up one deterministic fixture customer.",
            inputSchema: {
              type: "object",
              properties: { customerId: { type: "string", minLength: 1 } },
              required: ["customerId"],
              additionalProperties: false,
            },
            annotations: {
              title: "Lookup customer",
              readOnlyHint: true,
              destructiveHint: false,
              idempotentHint: true,
              openWorldHint: false,
            },
          },
        ],
      });
      break;
    case "tools/call": {
      if (request.params?.name !== "lookup_customer") {
        fail(request.id, -32602, "Unknown fixture tool");
        break;
      }
      const customerId = request.params?.arguments?.customerId;
      if (typeof customerId !== "string" || customerId.length === 0) {
        fail(request.id, -32602, "customerId is required");
        break;
      }
      const customer = {
        customerId,
        displayName: `Fixture Customer ${customerId}`,
        status: "active",
      };
      respond(request.id, {
        content: [{ type: "text", text: JSON.stringify(customer) }],
        structuredContent: customer,
        isError: false,
      });
      break;
    }
    default:
      fail(request.id, -32601, `Unsupported method: ${request.method}`);
  }
});
