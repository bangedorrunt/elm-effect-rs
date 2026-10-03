# Build a CLI

A process that parses arguments, runs one effect, and exits. A watcher that lives across messages is an application. Use `build-app.md` for that. General flag, help, and stdout rules are clig.dev. Load the `cli-design` skill when the command surface is the work. This file is how that surface sits on elm-effect.

## Crate

Own crate, depends on `elm-effect`. Parsing is a pure function from argv to a typed request, or a typed parse error. The effect starts after that value exists.

```text
src/cli.rs     argv -> Request, no I/O, no Effect
src/main.rs    layer, one effect, exit code
```

Clap 4 parses argv. The clap types do not leak into the domain effect. A test calls the pure parser with a `Vec<String>`.

## Exit

Use the exit table in ADR-0002, section "Product shape". Do not invent a second code for usage. `--json` serializes the success value. Human text is the TTY path. A service does not call `println` or `eprintln`.

## Order

1. Define the request enum. Subcommands are variants. Matching the variant does not open a socket or a file.
2. Invalid combinations fail in the parser, before `main` builds a layer.
3. Config files are read into `Boot` before the runtime starts. A bad file is a typed error and does not construct services. Precedence is flags, then environment, then the config file. Secrets are file paths or stdin, not flag values.
4. `main` builds the layer once, runs one effect with `Runtime::run_main`, and maps the exit table above.
5. A fiber the command starts is joined or scoped before `main` returns. `fork_detach` in a CLI keeps the process up after the result is printed.
6. `--dry-run` is a command variant whose interpreter performs no write. It is not a boolean checked inside the write.
7. Ctrl-C is an interrupt. The scope releases, then the process exits 130. A second interrupt does not skip a finalizer that already started. Finalizers are uninterruptible.

## Done

- [ ] Parser tests cover success, a missing required flag, and `--json` shape, with no runtime.
- [ ] A dropped effect performs no I/O.
- [ ] stdout of a success holds the value and no log line.
- [ ] `cargo test` passes with no network.
