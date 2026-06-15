# E29 — End-to-End Testing

> **Status:** Draft
>
> **Version:** 0.1   ·   **Last updated:** 2026-06-15
>
> **Purpose:** How babel-lsp is tested end to end — the coverage policy for full protocol journeys, the `pytest-lsp` harness, and the patterns every feature's E2E plan reuses. Each feature's own journeys live in its spec's §12 and link here.
>
> **Depends on:** [constitution](../constitution.md), [E17-testing](E17-testing.md)   ·   **Related:** [E01-architecture](E01-architecture.md)

> Requirement tag: **E2E**

---

## 1. Purpose & Scope

This spec defines how we test complete journeys through the running server, driving it the way a real editor does: a JSON-RPC client over stdio against the built binary. It is the authority every feature's **End-to-End Test Plan** (§12) defers to.

This spec covers:

- The E2E coverage policy every user-facing feature meets.
- The `pytest-lsp` harness and how it drives the real binary.
- Environment, seeding, and teardown for repeatable runs.
- The protocol-conformance journeys every feature inherits.
- Naming and structure for `E2E-NN` scenarios.

Out of scope: unit and integration testing and the shared fixtures registry — those live in [E17-testing](E17-testing.md).

## 2. Coverage Policy

**REQ-E2E-01 — Cover 100% of feature scope, end to end.**

Each user-facing feature's E2E plan (its §12) covers **all of its user-visible scope**: the happy path **and** every reasonably possible error path — an unknown msgid, a malformed catalog, a missing `pybabel`, an unresolved call, an empty workspace. A journey a real user can hit must have a scenario.

**REQ-E2E-02 — Happy and error paths are both first-class.**

An E2E plan that only walks the happy path is incomplete. Error paths are enumerated from the feature's edge cases (§10); each gets its own `E2E-NN` scenario with an asserted outcome.

## 3. Tools & Harness

The standard E2E toolchain; versions are pinned in [E03-tech-stack](E03-tech-stack.md).

- **Driver:** `pytest-lsp` — a real LSP client speaking JSON-RPC over stdio to the built `babel-lsp lsp` binary. This is the exact path an editor takes, so it catches integration bugs unit tests can't: wrong capability flags, off-by-encoding ranges, out-of-order notifications.
- **Runner & reporting:** `pytest`, with the server's stderr log captured on failure.
- **Where E2E runs:** locally via `pytest tests/e2e`, and in `qa.yml` ([F16](../features/F16-release-ci.md)) against the release build.

## 4. Environment, Seeding & Teardown

- **Test data:** each scenario opens a fixture workspace from the [E17 fixtures registry](E17-testing.md#5-fixtures-registry) (clean-shopfront and its broken variants); external tools use [fake-pybabel](E17-testing.md#fake-pybabel).
- **Isolation:** each scenario gets a fresh copy of its fixture in a temp dir and its own server process, so no scenario sees another's edits.
- **Teardown:** the server is shut down (`shutdown`/`exit`) and the temp workspace removed between scenarios.

## 5. Patterns

- **Wait on state, never sleep.** Scenarios await the publish, the response, or the relink they expect — never a fixed delay. The "a newly opened file always receives a publish" guarantee ([E01 REQ-ARCH-10](E01-architecture.md)) is the canonical "pass 2 ran" signal a scenario synchronizes on.
- **Assert ranges, not just codes.** A diagnostic or hover scenario asserts the exact range and content, against both negotiated encodings using [non-ascii-catalog](E17-testing.md#non-ascii-catalog).
- **Drive the real surfaces.** Scenarios call the real `textDocument/*` and `workspace/executeCommand` methods, not internal helpers.
- **Flake policy:** a flaky scenario is quarantined and fixed, never retried-until-green.

**REQ-E2E-03 — Protocol conformance is a shared journey set.**

Every feature inherits the protocol-conduct journeys from [E01 §5.6](E01-architecture.md), tested once here: a newly opened file always receives a (possibly empty) publish; a relink that clears a finding sends an empty publish; two rapid `didChange` events apply in order; an in-flight request honors `$/cancelRequest`; and an external catalog write (via the watcher) re-indexes and updates diagnostics ([F01 REQ-CAT-09](../features/F01-catalog-index.md)). Features reference these rather than re-testing them.

## 6. Conventions

- **Naming:** scenarios are `E2E-NN` within a feature, titled by the journey.
- **Structure:** given (a seeded fixture) → when (client requests) → then (asserted response/publish). When [acceptance criteria](../constitution.md#46-non-functional--operational-scope) are enabled, the same scenarios are written Given/When/Then in the feature's §12.3.
- **Where feature E2E plans link:** every feature's §12 links here for the harness and patterns rather than restating them.

## 7. Running E2E & CI

`pytest tests/e2e` runs the journeys locally against the built binary. `qa.yml` ([F16](../features/F16-release-ci.md)) runs them on every push and PR; a failing journey blocks merge, and the server log plus the failing exchange are attached as artifacts.

## 8. Cross-References

- **Depends on:** [constitution](../constitution.md) — the coverage principles this enforces; [E17-testing](E17-testing.md) — the categories and the fixtures registry this reuses.
- **Related:** [E01-architecture](E01-architecture.md) — the protocol conduct (§5.6) the conformance journeys assert.

## 9. Changelog

- **2026-06-15** — Initial draft: the E2E coverage policy (happy + error paths), the `pytest-lsp` stdio harness, fixture-backed seeding/isolation, the wait-on-state patterns, and the shared protocol-conformance journey set (REQ-E2E-03) carved out of the old E17.
