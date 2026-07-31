# Dependency Release Planner Contract

Implement `src/planner.js` and `src/cli.js` without external dependencies.

## Library

`parseWorkspace(jsonText)` accepts JSON shaped as:

```json
{
  "packages": [
    { "name": "core", "dependencies": [] },
    { "name": "app", "dependencies": ["core"] }
  ]
}
```

The root must contain exactly `packages`. Every package must contain exactly `name` and
`dependencies`. Names must match `^[a-z][a-z0-9-]{0,39}$`. Package names and each package's
dependencies must be unique. Reject self-dependencies, missing package references, malformed JSON,
unknown fields, and non-string values. Return a new array of packages sorted by name, with each
dependency list sorted. Do not mutate parsed inputs.

`buildReleaseWaves(packages)` returns an array of arrays. A package can appear only after all its
dependencies appeared in earlier waves. Put every currently available package in the same wave,
sort names inside each wave, and reject dependency cycles with an error containing `cycle`.

`renderReleasePlan(packages)` returns:

```json
{
  "packageCount": 2,
  "waveCount": 2,
  "waves": [
    { "index": 1, "packages": ["core"] },
    { "index": 2, "packages": ["app"] }
  ]
}
```

## CLI

`node src/cli.js --input <workspace.json> --output <plan.json>` reads the workspace, writes the
rendered plan as two-space JSON with one trailing newline, and prints exactly:

```text
Planned <packages> packages in <waves> waves.
```

On invalid arguments, missing dependencies, or cycles it must exit non-zero, explain the error on
stderr, and not create the output file. Do not modify `SPEC.md` or files under `test/`.
