# CLI conventions

## `--output-format` is mandatory (load-bearing rule)

Every `hydra` CLI command MUST support both a structured JSON-lines output and
a human-readable "pretty" output. The mechanism is the existing global
`--output-format` flag, defined once on the top-level `Cli` struct:

```text
--output-format <OUTPUT_FORMAT>  Output format (auto, jsonl, or pretty). [default: auto]
                                 [possible values: auto, jsonl, pretty]
```

- `auto` resolves to `pretty` on a TTY and `jsonl` otherwise — this is what
  scripts and agents actually see.
- `jsonl` emits one JSON object per line on stdout. Agents parse this; humans
  redirect it.
- `pretty` emits a formatted table/tree.

This is a hard requirement, not a nice-to-have. Agents drive `hydra` over
`jsonl` and break if a new command only renders pretty output. Existing
commands already follow this — `hydra issues create`, `hydra repos update`,
and friends all route through `CommandContext::output_format`.

### How to wire it on a new command

The flag is parsed once on the root `Cli` (see `hydra/src/cli.rs`) and reaches
your command as `CommandContext`. Render via the `Render` trait so both
formats share one call site:

```rust
pub trait Render {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()>;
    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()>;
}

render(MyRecords(&items), context.output_format, &mut buffer)?;
```

If you find yourself reaching for `println!` in a command body, stop —
implement `Render` for the response type and dispatch through the context.

## Other recurring conventions

- **One subcommand per file** under `hydra/src/command/`. CLI subcommand
  modules stay isolated; thin synchronous wrappers around async helpers are
  fine.
- **Flag naming is kebab-case.** Use `#[arg(long = "default-branch")]`, not
  `default_branch` (clap will rename `_` to `-`, but spell it out so the
  rendered `--help` is unambiguous).
- **`value_name` is `SCREAMING_SNAKE_CASE`** for the metavar that shows up in
  `--help`: `value_name = "ISSUE_ID"`, not `"issue-id"`.
- **Pair `--foo` with `--clear-foo`** when a field can be both set and
  explicitly unset, and add `conflicts_with` so the two can't be combined.
- **Env-var-backed args** use `#[arg(env = ENV_...)]` from
  `hydra-common::constants` — see [style.md](style.md#environment-variables-declare-on-the-arg-struct).
- **`--help` lines start with a capital letter and end with a period.** The
  rendered help is user-facing copy; treat it like prose.

## When prose names a command

Any prose in this repo that names a `hydra` subcommand or flag must match the
rendered `<cmd> --help` exactly — named flag vs. positional, required vs.
optional, value names. The cheap check is `cargo run -p hydra -- <cmd> --help`
before you commit.
