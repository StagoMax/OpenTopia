const path = require("node:path");
const { runChecks } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || "");
const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, checks: [
  { id: "precedence-decoding-and-query", run: async ({ assert, importFresh, path, workspace }) => {
    const { Router } = await importFresh(path.join(workspace, "src/router.js"));
    const router = new Router()
      .add("GET", "/files/:name", "parameter")
      .add("get", "/files/settings.json", "static")
      .add("GET", "/files/*rest", "wildcard");
    assert.deepEqual(router.match("gEt", "/files/settings.json?raw=1"), { value: "static", params: {} });
    assert.deepEqual(router.match("GET", "/files/a%20b"), { value: "parameter", params: { name: "a b" } });
    assert.deepEqual(router.match("GET", "/files/a/b%20c"), { value: "wildcard", params: { rest: "a/b c" } });
    assert.equal(router.match("GET", "/files/%E0%A4%A"), null);
  } },
  { id: "pattern-validation-and-literal-escaping", run: async ({ assert, importFresh, path, workspace }) => {
    const { Router } = await importFresh(path.join(workspace, "src/router.js"));
    const router = new Router().add("GET", "/v1.0/items", "literal");
    assert.equal(router.match("GET", "/v1x0/items"), null);
    assert.throws(() => new Router().add("GET", "/:id/:id", "bad"), /duplicate|parameter/i);
    assert.throws(() => new Router().add("GET", "/a/*rest/end", "bad"), /wildcard|terminal/i);
  } },
  { id: "public-tests", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" });
    assert.equal(run.status, 0, run.stderr || run.stdout);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
