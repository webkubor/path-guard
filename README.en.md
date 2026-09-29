# path-guard

Find out which **copy** of a command actually runs when you type it, what shadows it, and whether updates are actually taking effect.

## What it fixes

On 2026-09-01, OpenCode popped up:

```
Update Complete
Successfully updated to OpenCode v1.18.25. Please restart the application.
```

After restart, still the same "new version available" prompt. Update, restart, repeat. **The updater said success, the version never moved.**

The truth: four copies of node's global packages were on the machine, the 7th entry in PATH was a symlink pointing to a 1.17.8 installed by a different node version three months earlier, and the update had landed in the 21st entry.

**No one was lying**: npm honestly reported "1.18.25 installed", the updater saw npm exit code 0, the shell faithfully ran the first PATH match. Three components, each correct on its own, wrong together.

Following the same line uncovered more: `lark-cli` on a version mismatch (running 1.0.39, idle 1.0.69, npm at 1.0.92), 106 dangling symlinks pointing at deleted directories, 12 GB of zombie copies.

Existing tools all answer **"what did I install"** (`npm ls -g`, `brew list`, `pip list`). None answer **"what am I actually running, what's shadowing it, and does the running version match what's installed?"**

## Usage

```bash
path-guard doctor              # one-shot health check
path-guard which <cmd>         # deep which: hit, shadow, all copies + versions
path-guard dupes [--versions]  # list all commands with multiple copies
path-guard dangling            # dangling symlinks (visible to ls, fail at exec)
```

### `which` — the most common one

```
$ path-guard which opencode
command opencode

▶ actual exec  ~/.local/share/mise/installs/node/24.15.0/bin/opencode
                → ~/.local/share/mise/installs/node/24.15.0/lib/node_modules/opencode-ai/bin/opencode.exe
   runtime version 1.18.25
   declared by    opencode-ai@1.18.25

✅ no other copies, nothing shadows it
```

On the day of the incident, the same command would have shown `runtime version 1.17.8` while `declared by opencode-ai@1.18.25` — instant signal that installed ≠ running.

## Three design decisions

**One: "Multiple paths" ≠ "multiple copies."**
The same file referenced via several symlinks is normal layout (homebrew's bin is exactly this). You have to deduplicate by `realpath`; anything still >1 after that is a real duplicate. Counting paths alone floods you with false positives.

**Two: Runtime version and declared version are fetched separately, both are fetched.**
- Runtime: actually run `<cmd> --version` — what the user actually gets
- Declared: read from the owning `package.json` — what the package manager thinks

The divergence *is* the "update didn't take effect" signature. Looking at either side alone misses it.

**Three: Package name is not identity.**
When migrating I ran `npm i -g facet` and got a *different* package with the same name on npm ("Configuration mixin for constructors"), while the one I wanted had never been published and was a local `npm link`. So `declared_package` returns name alongside version, not just version.

## Why dangling symlinks get their own category

```
❌ ~/.local/bin/arkcli
   → ~/.nvm/versions/node/v24.15.0/bin/arkcli     # this directory was deleted
```

This is the sneakiest kind: `ls` sees it, `which` finds it, tab-completion completes, **and it only fails at exec time** with "no such file".

And `grep -r "nvm"` won't catch it — symlink targets only live in filesystem metadata, never in file content. Only `find -type l` with per-link `realpath` reveals them. After deleting some runtime, this residue scatters across PATH, app data dirs, various `node_modules`.

## Install

```bash
cargo install --path .
# or
cargo build --release && cp target/release/path-guard ~/.local/bin/
```

## Known limitations

- Version detection relies on `--version`; CLIs that don't support it only get a declared version (with no runtime check).
- Only scans `PATH`. App data directories (e.g. `~/.dsh/profiles/node_modules`) are not covered — the 106 dangling DSH symlinks lived outside PATH.
- Declared version only understands `package.json`; pipx / brew / cargo-installed packages aren't wired up yet.

## Install globally

```bash
cargo build --release
cp target/release/path-guard ~/.local/bin/     # make sure ~/.local/bin is in PATH
```

Or `cargo install --path .` (puts it in `~/.cargo/bin`).

Then just use it:

```bash
path-guard doctor
```

## `pinned` — the only preventive check

Everything else is post-mortem diagnostics; this one prevents the next incident.

```
$ path-guard pinned
PATH entries with version numbers baked in: 2
⚠️  ~/Library/Python/3.14/bin
    version segment 3.14
⚠️  ~/.nvm/versions/node/v24.15.0/bin
    version segment v24.15.0
    fix       nvm has no version-agnostic entry; consider mise/volta (which expose shims)
```

The reason it exists: after migrating to mise I changed `~/.nvm/versions/node/v24.15.0/bin` to `~/.local/share/mise/installs/node/24.15.0/bin` in 5 LaunchAgents and 2 scripts — **thought I'd fixed it; actually reproduced the same bad design with a new tool** — putting a version number into the path welds "where the tool is" and "which version of the tool" together. When node moves to 25, all of these will silently break, with the failure showing up the next time that service restarts, by which point nobody remembers the migration.

The right pattern is the version manager's **version-agnostic entry**: mise uses `shims/`, pyenv/rbenv/asdf likewise, homebrew uses `opt/<formula>/bin` instead of `Cellar/<version>/`.

## License

MIT