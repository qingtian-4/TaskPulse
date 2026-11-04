---
name: taskpulse-maintenance
description: Maintain TaskPulse's repository layout, shared guidance, verification entrypoints and module boundaries while preserving behavior and compatibility. Use for cleanup and structural maintenance, without replacing product design or authorizing deployment.
---

# TaskPulse maintenance

Use the existing architecture and checks. Aim for clear ownership and fewer
duplicated contracts, not fewer directories or a new stack. Read the
[architecture map](../../../docs/ARCHITECTURE.md) and relevant references below.

## Scope and classify

1. Read root guidance, inspect status/current origin/main, tracked files,
   package/CI entrypoints and actual callers. Preserve user edits and unfinished
   work. A local-only request takes precedence over standing remote workflow.
2. Briefly classify candidates as keep, move/merge, delete after proof, or
   uncertain. Search imports, generated commands, runtime embedding/bundle
   resources, subprocess paths, fixtures, docs and CI. No literal caller is not
   proof of disuse: optional/manual scripts and extensionless imports need review.
3. Preserve locales and both native/frontend consumers, protocols, sanitized
   fixtures, locks, license sources and published install/connection paths.
   `scripts/hooks/` is shipped runtime code, not a test directory.
4. Work in reviewable batches: guidance/guards, path moves, then justified local
   extraction. Compare moved code and review path rewrites before executing it.

## Select the owner

| Area | Read / preserve |
| --- | --- |
| Providers, watcher, store, requests, config | [Runtime contracts](references/runtime.md) |
| Native windows, settings, process ownership, WSL/focus | [Platform rules](references/platforms.md) |
| Build tools, CI, installers/upgrades, audits/artifacts | [Delivery rules](references/delivery.md) |

Keep adapters at the edges and durable-state ownership explicit. Extract pure
transformations or presentation when they have a clear owner and regression
coverage. Keep collection scheduling/generation guards, configuration CAS and
SourceSettings close/draft ownership in place unless the task needs deeper work.

Do not add a parallel provider/plugin system, event pipeline, runner or schema
copy. Shared protocol/capability definitions remain canonical. Another project's
organization does not imply adopting its stack or lifecycle semantics.

## Verify

- Update imports, relative roots, embedded resources, subprocess helpers, package
  scripts, CI commands/path filters and documentation together. Search old full
  paths and review dynamic/sibling references after a move.
- Reuse existing test scripts, frontend check/build, Rust fmt/Clippy/full tests
  and `scripts/verify.ps1`. Preserve separate CI exit-code checks. Do not weaken
  assertions or security to accommodate a structural change.
- After moving entrypoints/resources, verify a clean checkout with locked
  dependencies, actual `dev:desktop` startup and applicable offline/native smoke.
  A dry-run or browser mock does not verify native startup.
- Use temporary profiles, random loopback ports and exact owned processes.
  Inspect side effects before running a smoke. Live Agent/SSH/WSL scripts are
  conditional, not a batch of safe offline tests.
- Add boundary/guard tests when they catch a real failure, not to repeat wording.

## Finish

Record commits, commands/results, exact build identity/hashes and owned cleanup
locally. Distinguish actual checks from unavailable hosts/hardware. Keep real
accuracy/rendered-P95/return/resource evaluation separate from fixtures and short
native smoke. Preserve uncertain files instead of deleting to meet a count.

Maintain reusable guidance here and in focused references. Put brief ownership
facts in the map and commands in CONTRIBUTING/package scripts. Do not commit
task-specific audit reports, inventories, raw profiles or captures. Explicitly
allowlist shared guidance while excluding personal `.agents` content, secrets
and temporary output. User scope continues to control external actions.
