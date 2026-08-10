import crypto from "node:crypto";

export function hashInputs(inputs) {
  return crypto.createHash("sha256").update(JSON.stringify(inputs)).digest("hex");
}
