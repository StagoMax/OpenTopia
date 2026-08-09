const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { spawn, spawnSync } = require("node:child_process");

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (!value.startsWith("--")) continue;
    const key = value.slice(2);
    if (key === "skip-build") {
      args[key] = true;
      continue;
    }
    args[key] = argv[index + 1];
    index += 1;
  }
  return args;
}

function required(args, key) {
  const value = String(args[key] || "").trim();
  if (!value) throw new Error(`--${key} is required`);
  return value;
}

function unprotectWithDpapi(encrypted) {
  const script = [
    "Add-Type -AssemblyName System.Security",
    "$encoded = [Console]::In.ReadToEnd()",
    "$cipher = [Convert]::FromBase64String($encoded)",
    "$plain = [System.Security.Cryptography.ProtectedData]::Unprotect($cipher, $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)",
    "[Console]::Out.Write([Convert]::ToBase64String($plain))",
  ].join("; ");
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", script],
    {
      input: encrypted.toString("base64"),
      encoding: "utf8",
      windowsHide: true,
      maxBuffer: 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    throw new Error(
      `Windows DPAPI could not unlock the provider keyring: ${result.stderr.trim()}`,
    );
  }
  return Buffer.from(result.stdout.trim(), "base64");
}

function readSafeStorageMasterKey(userDataDir) {
  const localState = JSON.parse(
    fs.readFileSync(path.join(userDataDir, "Local State"), "utf8"),
  );
  const wrapped = Buffer.from(
    localState?.os_crypt?.encrypted_key || "",
    "base64",
  );
  if (
    wrapped.length <= 5 ||
    wrapped.subarray(0, 5).toString("ascii") !== "DPAPI"
  ) {
    throw new Error(
      "OpenTopia Local State does not contain a supported DPAPI key",
    );
  }
  return unprotectWithDpapi(wrapped.subarray(5));
}

function decryptSafeStorageValue(encryptedHex, masterKey) {
  const encrypted = Buffer.from(encryptedHex, "hex");
  if (
    encrypted.length <= 31 ||
    encrypted.subarray(0, 3).toString("ascii") !== "v10"
  ) {
    throw new Error(
      "Provider keyring entry does not use the supported safeStorage v10 format",
    );
  }
  const nonce = encrypted.subarray(3, 15);
  const authenticationTag = encrypted.subarray(encrypted.length - 16);
  const ciphertext = encrypted.subarray(15, encrypted.length - 16);
  const decipher = crypto.createDecipheriv("aes-256-gcm", masterKey, nonce);
  decipher.setAuthTag(authenticationTag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const repoRoot = path.resolve(required(args, "repo-root"));
  const userDataDir = path.resolve(required(args, "user-data"));
  const providerId = required(args, "provider-id");
  const profile = required(args, "profile");
  const baseUrl = required(args, "base-url").replace(/\/+$/, "");
  const model = required(args, "model");
  const envFile = path.resolve(required(args, "env-file"));
  const suitePath = path.resolve(required(args, "suite"));
  const targetPath = path.resolve(required(args, "target"));
  const reasoningEffort = String(args["reasoning-effort"] || "").trim();
  const repetitions = String(args.repetitions || "1");
  const port = String(args.port || "8812");

  if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(profile)) {
    throw new Error("--profile must be a valid environment prefix");
  }
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$/.test(providerId)) {
    throw new Error("--provider-id contains unsupported characters");
  }

  const store = JSON.parse(
    fs.readFileSync(path.join(userDataDir, "secrets.json"), "utf8"),
  );
  const entry = store?.secrets?.[`provider-api-key:${providerId}`];
  if (!entry?.encryptedHex) {
    throw new Error(
      `Provider ${providerId} is not configured in the OpenTopia keyring`,
    );
  }

  const masterKey = readSafeStorageMasterKey(userDataDir);
  const apiKeyBytes = decryptSafeStorageValue(entry.encryptedHex, masterKey);
  const apiKey = apiKeyBytes.toString("utf8");
  if (!apiKey.trim())
    throw new Error(`Provider ${providerId} decrypted to an empty credential`);

  const runner = path.join(
    repoRoot,
    "scripts",
    "evaluate-opentopia-tool-suite.ps1",
  );
  const runnerArgs = [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    runner,
    "-EnvFile",
    envFile,
    "-Profile",
    profile,
    "-ExpectedModel",
    model,
    "-Port",
    port,
    "-Repetitions",
    repetitions,
    "-SuitePath",
    suitePath,
    "-TargetPath",
    targetPath,
  ];
  if (reasoningEffort) runnerArgs.push("-ReasoningEffort", reasoningEffort);
  if (args["skip-build"]) runnerArgs.push("-SkipBuild");

  const environment = {
    ...process.env,
    [`${profile}_API_KEY`]: apiKey,
    [`${profile}_BASE_URL`]: baseUrl,
    [`${profile}_MODEL`]: model,
  };
  const child = spawn("powershell.exe", runnerArgs, {
    cwd: repoRoot,
    env: environment,
    stdio: "inherit",
    windowsHide: true,
  });
  const exitCode = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code) => resolve(code ?? 1));
  });

  masterKey.fill(0);
  apiKeyBytes.fill(0);
  environment[`${profile}_API_KEY`] = "";
  process.stdout.write(
    `${JSON.stringify(
      {
        completed: true,
        exitCode,
        providerId,
        profile,
        baseUrl,
        model,
        secretValueExposed: false,
      },
      null,
      2,
    )}\n`,
  );
  process.exitCode = exitCode;
}

main().catch((error) => {
  process.stderr.write(
    `${String(error?.message || error).replace(/Bearer\s+\S+/gi, "Bearer <redacted>")}\n`,
  );
  process.exitCode = 1;
});
