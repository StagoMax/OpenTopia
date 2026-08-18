import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import ts from "typescript";

const clientPath = join(process.cwd(), "src", "api", "client.ts");
const source = await readFile(clientPath, "utf8");
const file = ts.createSourceFile(
  clientPath,
  source,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TS,
);
const helperNames = new Set(["get", "post", "patch", "put", "delete"]);
const edits = [];

for (const statement of file.statements) {
  if (
    !ts.isClassDeclaration(statement) ||
    statement.name?.text !== "ApiClient"
  ) {
    continue;
  }
  for (const member of statement.members) {
    if (!ts.isMethodDeclaration(member) || !member.body || !member.name)
      continue;
    const methodName = member.name.getText(file);
    if (helperNames.has(methodName) || methodName === "openAuthenticatedSse") {
      continue;
    }
    visit(member.body, methodName);
  }
}

function visit(node, methodName) {
  if (ts.isCallExpression(node)) {
    const expression = node.expression.getText(file);
    const helper = /^this\.(get|post|patch|put|delete)$/.exec(expression);
    if (helper && node.arguments.length > 0) {
      const first = node.arguments[0];
      if (!ts.isStringLiteral(first) || first.text !== methodName) {
        edits.push({
          position: first.getStart(file),
          text: `"${methodName}", `,
        });
      }
    }
    if (
      expression === "parseResponse" &&
      node.arguments.length === 1 &&
      node.arguments[0]
    ) {
      edits.push({
        position: node.arguments[0].getEnd(),
        text: `, "${methodName}"`,
      });
    }
  }
  ts.forEachChild(node, (child) => visit(child, methodName));
}

let migrated = source;
for (const edit of edits.sort(
  (left, right) => right.position - left.position,
)) {
  migrated =
    migrated.slice(0, edit.position) +
    edit.text +
    migrated.slice(edit.position);
}
await writeFile(clientPath, migrated, "utf8");
console.log(`registered ${edits.length} HTTP contract call sites`);
