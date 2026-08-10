import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { buildRelease } from "../src/deploy.js";

test("builds a verified release", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "deploy-public-"));
  const source = path.join(root, "source"); const output = path.join(root, "output");
  await fs.mkdir(source); await fs.writeFile(path.join(source, "index.txt"), "hello");
  const sha256 = crypto.createHash("sha256").update("hello").digest("hex");
  await buildRelease(source, output, { files: [{ source: "index.txt", destination: "index.html", sha256 }] });
  assert.equal(await fs.readFile(path.join(output, "index.html"), "utf8"), "hello");
});
