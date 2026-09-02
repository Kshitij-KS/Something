---
meta:
  title: Maintain Callback's repository documentation
  navLabel: Documentation Plan
  category: Project
  contentType: Reference
---

# Maintain Callback's repository documentation

This page defines the audience, authority, vocabulary, and maintenance rules for Callback's repository documentation. Use it when code, release evidence, platform support, or product scope changes.

## Use one source for each kind of truth

Each document has one job. Resolve conflicts with this authority order:

1. **Implemented behavior**: source code, SQL migrations, package metadata, and workflows
2. **Current engineering state**: [`current-state.md`](current-state.md)
3. **Runtime design**: [`architecture.md`](architecture.md)
4. **Installed-runtime privacy contract**: [`privacy.md`](privacy.md)
5. **Human evidence requirements**: [`kill-gates.md`](kill-gates.md)
6. **Release qualification and publication**: [`release.md`](release.md)
7. **Planned work and release targets**: [`roadmap.md`](roadmap.md)
8. **Historical intent**: [`../callback-spec.md`](../callback-spec.md) and tool-specific plans

Historical documents can explain why a decision was made. They do not override current code or the references in `docs/`.

## Serve three audiences

The documentation supports three reader groups:

- **Testers**: need current capabilities, limitations, privacy boundaries, and manual qualification steps
- **Contributors**: need module ownership, runtime flows, development commands, and validation expectations
- **Release owners**: need gate definitions, artifact rules, signing status, and publication criteria

`README.md` provides the shared entry point. Each linked page then answers one reader task.

## Apply status terms consistently

Use these terms without treating them as synonyms:

- **Implemented**: production code exists in the repository
- **Automated validation passed**: the named test or check passed for a specific revision
- **Human evidence passed**: a person completed the defined trial and recorded evidence
- **Installed-build qualified**: the packaged application passed the Windows matrix
- **Release candidate**: immutable test artifacts exist, but publication criteria may remain open
- **General availability**: every required gate, installed check, distribution decision, and release approval passed
- **Planned**: accepted future scope without a completion claim
- **Exploratory**: research that has no release commitment

A feature can be implemented while its human gate remains pending. Version plans must show both states.

## Update documentation with code

Update the affected references in the same change when you modify:

- User-visible behavior or a supported workflow
- A Tauri command, protocol envelope, database migration, or state transition
- Capture selectors, permissions, framing limits, or privacy boundaries
- Build, validation, packaging, signing, or publication behavior
- A kill-gate definition, result, or release requirement
- Supported platforms, browsers, sites, or installation methods

Do not copy volatile gate results into several files. Keep gate definitions in `kill-gates.md` and version-specific evidence in the release record.

## Keep unresolved decisions explicit

The current documentation leaves these decisions open rather than guessing:

- The exact actionable-acceptance formula must be frozen before the two-week trial
- Authenticode timing and certificate ownership remain undecided
- Chrome Web Store identity and publication remain unproven
- The first supported non-Windows platform has not been selected
- Additional capture sites and browsers remain exploratory
- Local database encryption has no accepted design or release target

Close an open question only with a documented decision and supporting implementation or evidence.
