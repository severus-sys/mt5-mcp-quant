# Domain Docs

This repository uses a single-context domain documentation layout.

## Before exploring, read these

- `CONTEXT.md` at the repository root, when present.
- Relevant ADRs under `docs/adr/`, when present.

Missing domain files are not errors. Continue silently. Create or update them only when domain terminology or an architectural decision is actually resolved.

## Layout

```
/
├── CONTEXT.md
├── docs/adr/
└── src/
```

## Vocabulary

Use terminology defined in `CONTEXT.md`. Avoid drifting to synonyms the glossary explicitly rejects.

## ADR conflicts

If proposed work conflicts with an existing ADR, surface the conflict explicitly rather than silently overriding the recorded decision.
