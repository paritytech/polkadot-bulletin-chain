---
name: benchmarking
description: Benchmark a Substrate/Polkadot SDK runtime on a remote VM via SSH + rsync, then pull updated weight files back. Uses frame-omni-bencher.
---

# Remote Runtime Benchmark

Run runtime benchmarks on a remote VM (e.g. a reference machine) and bring the updated weight files back to the local working copy. Useful when local hardware is too slow or doesn't match reference specs.

The skill operates on the **current working directory's git repo** (a Substrate/Polkadot-SDK style project). Never store remote credentials or paths — ask every run.

## Prerequisites (tell the user up front)

- SSH key-based access to the remote VM
- `rsync`, `zstd`, and `tar` installed locally and on the remote
- Rust toolchain on the remote (matching the project's `rust-toolchain.toml` if present)
- `frame-omni-bencher` installed on the remote (this skill offers to install it)
- `python3` on the remote (the project's bench driver `scripts/cmd/cmd.py` is Python)

## Workflow

### 1. Verify local branch

```bash
git rev-parse --abbrev-ref HEAD
```

Ask the user which branch they want to benchmark. If it differs from the current branch:

```bash
git status --porcelain
```

If the working tree is dirty, ask the user how to proceed (continue with dirty tree on the chosen branch, stash, or abort). Then `git checkout <branch>`.

### 2. Ask for remote details (every run, no caching)

Ask the user two things, separately:

1. **Remote VM** — `user@host` (e.g. `ubuntu@10.0.0.5`)
2. **Remote parent folder** — absolute path on the VM under which we'll place the project (e.g. `/home/ubuntu/work`). The skill will create `<remote-parent>/<project-name>/` (where `<project-name>` is the local repo's directory basename, `$(basename $PWD)`) and copy the local working tree's contents into it.

**Do not persist any of these values to memory, env, or files.**

Git remote URL is **not** asked for — the skill never interacts with the remote git host (transport is tar+scp/rsync only).

### 3. Verify remote reachability

```bash
ssh -o BatchMode=yes -o ConnectTimeout=5 <user@host> 'echo ok && uname -a'
```

If this fails, stop and report.

### 4. Local → remote: tar+zstd, then scp, then extract

Use a compressed tarball for the initial upload — it's faster than rsync for a cold push.

Resolve project name and remote project folder:

```bash
PROJECT="$(basename "$PWD")"          # e.g. polkadot-bulletin-chain
REMOTE_PROJECT="<remote-parent>/$PROJECT"
ARCHIVE="/tmp/$PROJECT.tar.zst"
```

Create the archive locally, honoring `.gitignore` exactly via `git ls-files` (lists tracked + untracked-but-not-ignored files; this is more precise than rsync's `:- .gitignore` filter):

```bash
git ls-files --cached --others --exclude-standard -z \
  | tar --null -T - -cf - \
  | zstd -T0 -19 -o "$ARCHIVE"
```

Prepare the remote folder and copy the archive:

```bash
ssh <user@host> "mkdir -p $REMOTE_PROJECT"
scp "$ARCHIVE" <user@host>:/tmp/
```

Extract on the remote and clean up:

```bash
ssh <user@host> "tar -I zstd -xf /tmp/$PROJECT.tar.zst -C $REMOTE_PROJECT && rm /tmp/$PROJECT.tar.zst"
rm "$ARCHIVE"
```

Notes:
- `git ls-files --cached --others --exclude-standard` gives the canonical "files git would track or could track" list; `.git/`, `target/`, and anything in `.gitignore` are automatically excluded. Requires being inside a git repo (the skill assumes this).
- `zstd -T0` uses all cores; `-19` is high compression. Drop to `-3` if you want speed over size.
- The `/tmp/` archive is removed on both sides after extraction.

### 5. Pick the runtime

If `scripts/runtimes-matrix.json` exists in the repo, parse it and present the `name` field of each entry as a choice. Capture the matching `name`, `package`, and `path`.

Example (bulletin chain): `bulletin-westend` → package `bulletin-westend-runtime`, path `runtimes/bulletin-westend`.

If the matrix file is absent, ask the user directly for the cargo `package` name and the runtime `path` (relative to repo root).

Optionally ask: *"Run all pallets, or a filtered subset (space-separated names)?"*

### 6. Ensure `frame-omni-bencher` is on the remote

Check first:

```bash
ssh <user@host> 'command -v frame-omni-bencher || ls ~/.cargo/bin/frame-omni-bencher 2>/dev/null'
```

If absent, ask the user:
> "frame-omni-bencher is not installed on the remote. Choose: (a) I'll install it myself — abort and re-run later, or (b) install it now via `cargo install frame-omni-bencher --locked` (takes several minutes)."

If (b):

```bash
ssh <user@host> 'cargo install frame-omni-bencher --locked'
```

### 7. Build the runtime wasm on the remote

```bash
ssh <user@host> "cd $REMOTE_PROJECT && cargo build --profile production -p <package> --features runtime-benchmarks"
```

Wasm output path (replace `-` with `_` in package name for the file):

```
target/production/wbuild/<package>/<package_underscored>.wasm
```

Stream output so the user sees build progress.

### 8. Write the launcher script on the remote

The skill **does not ship a static runner script**. Instead it streams a small launcher to `/tmp` on the remote via `ssh + heredoc`. The launcher delegates per-pallet benchmarking to the project's existing driver `scripts/cmd/cmd.py`, which already handles XCM templates, excluded extrinsics, and runtime-matrix lookups (no need to duplicate that logic).

Pick a `<tag>` (use the runtime name from step 5, e.g. `bulletin-paseo`).

Stream the launcher script:

```bash
ssh <user@host> "cat > /tmp/<tag>.launcher.sh" <<'LAUNCHER_EOF'
#!/usr/bin/env bash
# Generated by the benchmarking skill. Drives scripts/cmd/cmd.py one pallet
# at a time so each invocation gives an atomic OK/FAIL via its exit code.
set -uo pipefail

# Non-interactive SSH usually doesn't load .bashrc/.profile.
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

: "${BENCH_PROJECT_DIR:?must be set}"
: "${BENCH_RUNTIME:?must be set (matches a name in scripts/runtimes-matrix.json)}"
: "${BENCH_PACKAGE:?must be set (cargo package name)}"
: "${BENCH_TAG:?must be set}"

cd "$BENCH_PROJECT_DIR"

LOG="/tmp/$BENCH_TAG.log"
STATUS="/tmp/$BENCH_TAG.status"
DONE="/tmp/$BENCH_TAG.done"

: > "$LOG"
: > "$STATUS"
rm -f "$DONE"

TS() { date -u '+%Y-%m-%dT%H:%M:%SZ'; }

# Resolve pallet list: BENCH_PALLETS (newline-separated) wins, else --list.
if [[ -n "${BENCH_PALLETS:-}" ]]; then
  mapfile -t PALLETS <<< "$BENCH_PALLETS"
else
  echo "$(TS) listing pallets via frame-omni-bencher..." | tee -a "$LOG" "$STATUS"
  PKG_UNDER="${BENCH_PACKAGE//-/_}"
  WASM="target/production/wbuild/$BENCH_PACKAGE/$PKG_UNDER.wasm"
  if [[ ! -f "$WASM" ]]; then
    echo "$(TS) ERROR: wasm not found at $WASM. Build it first (step 7)." | tee -a "$LOG" "$STATUS"
    touch "$DONE"; exit 1
  fi
  mapfile -t PALLETS < <(
    frame-omni-bencher v1 benchmark pallet --no-csv-header --all --list \
      --runtime="$WASM" 2>/dev/null \
    | awk -F, '{print $1}' | sort -u | grep -v '^$'
  )
fi

if [[ ${#PALLETS[@]} -eq 0 ]]; then
  echo "$(TS) ERROR: no pallets to bench" | tee -a "$LOG" "$STATUS"
  touch "$DONE"; exit 1
fi

echo "$(TS) starting bench: ${#PALLETS[@]} pallets | runtime=$BENCH_RUNTIME" | tee -a "$LOG" "$STATUS"

OK=(); FAIL=()
for i in "${!PALLETS[@]}"; do
  P="${PALLETS[$i]}"
  IDX=$((i+1))
  echo "$(TS) [$IDX/${#PALLETS[@]}] start $P" | tee -a "$LOG" "$STATUS"
  if python3 scripts/cmd/cmd.py bench --runtime "$BENCH_RUNTIME" --pallet "$P" >>"$LOG" 2>&1; then
    OK+=("$P")
    echo "$(TS) [$IDX/${#PALLETS[@]}] OK   $P" | tee -a "$STATUS"
  else
    FAIL+=("$P")
    echo "$(TS) [$IDX/${#PALLETS[@]}] FAIL $P" | tee -a "$STATUS"
  fi
done

echo "$(TS) finished. success=${#OK[@]} failed=${#FAIL[@]}" | tee -a "$LOG" "$STATUS"
echo "success: ${OK[*]:-(none)}" | tee -a "$STATUS"
echo "failed:  ${FAIL[*]:-(none)}" | tee -a "$STATUS"
touch "$DONE"
LAUNCHER_EOF

ssh <user@host> "chmod +x /tmp/<tag>.launcher.sh"
```

Notes:
- The launcher does **not** rebuild the runtime — step 7 already did that. `cmd.py` will internally call `cargo build` once per invocation; on a hot target that's a fast incremental no-op (~1s).
- Per-pallet `OK`/`FAIL` lines come from the exit code of each `cmd.py` call (one pallet per invocation), so the `.status` file gives atomic progress.
- XCM template selection, excluded extrinsics, and runtime-matrix config are all handled by `cmd.py` — the launcher is intentionally thin.

### 9. Launch the benchmark loop, detached

Build the env-var prelude from the runtime selection in step 5:

```bash
ENV_PRELUDE="\
  BENCH_PROJECT_DIR=$REMOTE_PROJECT \
  BENCH_RUNTIME=<runtime-name> \
  BENCH_PACKAGE=<package> \
  BENCH_TAG=<tag>"
```

Optional override if the user requested a pallet subset (newline-separated):

```bash
ENV_PRELUDE+=" BENCH_PALLETS=$(printf '%s\n' pallet_a pallet_b)"
```

Launch detached so the run survives the ssh session ending:

```bash
ssh <user@host> "$ENV_PRELUDE nohup /tmp/<tag>.launcher.sh </dev/null >/tmp/<tag>.runner.out 2>&1 & disown; echo launched pid=\$!"
```

The launcher produces three files in `/tmp` (prefixed with `BENCH_TAG`):

- `<tag>.log` — full stdout/stderr of every `cmd.py` invocation
- `<tag>.status` — one line per pallet (`OK` / `FAIL`) plus the final summary
- `<tag>.done` — touched when the loop finishes (success or fail)

### 9a. Monitor progress

To poll status (every few minutes is fine):

```bash
ssh <user@host> "tail -n 30 /tmp/<tag>.status; echo ---; ls /tmp/<tag>.done 2>/dev/null && echo 'DONE' || echo 'IN PROGRESS'"
```

You can schedule recurring check-ins with `CronCreate` (e.g. cron `*/5 * * * *`) and report each fire's tail back to the user. Delete the cron once `<tag>.done` exists.

If something looks stuck, inspect the active pallet's output:

```bash
ssh <user@host> "tail -n 100 /tmp/<tag>.log"
```

### 10. Pull updated weight files back (rsync — fast on incremental)

Wait until `<tag>.done` exists on the remote, then use rsync to pull only the changed weight files. Scope tightly to the runtime's `src/weights/` directory so unrelated local files are never touched:

```bash
rsync -avz \
  --filter=':- .gitignore' \
  --exclude='.git' \
  <user@host>:$REMOTE_PROJECT/<runtime-path>/src/weights/ \
  ./<runtime-path>/src/weights/
```

Also pull the status file for the user's records (optional):

```bash
scp <user@host>:/tmp/<tag>.status ./<tag>.status.txt
```

### 11. Show the diff

```bash
git status
git diff --stat <runtime-path>/src/weights/
```

Tell the user to review the changes and commit when ready. Suggest running `/format` before committing (project convention).

### 12. Clean up the remote project folder (with explicit confirmation)

After the user has confirmed the pulled-back files look correct, ask:

> "Benchmarking finished and the updated weight files are pulled back. Delete the remote project folder `<user@host>:$REMOTE_PROJECT` and the launcher artefacts `/tmp/<tag>.{launcher.sh,log,status,done,runner.out}` to free space? (yes/no)"

Only on a clear `yes` from the user, run:

```bash
ssh <user@host> "rm -rf $REMOTE_PROJECT /tmp/<tag>.launcher.sh /tmp/<tag>.log /tmp/<tag>.status /tmp/<tag>.done /tmp/<tag>.runner.out"
```

If the user says no, leave it in place and tell them the paths so they can clean up later.

## Constraints

- **Never persist** `user@host` or remote paths (no memory writes, no temp files, no env).
- **Do not interact with the remote git host.** No `git push`, no clone on the remote — transport is tar+scp (push) and rsync (pull) only.
- Do not run destructive commands (`rm -rf`, etc.) on the remote without explicit confirmation. The step 12 cleanup must be confirmed with a clear `yes`.
- The remote `.git` directory (if any) is never overwritten — `git ls-files` excludes it on push, `--exclude='.git'` covers it on the rsync pull.

## Common failure modes

- **`git ls-files` returns nothing / fails**: the working directory isn't a git repo. The skill assumes a git repo — abort and tell the user.
- **`zstd` not installed locally**: install via the system package manager (`apt install zstd`, `brew install zstd`, etc.) before retrying.
- **`frame-omni-bencher` not found after install**: ensure `~/.cargo/bin` is in the remote's `PATH`. Try `ssh <user@host> 'source ~/.cargo/env && frame-omni-bencher --version'`.
- **Empty pallet list**: the wasm wasn't built with `--features runtime-benchmarks`, or the build silently failed. Re-check step 7's output.
- **Permission denied on remote parent folder**: the user may need to `mkdir`/`chown` the parent path manually.
- **rsync pull pulls more than expected**: the source path in step 10 should be tightly scoped to `<runtime-path>/src/weights/`. Don't widen it.
