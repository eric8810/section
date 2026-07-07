# Section AgentFS Requirements

## Status

This document records the product direction for Section as an AgentFS.

It is directional product requirements, not a detailed implementation plan. The current source/path sync layer remains the infrastructure baseline. The AgentFS direction defines the business semantics that should sit above that sync layer.

Section AgentFS should remain a focused filesystem product.

The concrete first design proposal lives in [AGENTFS_DESIGN_PROPOSAL.md](AGENTFS_DESIGN_PROPOSAL.md). The development contract for the MVP lives in [AGENTFS_MVP_CONTRACT.md](AGENTFS_MVP_CONTRACT.md).

## Product Definition

Section is an agent-owned, agent-governed shared filesystem.

Agents create filesystems, grant other agents access, and move changes into shared truth through policy-controlled commits.

正式产品必须有服务端控制面。

```text
Section Control Service 负责身份、发现、grant、share、source profile、credential。
AgentFS Local 负责 attach、local edit、status、commit。
Sync layer 负责文件同步。
```

跨机 / 多 agent 分享必须走 Section Control Service。不要设计离线 invite file、手动 source bootstrap、直接分享 source 密钥。

## Core Distinction

Section's core distinction is not a checklist of filesystem features.

The core distinction is:

```text
Agent-governed truth.
```

Agents do not merely use the filesystem. Agents govern what becomes the shared filesystem truth.

Most agent filesystem projects focus on one of these ideas:

- agent uses a filesystem
- agent state lives in a filesystem
- agent workspace can checkpoint, restore, or fork
- agent runtime can run hooks
- many agents can mount the same workspace

Section's point of view is different:

```text
agent owns filesystem
agent grants filesystem access
agent decides which mutations become truth
filesystem records and enforces that governance
```

From this point of view, grants, commits, `AGENTS.md`, hooks, watch, and audit are not separate differentiating features. They are expressions of the same core model: shared filesystem truth is governed by agents.

## Product Boundary

For this repo, the boundary is:

```text
Build one excellent agent-governed filesystem.
```

Other resource types may appear inside AgentFS as files or artifacts, but they are not first-class Section AgentFS resources.

## Core Product Model

The minimum AgentFS core is:

- `Agent`
- `Installation`
- `FS`
- `SourceProfile`
- `Grant`
- `Share`
- `Credential`
- `Commit`
- `Event`

These extension concepts are important, but they should not define the first core:

- `Rule`
- `Hook`

The relationship is:

```text
Agent creates FS
Agent owns FS
Agent grants other agents access through the control service
Agent shares FS through a server-side share record
Other agents accept shares after authentication
Agents attach FS as normal files with server-issued sync credentials
Agents edit locally
Agents commit through policy
All agents observe accepted FS mutations
```

Rules and hooks extend this model after the commit boundary is clear.

## Agent Perspective

From an agent's perspective, Section should answer these questions:

- Which filesystems do I own?
- Which filesystems can I access?
- What rules apply inside this filesystem?
- What files can I read?
- What files can I change?
- What changes am I allowed to commit?
- What changed while I was away?
- Which accepted mutations affected the filesystem truth?
- Which policy or grant allowed an accepted mutation?

Later automation and rules layers should also answer:

- Which hooks may run before or after my action?

## Directional Requirements

### 1. Agent-Owned Filesystems

An agent can create an FS.

The creating agent becomes the owner. Ownership is the root authority for managing access, rules, and policy decisions for that FS.

The FS must persist independently from any single runtime, process, machine, or session.

### 2. Grant-Based Access

Access is not implicit.

An agent can access an FS only if it owns the FS or has been granted access by an authorized agent.

The authoritative grant registry lives in Section Control Service. Metadata in the backing source may mirror grant state for audit or sync, but it must not be the product authority for cross-machine access.

Grants should express coarse capabilities first:

- read
- commit
- manage

In the normal local-directory model, Section cannot reliably prevent raw local file edits. Grants should therefore govern access to the FS and acceptance of changes into shared truth, not every local write syscall.

Path-scoped grants and resolve capabilities may exist later, but the first product direction should keep the mental model simple: an agent can read an FS, commit to an FS, or manage an FS.

### 3. Local File Interface

An attached FS should appear as normal files and directories.

Agents should be able to use ordinary tools:

- shell commands
- editors
- test runners
- compilers
- language servers
- existing file-based workflows

The local file tree is the work surface. Section's control plane carries ownership, grants, policy, events, conflict state, and commit state.

The local file tree is not the authority boundary in the first design. The authority boundary is commit.

### 3a. Service-Controlled Sharing

An FS can be shared only through the Section Control Service.

The service must handle:

- authenticated agent identity
- installation identity
- server-side share records
- FS discovery for granted agents
- source profile lookup
- short-lived sync credential issuance
- share and grant audit

