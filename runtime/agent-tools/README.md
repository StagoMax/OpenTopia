# OpenTopia agent tools runtime

This runtime is the stable command-line boundary exposed to local coding
agents. Windows release builds currently bundle two pinned tools:

- `rg` for fast workspace search;
- MinGit for non-interactive source-control operations.

The source archives, versions, maximum sizes, and SHA-256 digests are pinned in
`runtime-lock.json`. Run the preparer directly for development or let
`scripts/build-desktop.ps1` prepare it during a release build:

```powershell
pnpm runtime:agent-tools
```

Set `OPENTOPIA_AGENT_TOOLS_ROOT` to a prepared directory to test an explicit
offline runtime. Large, capability-specific runtimes such as Node, Python, and
media tooling should be added as separately activated capabilities instead of
growing the always-on core bundle.
