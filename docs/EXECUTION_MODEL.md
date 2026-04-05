# Mounted Workspace Execution Model

## Why this exists

Section's product goal is not "multiple access methods." The target model is that humans, agents, shell tools, editors, and scripts all operate on the same mounted workspace.

That means execute/scripting semantics need to be defined around the mounted tree first, with `section exec` kept as a supplementary fallback.

## Primary execution model

The primary execution surface is the mounted path:

- `bash /mnt/section/<source>/scripts/task.sh`
- `python3 /mnt/section/<source>/tools/analyze.py`
- `node /mnt/section/<source>/scripts/job.mjs`
- editors and IDEs opening `/mnt/section/<source>/...`

These should all behave like normal filesystem workflows:

- paths are real mounted paths, not Section-specific URIs
- relative-path script logic should resolve inside the mounted tree
- writes should flow back to the backend through the mounted path
- humans and agents should observe the same tree shape and file locations

## Role of `section exec`

`section exec` remains useful, but it is not the main collaborative model.

Use `section exec` for:

- quick fallback execution when mount is unavailable
- diagnostics and spot checks
- early bootstrap before the mounted workspace is ready

Do not treat `section exec` as the canonical collaboration path, because it bypasses the shared mounted namespace that shell tools, editors, and humans are supposed to share.

## Consistency model for scripts

Scripts running against the mounted workspace should assume:

- writes performed through the mount are authoritative and should persist to the backend
- reads through the mount reflect the current mounted cache view
- out-of-band backend mutations are not required to appear instantly in a previously cached mounted path
- when freshness matters after an out-of-band mutation, refresh the mounted path and then read it again

For the current Linux adapter, refresh visibility is exposed through the mounted path xattr:

- `user.section.refresh` on Linux
- `section.refresh` on other platforms when available

`section refresh <path>` remains a control-plane convenience wrapper around that same mounted-path refresh behavior when a mount is active.

## Failure expectations

Scripts should fail like normal filesystem workflows when the mount is unavailable:

- missing mount path
- permission denied from the adapter/runtime/backend
- stale view before an explicit refresh after an out-of-band mutation

The CLI control plane is responsible for helping manage that state:

- `section status`
- `section source add/remove/list`
- `section refresh`

But the script itself should still target the mounted path as its working surface.

## Validation scenarios

The Linux reference path should cover at least these scenarios:

1. Run a bash script directly from the mounted workspace and verify relative-path reads/writes stay inside the mounted tree.
2. Mutate backend content out of band, trigger refresh on the mounted path, then run a Python script from the mounted workspace and verify it reads the refreshed content.
3. Confirm the resulting files are visible both through the mounted path and in the backend source directory.

The repo now includes a repeatable validation script for that flow:

- `scripts/validate-mounted-workspace-exec.sh`

Run it on a Linux environment with FUSE available:

```bash
scripts/validate-mounted-workspace-exec.sh
```

Expected result:

- it prints `VALIDATION_OK`
- bash and Python both execute from the mounted workspace
- write-through and refresh semantics both hold
