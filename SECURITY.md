# Security Policy

OpenTopia can execute commands, access local workspaces, connect to model
providers, and load extensions. Security reports are taken seriously,
especially when they affect sandbox boundaries, approvals, credentials, local
data, or untrusted tool output.

## Supported versions

OpenTopia is currently a developer preview without stable releases. Security
fixes are applied to the latest revision of the `main` branch. Older commits,
local forks, and unofficial builds are not maintained as supported versions.

## Reporting a vulnerability

Do **not** open a public issue for a suspected vulnerability.

Email `laiyejian666@gmail.com` with the subject
`[OpenTopia Security] <short description>`. Include, when available:

- the affected commit or version;
- the relevant component and configuration;
- reproduction steps or a minimal proof of concept;
- the potential impact;
- any suggested mitigation;
- whether the report may be shared with upstream projects.

Please remove API keys, access tokens, private source code, personal data, and
other unrelated secrets from logs or attachments. You should receive an
acknowledgement within five business days. Investigation and remediation time
will depend on severity and complexity.

## Scope

Examples of in-scope issues include:

- escaping an enforced sandbox or bypassing an approval boundary;
- exposing provider credentials or stored secrets to the renderer, logs, or an
  unauthorized extension;
- unauthorized workspace access or path traversal;
- unsafe handling of untrusted MCP, plugin, skill, browser, or computer-use
  content;
- cross-task data leakage;
- remote code execution through a default OpenTopia workflow.

Reports about a third-party model provider, MCP server, plugin, browser, or
other dependency may need to be coordinated with that project's maintainer.
OpenTopia's documentation, defaults, and integration behavior remain in scope.

## Safe evaluation

The preview should be evaluated in a disposable or version-controlled
workspace. Review approval prompts, use the narrowest viable sandbox mode, and
do not assume that “local-first” means prompts never leave the machine. A
configured remote provider receives the context required to complete a model
request.

## Disclosure

Please allow reasonable time to investigate and publish a fix before public
disclosure. OpenTopia will credit reporters who want acknowledgement, unless
they prefer to remain anonymous.
