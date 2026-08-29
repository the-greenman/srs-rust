## Summary

<!-- What does this PR do, and why? One or two sentences. -->

## Linked issue

Closes #<!-- issue number -->

<!-- Every PR must reference a tracking issue so work is never stranded on an abandoned branch.
     Use "Closes #N" so the issue auto-closes on merge and the board reconciles.
     If this PR intentionally should NOT close the issue, use "Refs #N" and say why. -->

## Checklist

- [ ] Linked to a tracking issue above (`Closes #N`, or `Refs #N` with a reason)
- [ ] Tests / validation pass locally
- [ ] If I close this PR **without merging** and the work is still wanted, I will reopen/flag the linked issue with a pointer to the branch before abandoning it

## Implementation charter (substantive PRs only — see ADR-048)

Presence-only, judgment stays human:

- [ ] **Spec-first** — governing spec decision/RFC/invariant named
- [ ] **Layer test** — owning layer named (core service / CLI-WASM adapter / client)
- [ ] **One way per goal** — existing mechanism named, or declared-twin exception justified
- [ ] **Parity and mirror obligations** — schema mirrors / payload sync / pin choreography named (or "none")
- [ ] **Decision mode** — clear/complicated/complex/chaotic named