A share is not a file and is not a source credential.

```text
share = server-side access record
accept = authenticated agent accepts that server-side record
credential = short-lived sync credential issued after grant check
```

The product must not require agents to manually copy source configuration or exchange long-lived source keys.

### 4. Local Edits Are Not Shared Truth

An agent may edit local files after attaching an FS.

Local edits are work-in-progress. They do not automatically become the shared truth of the FS.

This distinction is central:

```text
local edit != accepted FS mutation
commit accepted by policy == shared truth mutation
```

### 5. Policy-Controlled Commits

Commit is the boundary where a local change becomes shared FS truth.

In the first design, before accepting a commit, Section should check:

- the committing agent's grants
- the current FS state
- conflict or stale-base state

Later policy layers may add:

- target path scope
- FS rules
- required approval
- required validation steps

If accepted, the FS truth advances and other agents can observe the mutation.

If rejected, the change remains local or pending.

The first design should define accepted commits as the governed truth record. The backing source is the materialized filesystem state used for ordinary file access and sync.

### 6. Observable Mutations

All accepted FS mutations must be observable.

At minimum, agents should be able to see:

- who changed something
- what paths changed
- when the change happened
- why the change happened
- which previous state it was based on
- which grant allowed it

Later policy layers should also expose which rule or approval allowed it.

Section should avoid silent shared-truth mutation. Agent-visible eventing is part of the product contract.

### 7. FS-Local Rules

FS-local rules are an extension layer after the core commit boundary.

Each FS can define its own agent rules through `AGENTS.md`.

`AGENTS.md` is the FS-local behavior contract for agents. It should describe rules such as:

- required files to read before changing the FS
- allowed or forbidden path scopes
- required tests before commit
- paths that require owner approval
- conflict handling policy
- communication expectations

Section should treat `AGENTS.md` as a first-class rules surface for agent behavior, while keeping final enforcement in the control plane.

Because `AGENTS.md` influences behavior, changing it should require manage authority or owner approval once rules enforcement exists.

### 8. Hooks For Agent And Script Automation

Hooks are an automation layer after the core commit boundary.

An FS should support hooks so agents and scripts can automate repeatable behavior around filesystem activity.

Hooks are not the primary trust boundary. They are automation attached to FS events and policy stages. The control plane still decides whether a mutation is accepted.

Hooks should be able to run around events such as:

- FS attach
- local change detected
- commit proposed
- commit preflight
- commit accepted
- commit rejected
- conflict detected
- grant changed
- rules changed

Hook targets may include:

- a local script
- an agent
- a command in a sandbox
- a remote webhook
- a validation step

Typical uses:

- run tests before commit
- format files before proposing a commit
- ask an owner agent for approval
- notify another agent about a path
- generate a summary of changed files
- block risky paths unless an approval exists
- update derived files after an accepted commit

Hook execution should be observable. Agents should be able to see which hook ran, why it ran, what it produced, and whether it affected the commit decision.

An FS may define hooks through a dedicated control-plane API, and may reference expected hooks from `AGENTS.md` so agents understand the local automation contract.

Hooks need an explicit trust model before they can block commits. At minimum, hook installation and mutation should require owner or manage authority.

## Minimal Roles

The initial role model can stay small:

- `owner`: can manage the FS, grants, rules, and policy decisions
- `reader`: can attach and read
- `writer`: can attach, edit locally, and commit accepted changes
- `manager`: can grant/revoke and manage FS metadata without being the original owner

These roles are product concepts. They can later map to a richer policy engine if needed.

## Current Section Mapping

Current Section concepts map into AgentFS as infrastructure:

| Current Section Concept | AgentFS Meaning |
| --- | --- |
| source | backing storage |
| local root | mounted work surface |
| source/path sync state | FS truth and path state infrastructure |
| watch | mutation and coordination event feed |
| compare | commit preflight and freshness check |
| resolve | conflict or policy decision |
| `.section/root.json` | local discovery for attached FS |
| hooks | deferred automation layer around FS events |

The AgentFS direction does not discard source/path sync. It makes source/path sync the substrate under an agent-owned filesystem product model.

## Non-Goals For The Direction

Section AgentFS is not primarily:

- a generic cloud drive
- a general-purpose Git replacement
- a memory or RAG database
- an agent runtime or sandbox manager

Those capabilities may integrate with Section, but the core product direction is shared filesystem governance for agents.

## Product Summary

The shortest product statement is:

```text
FS is owned by agents.
Access is granted.
Local edits are not automatically truth.
Shared truth changes through policy-checked commits.
All accepted mutations are observable.
```

Rules and hooks extend this core after the commit boundary is stable.

Section should evolve from a sync-aware shared folder into an agent-owned shared filesystem where agents can safely collaborate through files.
