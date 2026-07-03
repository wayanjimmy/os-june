# Bundled Hermes skills

This directory contains June-bundled read-only Hermes skills that ship outside
the pinned Hermes runtime. June adds this directory to `skills.external_dirs` at
Hermes startup, after user-global `~/.agents/skills`.

## unbroker

Source: `NousResearch/hermes-agent/optional-skills/security/unbroker`
Commit: `20c83af66485fc1cc546bae4477ddbbc55bd9d0b`

The current pinned Hermes runtime is `v2026.6.19`, and the latest tagged Hermes
release available when this was added (`v2026.7.1`) did not include `unbroker`.
Vendoring the skill keeps the runtime compatibility pin unchanged while making
the skill available in June.
