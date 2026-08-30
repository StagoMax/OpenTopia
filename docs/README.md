# OpenTopia documentation

This directory contains architecture notes, implementation decisions, operating
guides, and evaluation reports. Use this page as the stable entry point; many
other files are working notes tied to a specific investigation or date.

## Start here

| Document | Use it for |
| --- | --- |
| [Executive summary](executive-summary.md) | Product and technical overview |
| [Detailed architecture](architecture-detailed.md) | Components, data flow, and major dependencies |
| [Agent runtime boundaries](agent-runtime-boundaries.md) | Ownership between the desktop, server, core, and tools |
| [Current agent-loop architecture](agent-loop-architecture-current.md) | Agent execution and orchestration flow |
| [Implementation backlog](implementation-backlog.md) | Known gaps and release work |
| [Source adaptation map](source-adaptation-map.md) | Upstream influences and adapted designs |

## Runtime and safety

- [AI coding work-agent architecture](ai-coding-work-agent-architecture.md)
- [Context compaction design](context-compaction-design.md)
- [Plan mode runtime design](plan-mode-runtime-design.md)
- [Planning tools architecture](planning-tools-architecture-current.md)
- [Multi-agent architecture analysis](multi-agent-architecture-analysis.md)
- [MCP sandbox implementation plan](mcp-sandbox-implementation-plan.md)
- [MCP attachment inspection](mcp-attachment-inspection-capability.md)
- [Harness and plugin boundary](harness-plugin-boundary-design-zh-cn.md)
- [Provider tool-cache release gate](provider-tool-cache-release-gate.md)

## Desktop and integrations

- [Browser and computer-use design](browser-computer-use-technical-design.md)
- [Web search integration](web-search.md)
- [Library retrieval providers](library-retrieval-providers.md)
- [Flow mode refactor design](flow-mode-refactor-design-zh-cn.md)
- [Enterprise agent platform design](enterprise-agent-platform-design-zh-cn.md)

## Evaluation

- [Evaluation system](evaluation-system.md)
- [Application-agent evaluation framework](application-agent-evaluation-framework.md)
- [Architecture benchmark protocol](evaluations/architecture-benchmark-protocol.md)
- [Evaluation reports and fixtures](evaluations/)
- [Runnable evaluation package](../evaluation/README.md)

## Design records

- [ADR 0001: MVP shell and runtime](adr/0001-mvp-shell-and-runtime.md)
- [Prompt and context classification](prompt-context-classification.md)
- [OpenTopia prompt runtime design](opentopia-prompt-runtime-design-zh-cn.md)
- [Target architecture refactor](agent-core-target-architecture-refactor-zh-cn.md)
- [Earlier architecture refactor plan](architecture-refactor-plan-zh-cn.md)
- [Archived research material](research/README.md)

## Document conventions

- Prefer stable, descriptive filenames for documents linked from this index.
- Put dated investigations and benchmark results under `docs/evaluations/` or a
  clearly named analysis document.
- Mark proposals as proposed, accepted, superseded, or implemented when their
  status would otherwise be ambiguous.
- Update this index when adding a new long-lived guide or architectural source
  of truth.

For contribution workflow and validation commands, see
[CONTRIBUTING.md](../CONTRIBUTING.md).
