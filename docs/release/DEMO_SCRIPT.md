# GitHub Release Demo Script

## Goal

Show the core Section story in 30 to 60 seconds:

1. connect remote storage
2. sync into a local folder
3. work in normal local paths
4. surface sync state only when it matters

## Demo Setup

Prepare:

- one local filesystem demo source
- one bound local root
- one text file under `docs/readme.txt`
- a terminal with enough font size to read commands

Suggested paths:

- remote source root: `/srv/section-demo-source`
- local bound root: `~/section-demo`

## Demo Flow

### 1. Add and bind

```bash
section source add demo --provider fs --opt root=/srv/section-demo-source
section source bind demo ~/section-demo
```

Narration:

> Section starts with a source and a local folder. Humans and agents will both work in that local tree.

### 2. Sync

```bash
section source sync demo
```

Narration:

> Sync pulls the remote source into the local tree and records sync truth in the control plane.

### 3. Show the local tree

```bash
cd ~/section-demo
ls
cat docs/readme.txt
```

Narration:

> Daily work stays in normal local paths.

### 4. Start watch

```bash
section --json watch ~/section-demo
```

Narration:

> Agents subscribe once, then react to events instead of polling every file.

### 5. Trigger a divergence

Create one local change and one remote change.

Then:

```bash
section source sync demo
section --json path compare ~/section-demo/docs/readme.txt
```

Narration:

> If both sides changed, Section enters conflict instead of silently letting timing decide.

### 6. Resolve explicitly

```bash
section --json path resolve ~/section-demo/docs/readme.txt --strategy use-local
```

Narration:

> Resolve is explicit and happens by local path.

## Recording Notes

- keep the recording under 60 seconds
- keep commands large and readable
- cut dead time between command execution
- prefer one clear text-file conflict over a busier example
- end on the local tree, not on internal architecture
