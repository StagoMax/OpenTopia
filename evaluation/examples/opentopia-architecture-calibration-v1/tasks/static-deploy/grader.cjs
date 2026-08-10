const path = require("node:path");
const { runChecks, temporaryDirectory } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || "");
const seed = path.join(__dirname, "seed");
const digest = (crypto, value) => crypto.createHash("sha256").update(value).digest("hex");

runChecks({ workspace, seed, protectedPaths: ["RUNBOOK.md", "test"], checks: [
  { id: "atomic-build-and-manifest", run: async ({ assert, crypto, fs, importFresh, path, workspace }) => {
    const { buildRelease } = await importFresh(path.join(workspace, "src/deploy.js"));
    const root = temporaryDirectory("deploy-hidden-"); const source = path.join(root, "src"); const output = path.join(root, "out");
    fs.mkdirSync(source); fs.mkdirSync(output); fs.writeFileSync(path.join(output, "old.txt"), "old");
    fs.mkdirSync(path.join(source, "assets")); fs.writeFileSync(path.join(source, "index"), "HOME"); fs.writeFileSync(path.join(source, "assets/app.js"), "APP");
    const manifest = { files: [
      { source: "assets/app.js", destination: "assets/app.js", sha256: digest(crypto, "APP") },
      { source: "index", destination: "index.html", sha256: digest(crypto, "HOME") }
    ] };
    await buildRelease(source, output, manifest);
    assert.equal(fs.existsSync(path.join(output, "old.txt")), false);
    assert.equal(fs.readFileSync(path.join(output, "assets/app.js"), "utf8"), "APP");
    const release = JSON.parse(fs.readFileSync(path.join(output, "release.json"), "utf8"));
    assert.deepEqual(release.files, ["assets/app.js", "index.html"]);
    assert.match(release.contentSha256, /^[a-f0-9]{64}$/);
    fs.writeFileSync(path.join(source, "index"), "CHANGED");
    await assert.rejects(buildRelease(source, output, manifest), /hash|sha/i);
    assert.equal(fs.readFileSync(path.join(output, "index.html"), "utf8"), "HOME");
  } },
  { id: "server-http-contract", run: async ({ assert, crypto, fs, importFresh, path, request, workspace }) => {
    const { createStaticServer } = await importFresh(path.join(workspace, "src/deploy.js"));
    const root = temporaryDirectory("serve-hidden-"); fs.writeFileSync(path.join(root, "index.html"), "hello");
    const server = createStaticServer(root); await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    try {
      const port = server.address().port; const first = await request(`http://127.0.0.1:${port}/`);
      assert.equal(first.status, 200); assert.equal(first.body, "hello");
      assert.equal(first.headers.etag, `"${digest(crypto, "hello")}"`);
      const traversal = await request(`http://127.0.0.1:${port}/%2e%2e/secret`); assert.notEqual(traversal.status, 200);
    } finally { await new Promise((resolve) => server.close(resolve)); }
  } },
  { id: "public-tests", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" });
    assert.equal(run.status, 0, run.stderr || run.stdout);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
