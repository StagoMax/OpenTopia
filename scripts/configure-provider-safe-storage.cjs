const { app, safeStorage } = require("electron");
const fs = require("node:fs");
const path = require("node:path");

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (!value.startsWith("--")) continue;
    const key = value.slice(2);
    args[key] = argv[index + 1];
    index += 1;
  }
  return args;
}

function readDotEnv(filePath) {
  const values = {};
  for (const rawLine of fs.readFileSync(filePath, "utf8").split(/\r?\n/)) {
    let line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("export ")) line = line.slice(7).trim();
    const separator = line.indexOf("=");
    if (separator <= 0) continue;
    const key = line.slice(0, separator).trim();
    let value = line.slice(separator + 1).trim();
    if (
      value.length >= 2 &&
      ((value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'")))
    ) {
      value = value.slice(1, -1);
    }
    values[key] = value;
  }
  return values;
}

function readSecretStore(targetPath) {
  try {
    const parsed = JSON.parse(fs.readFileSync(targetPath, "utf8"));
    if (parsed?.version === 1 && parsed.secrets) return parsed;
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  return { version: 1, secrets: {} };
}

function selectedBackend() {
  try {
    return typeof safeStorage.getSelectedStorageBackend === "function"
      ? safeStorage.getSelectedStorageBackend()
      : null;
  } catch {
    return null;
  }
}

const args = parseArgs(process.argv.slice(2));
const envFile = path.resolve(args["env-file"] || "");
const profile = args.profile || "AUDIT_COPILOT_LLM";
const targetUserData = path.resolve(args["target-user-data"] || "");
const runtimeUserData = args["runtime-user-data"]
  ? path.resolve(args["runtime-user-data"])
  : null;

if (!envFile || !targetUserData) {
  throw new Error("--env-file and --target-user-data are required");
}
if (runtimeUserData) app.setPath("userData", runtimeUserData);

app.whenReady().then(() => {
  const values = readDotEnv(envFile);
  const apiKey = values[`${profile}_API_KEY`];
  const baseUrl = values[`${profile}_BASE_URL`];
  const model = values[`${profile}_MODEL`];
  if (!apiKey || !baseUrl || !model) {
    throw new Error("The selected provider profile is incomplete");
  }
  if (!safeStorage.isEncryptionAvailable()) {
    throw new Error("Electron safeStorage encryption is unavailable");
  }

  const secretsPath = path.join(targetUserData, "secrets.json");
  const store = readSecretStore(secretsPath);
  const encrypted = safeStorage.encryptString(apiKey);
  const decrypted = safeStorage.decryptString(encrypted);
  if (decrypted !== apiKey) {
    throw new Error("safeStorage round-trip verification failed");
  }

  store.version = 1;
  store.secrets ||= {};
  store.secrets["provider-api-key"] = {
    kind: "safeStorage",
    envTarget: "OPENTOPIA_API_KEY",
    encryptedHex: encrypted.toString("hex"),
    updatedAt: new Date().toISOString(),
  };
  fs.mkdirSync(targetUserData, { recursive: true });
  fs.writeFileSync(secretsPath, `${JSON.stringify(store, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o600,
  });
  try {
    fs.chmodSync(secretsPath, 0o600);
  } catch {
    // Windows access is additionally protected by DPAPI and the user profile ACL.
  }

  const output = {
    configured: true,
    profile,
    baseUrl,
    model,
    storagePath: secretsPath,
    storageBackend: selectedBackend(),
    encryptionAvailable: true,
    providerApiKeyConfigured: true,
    secretValueExposed: false,
  };
  const json = JSON.stringify(output, null, 2);
  if (json.includes(apiKey)) {
    throw new Error("Secret audit rejected safeStorage metadata output");
  }
  process.stdout.write(`${json}\n`);
  app.quit();
});

process.on("uncaughtException", (error) => {
  process.stderr.write(`${error.message}\n`);
  app.exit(1);
});
