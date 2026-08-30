# Contributing to OpenTopia

Thank you for helping improve OpenTopia. This project is an active developer
preview, so focused fixes, tests, documentation, and well-scoped design
proposals are especially useful.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Before you start

- Search existing issues before opening a new one.
- Use the bug or feature request form so maintainers receive enough context.
- Open an issue before investing in a large feature, architectural change, new
  provider integration, or change to a security boundary.
- Report vulnerabilities privately according to [SECURITY.md](SECURITY.md).

Small fixes and documentation improvements can go directly to a pull request.

## Development setup

OpenTopia currently develops and packages on Windows.

### Prerequisites

- Rust stable toolchain
- Node.js 22 or newer
- pnpm 10 or newer
- Git

### Install and run

```powershell
git clone https://github.com/StagoMax/OpenTopia.git
cd OpenTopia
pnpm install
pnpm dev:desktop
```

The desktop development command starts both the Electron renderer and the local
Rust server. The built-in mock provider is the easiest way to work on UI or
runtime behavior without configuring external credentials.

Never commit API keys, provider credentials, signing identities, local
databases, or captured user data.

## How the repository is organized

| Path | Responsibility |
| --- | --- |
| `apps/desktop/` | Electron shell and React desktop UI |
| `crates/opentopia-core/` | Agent loop, tools, policy, persistence, and provider contracts |
| `crates/opentopia-server/` | Local API and event streaming |
| `crates/opentopia-cli/` | Command-line entry point |
| `crates/opentopia-windows-sandbox/` | Windows sandbox helper |
| `evaluation/` | Evaluation runner, suites, and result schemas |
| `docs/` | Architecture and operating documentation |

The [documentation index](docs/README.md) points to the current architecture
documents and design boundaries.

## Making a change

1. Create a focused branch from `main`.
2. Confirm the root cause and the module that owns the behavior before editing.
3. Keep the change scoped and preserve compatibility unless the proposal
   explicitly requires a breaking change.
4. Add or update tests around the changed boundary when practical.
5. Run focused checks while iterating, then the full verification suite before
   requesting review.
6. Update user-facing or architectural documentation when behavior changes.

### Desktop UI changes

Before changing visible UI under `apps/desktop/src`, read
[`design-system/MASTER.md`](design-system/MASTER.md) and
[`apps/desktop/src/styles/tokens.css`](apps/desktop/src/styles/tokens.css).

- Reuse primitives from `apps/desktop/src/components/ui/` where they fit.
- Use semantic design tokens instead of raw colors, spacing, radii, font sizes,
  or z-index values.
- Use `lucide-react` for icons.
- Keep keyboard focus visible and use semantic interactive elements.
- Give icon-only controls an `aria-label`.

## Validation

Use the smallest relevant check while developing:

```powershell
# Rust workspace
cargo check --workspace

# Desktop contracts, types, and tests
pnpm --filter @opentopia/desktop contracts:check
pnpm --filter @opentopia/desktop typecheck
pnpm --filter @opentopia/desktop test

# Design-system and repository boundary checks
pnpm design:check
pnpm boundary:check

# Evaluation harness tests
pnpm test:evaluation
```

Before opening a pull request, run the complete suite:

```powershell
pnpm check
```

If a check cannot be run locally, explain why and list the checks you did run
in the pull request.

## Pull requests

A reviewable pull request should:

- explain the problem and why the chosen boundary owns the fix;
- stay focused on one coherent change;
- link the relevant issue when one exists;
- describe user-visible and compatibility impact;
- include tests or explain why tests are not applicable;
- include screenshots or a short recording for visible UI changes;
- call out security, data migration, provider, sandbox, or performance impact;
- leave unrelated formatting and refactors out of the diff.

Maintainers may ask to split a pull request when independent changes would be
safer to review and revert separately.

## Commit messages

Use a short, imperative summary. The repository commonly uses Conventional
Commit-style prefixes:

```text
feat(desktop): add task filter
fix(agent): preserve approval state after replay
docs: clarify provider configuration
test(evaluation): cover timeout recovery
```

The prefix is a convention for readable history, not a substitute for a clear
pull request description.

## License

By contributing, you agree that your contributions will be licensed under the
project's [MIT License](LICENSE).
