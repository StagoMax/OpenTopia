# Atomic static deployment

Implement the dependency-free tool in `src/deploy.js` and `src/cli.js`.

`buildRelease(source, output, manifest)` receives a source directory and a
manifest `{ files: [{ source, destination, sha256 }] }`. Validate every relative
path, reject duplicates and hash mismatches, and copy only listed files into a
new release. A failed build must leave the previous output untouched. A
successful build atomically replaces it and writes `release.json` containing
the sorted destination paths and a deterministic `contentSha256` computed from
`destination + "\0" + file bytes` in destination order.

`createStaticServer(root)` returns an HTTP server that:

- serves only files below root;
- maps `/` to `/index.html`;
- returns 404 for absent files and 400 for malformed URL escapes;
- rejects traversal instead of normalizing it outside root;
- sends an ETag equal to the quoted lowercase SHA-256 of the response body;
- returns 304 when `If-None-Match` matches.

CLI commands:

```text
node src/cli.js build --source <dir> --manifest <json> --output <dir>
node src/cli.js serve --root <dir> --port <number>
```

The server prints `Serving <root> on <port>.` once it is listening.
