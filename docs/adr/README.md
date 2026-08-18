# Architecture Decision Records

This directory holds Architecture Decision Records (ADRs) for `paws` — short documents that
capture a significant architectural decision, the context that forced it, the options actually
considered, and why one was chosen over the others. See [adr.github.io](https://adr.github.io/)
for background on the practice in general.

Format: [MADR](https://adr.github.io/adr-templates/) (Markdown Architectural Decision Records),
using the full template — decision drivers and a pros/cons breakdown of the options considered,
not just the outcome. That trade-off record is the point: it's what lets someone later ask "why
didn't we just do X" and get a real answer instead of re-litigating a settled question from
scratch.

## Index

- [0001 — Route all container execution through Dagger, not direct `docker`/`cross` spawns](0001-route-container-execution-through-dagger.md)

## When to write one

Write an ADR when a decision would be expensive to re-litigate from scratch later — usually
because it involves a real trade-off (not a single obviously-correct option), affects how future
work in the area gets built, or reverses/revises an earlier approach that was already shipped.
Don't write one for a decision with no real alternative, or for something already fully explained
by [`.specify/memory/constitution.md`](../../.specify/memory/constitution.md)'s governing
principles — an ADR documents a specific decision, not a standing rule.

## Numbering

Sequential, zero-padded to 4 digits (`0001`, `0002`, ...), never reused. A superseded ADR stays in
place with its status updated (`Superseded by ADR-00NN`) rather than being deleted — the record of
why the old decision was made is still useful context, even after it's replaced.
