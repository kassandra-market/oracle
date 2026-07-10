# Kassandra agent skills

Reusable [agent skills](https://github.com/vercel-labs/skills) distilled from working on
this repo. Each skill is a directory with a `SKILL.md` (YAML frontmatter: `name` +
`description`, then the guidance), following the flat `skills/<name>/SKILL.md` layout that
`npx skills` discovers.

## Install

Install every skill in this repo into your coding agent (Claude Code, etc.):

```bash
npx skills add Dodecahedr0x/kassandra
```

Or a single skill directly:

```bash
npx skills add https://github.com/Dodecahedr0x/kassandra/tree/master/skills/wire-format-single-source-of-truth
```

List what's available without installing:

```bash
npx skills add Dodecahedr0x/kassandra --list
```

## Skills

**Integrating with Kassandra** (small, specific reference skills for building against the
protocol):

| Skill | What it's for |
| --- | --- |
| [`kassandra-ts-client`](./kassandra-ts-client/SKILL.md) | Build a Kassandra instruction, decode an account, or derive a PDA from **TypeScript / a dApp** via `@kassandra-market/oracles`. |
| [`kassandra-rust-client`](./kassandra-rust-client/SKILL.md) | Same from **Rust** (a test harness, keeper/bot, or service) via the `kassandra-oracles-sdk` crate. |
| [`kassandra-ai-runner`](./kassandra-ai-runner/SKILL.md) | Produce, submit, or verify a Kassandra **AI claim** with the `kassandra-runner` (`run` / `verify` / keeper `--submit`). |

**General technique:**

| Skill | What it's for |
| --- | --- |
| [`wire-format-single-source-of-truth`](./wire-format-single-source-of-truth/SKILL.md) | When a second consumer (tests, an off-chain keeper/bot, a frontend) needs to build the same on-chain instruction / protocol message / wire payload — define the encoding once in a client library derived from the schema and route every consumer through it, instead of hand-rolling account metas + discriminants + serialization in each place. |

## Adding a skill

Skills follow test-driven development (see the `superpowers:writing-skills` methodology):
write a pressure scenario, watch an agent fail without the skill, write the skill to fix
exactly that failure, then re-test. Keep the `description` frontmatter in **double quotes**
(and avoid `:` + space or unescaped `"` inside it) so strict YAML parsers like `npx skills`
can read it.
