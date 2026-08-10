# Fixed core prompt modules

Each Markdown file in this directory owns one concern of the fixed OpenTopia
agent contract. The authoritative assembly order lives in
`crates/opentopia-core/src/base_prompt.rs` as `BASE_PROMPT_MODULES`.

To change wording, edit only the relevant module. To add, remove, or reorder a
module, update the manifest and its order test in `base_prompt.rs`. The assembler
normalizes outer whitespace, inserts exactly one blank line between modules,
and preserves a final newline in the compiled prompt.

Conditional and dynamic prompt modules remain assembled after this fixed core
by `prompt_runtime.rs` and `agent_model_context_with_runtime`.
