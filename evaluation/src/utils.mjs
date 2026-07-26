import { createHash, randomUUID } from "node:crypto";
import { cp, lstat, mkdir, readFile, readdir, stat } from "node:fs/promises";
import path from "node:path";

export function makeId(prefix) {
  const timestamp = new Date().toISOString().replace(/[-:.]/g, "").replace("Z", "Z");
  return `${prefix}_${timestamp}_${randomUUID().slice(0, 8)}`;
}

export function resolveInside(baseDirectory, candidate, label = "path", options = {}) {
  const base = path.resolve(baseDirectory);
  const resolved = path.resolve(base, candidate);
  const relative = path.relative(base, resolved);
  if ((relative === "" && options.allowBase !== false) || (!relative.startsWith("..") && !path.isAbsolute(relative))) return resolved;
  throw new Error(`${label} escapes its allowed directory: ${candidate}`);
}

export function replacePlaceholders(value, variables) {
  if (typeof value !== "string") return value;
  return value.replace(/\{([A-Za-z][A-Za-z0-9]*)\}/g, (match, name) => {
    if (!(name in variables)) throw new Error(`Unknown placeholder ${match}`);
    return String(variables[name]);
  });
}

export async function ensureDirectory(directory) {
  await mkdir(directory, { recursive: true });
  return directory;
}

export async function copyFixture(source, destination) {
  await assertNoSymlinks(source);
  await ensureDirectory(destination);
  await cp(source, destination, { recursive: true, force: true, errorOnExist: false });
}

async function assertNoSymlinks(root) {
  const metadata = await lstat(root);
  if (metadata.isSymbolicLink()) throw new Error(`Fixture cannot be a symbolic link: ${root}`);
  if (!metadata.isDirectory()) return;
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const child = path.join(root, entry.name);
    if (entry.isSymbolicLink()) throw new Error(`Fixture contains a symbolic link: ${child}`);
    if (entry.isDirectory()) await assertNoSymlinks(child);
  }
}

export async function sha256File(filePath) {
  const content = await readFile(filePath);
  return createHash("sha256").update(content).digest("hex");
}

export async function sha256Text(value) {
  return createHash("sha256").update(value).digest("hex");
}

export async function walkFiles(root, options = {}) {
  const result = [];
  const maximumFiles = options.maximumFiles ?? 10000;
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const fullPath = path.join(directory, entry.name);
      if (entry.isDirectory()) await visit(fullPath);
      if (entry.isFile()) result.push(fullPath);
      if (result.length > maximumFiles) throw new Error(`File walk exceeded ${maximumFiles} files under ${root}`);
    }
  }
  try {
    await visit(root);
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
  return result;
}

export async function snapshotPaths(workspace, relativePaths) {
  const snapshot = {};
  for (const relativePath of relativePaths ?? []) {
    const target = resolveInside(workspace, relativePath, "protected path");
    try {
      const metadata = await stat(target);
      snapshot[relativePath] = metadata.isFile()
        ? { kind: "file", sha256: await sha256File(target) }
        : { kind: "directory", files: await snapshotDirectory(target) };
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      snapshot[relativePath] = { kind: "missing" };
    }
  }
  return snapshot;
}

async function snapshotDirectory(directory) {
  const files = {};
  for (const file of await walkFiles(directory)) {
    files[path.relative(directory, file).replaceAll(path.sep, "/")] = await sha256File(file);
  }
  return files;
}

export function redact(value, secrets) {
  if (value === undefined || value === null) return value;
  let output = String(value).replace(/Bearer\s+[A-Za-z0-9._~+/=-]+/gi, "Bearer <redacted>");
  for (const secret of secrets ?? []) {
    if (secret) output = output.split(secret).join("<redacted>");
  }
  return output;
}
