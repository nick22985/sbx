# Project `.sbx/` examples

Every sbx project keeps its config in a single `.sbx/config.toml` at the
project root. This folder shows an annotated example next to the few
optional files that still live alongside it.

Start from these:

```sh
cd ~/path/to/some/project
sbx init npm                              # creates .sbx/config.toml with flavor = "npm"
cp ../sbx/examples/sbx/config.toml  .sbx/config.toml   # then edit
```

If you have an older project with separate `flavor`, `hostname`, `ports`,
etc. files in `.sbx/`, run `sbx migrate` once and it will consolidate them
into `config.toml` and move the originals into `.sbx/legacy/`.

## Files

| File          | Purpose                                                 | Format                           |
| ------------- | ------------------------------------------------------- | -------------------------------- |
| `config.toml` | Required. All project knobs live here.                  | TOML. See the annotated example. |
| `Dockerfile`  | Optional. Per-project layer on top of the flavor image. | Standard Dockerfile.             |

`flavor` (set inside `config.toml`) is the only required value; everything
else is opt-in.

## Generated vs hand-edited

`.sbx/config.toml` is meant to be hand-edited and committed. `sbx config <subcommand>`
commands rewrite the same file in place. For unshared/worktree-private state,
sbx mirrors a second `config.toml` under `$XDG_DATA_HOME/sbx/private/<key>/.sbx/`
- the local file always wins when both are present.
