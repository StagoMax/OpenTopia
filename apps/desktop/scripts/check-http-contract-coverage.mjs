import { readdir, readFile } from "node:fs/promises";
import { join, relative } from "node:path";
import ts from "typescript";

const desktopRoot = process.cwd();
const clientPath = join(desktopRoot, "src", "api", "client.ts");
const clientDirectory = join(desktopRoot, "src", "api", "client");
const schemaPath = join(
  desktopRoot,
  "src",
  "api",
  "generated",
  "desktop-http-v1.schema.json",
);
const clientModulePaths = (
  await readdir(clientDirectory, {
    withFileTypes: true,
  })
)
  .filter((entry) => entry.isFile() && entry.name.endsWith(".ts"))
  .map((entry) => join(clientDirectory, entry.name))
  .sort();
const clientPaths = [clientPath, ...clientModulePaths];
const [sources, schemaText] = await Promise.all([
  Promise.all(clientPaths.map((path) => readFile(path, "utf8"))),
  readFile(schemaPath, "utf8"),
]);
const schema = JSON.parse(schemaText);
const registered = new Set(Object.keys(schema.properties ?? {}));
const used = new Set();
const errors = [];
const jsonHelpers = new Set(["get", "post", "patch", "put", "delete"]);
const nonJsonFetchMethods = new Set([
  "getPluginAppContent",
  "getPreviewContent",
  "openAuthenticatedSse",
]);

for (const [index, source] of sources.entries()) {
  const sourcePath = clientPaths[index];
  const sourceLabel = relative(desktopRoot, sourcePath).replaceAll("\\", "/");
  const file = ts.createSourceFile(
    sourcePath,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );

  visit(file, (call) => {
    if (call.expression.getText(file) !== "JSON.parse") return;
    const line =
      file.getLineAndCharacterOfPosition(call.getStart(file)).line + 1;
    errors.push(
      `${sourceLabel}:${line}: JSON parsing must stay inside a registered HTTP or SSE decoder`,
    );
  });

  for (const statement of file.statements) {
    if (!ts.isClassDeclaration(statement)) continue;
    for (const member of statement.members) {
      if (!ts.isMethodDeclaration(member) || !member.body || !member.name)
        continue;
      const methodName = member.name.getText(file);
      if (jsonHelpers.has(methodName)) continue;
      let usesFetch = false;
      let hasBoundaryDecoder = false;

      visit(member.body, (call) => {
        const expression = call.expression.getText(file);
        const helperMatch = /^this\.(get|post|patch|put|delete)$/.exec(
          expression,
        );
        if (helperMatch) {
          checkContractArgument(
            call.arguments[0],
            methodName,
            call,
            file,
            sourceLabel,
          );
          hasBoundaryDecoder = true;
        } else if (expression === "parseResponse") {
          checkContractArgument(
            call.arguments[1],
            methodName,
            call,
            file,
            sourceLabel,
          );
          hasBoundaryDecoder = true;
        } else if (expression === "fetch") {
          usesFetch = true;
        }
      });

      if (
        usesFetch &&
        !hasBoundaryDecoder &&
        !nonJsonFetchMethods.has(methodName)
      ) {
        errors.push(
          `${sourceLabel}:${methodName}: raw fetch is missing a registered boundary decoder`,
        );
      }
    }
  }
}

for (const contract of registered) {
  if (!used.has(contract)) {
    errors.push(`${contract}: Rust response contract has no Desktop call site`);
  }
}

if (errors.length > 0) {
  throw new Error(
    `Desktop HTTP contract coverage failed:\n${errors.join("\n")}`,
  );
}

console.log(`covered ${used.size} JSON HTTP response contracts`);

function checkContractArgument(argument, methodName, call, file, sourceLabel) {
  const line = file.getLineAndCharacterOfPosition(call.getStart(file)).line + 1;
  if (!argument || !ts.isStringLiteral(argument)) {
    errors.push(
      `${sourceLabel}:${methodName}:${line}: contract key must be a string literal`,
    );
    return;
  }
  if (argument.text !== methodName) {
    errors.push(
      `${sourceLabel}:${methodName}:${line}: contract key is ${JSON.stringify(argument.text)}`,
    );
  }
  if (!registered.has(argument.text)) {
    errors.push(
      `${sourceLabel}:${methodName}:${line}: ${argument.text} is not registered by Rust`,
    );
  }
  used.add(argument.text);
}

function visit(node, callback) {
  if (ts.isCallExpression(node)) callback(node);
  ts.forEachChild(node, (child) => visit(child, callback));
}
