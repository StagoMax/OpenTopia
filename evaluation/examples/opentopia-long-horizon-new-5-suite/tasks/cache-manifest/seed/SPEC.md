# Cache Manifest Verification Contract

Implement the dependency-free Node.js cache manifest verification tool in this workspace.

## Library

`src/cache.js` must export:

- `validateEntries(entries)`: validate an array of unique entries shaped `{ path, size, sha256 }`. Paths use `/`, are non-empty relative paths, contain no empty, `.` or `..` segment, and must not start with `/` or a drive prefix. Size is a non-negative integer. SHA-256 is exactly 64 hexadecimal characters and is normalized to lowercase. Return copies sorted by `path`.
- `diffCacheManifest(expected, observed)`: validate both arrays and return `{ missing, unexpected, changed, unchanged }`. Missing and unexpected entries use the normalized entry shape. Changed rows are `{ path, expectedSize, observedSize, expectedSha256, observedSha256, reasons }`; reasons contains `size` and/or `sha256` in that order. Unchanged rows use the normalized entry shape. Every array is sorted by path.
- `summarizeDiff(diff)`: return `{ expected, observed, missing, unexpected, changed, unchanged, valid }`. `valid` is true only when missing, unexpected, and changed are all zero.

Do not mutate caller-owned arrays or objects.

## CLI

`node src/cli.js --expected <json> --observed <json> --output <json>` reads an entry array from each input file, writes `{ "diff": ..., "summary": ... }` as pretty JSON with a trailing newline, and prints exactly:

`Verified E expected entries: M missing, U unexpected, C changed.`

A valid cache exits 0. A completed comparison that finds any difference still writes the report but exits 1. Invalid arguments or data must print a concise error to stderr, exit 2, and not write an output file.

## Constraints

- Use only Node.js built-ins.
- Do not modify this specification or files under `test/`.
- Run `npm test` before declaring a phase complete.
