import { describe, expect, it } from 'vitest';
import { RequestGuard } from '../requestGuard.ts';

describe('RequestGuard', () => {
  it('has an initial token that is current', () => {
    const guard = new RequestGuard();
    // The guard starts with no fetch in flight; `next()` issues the first
    // real token. Before that, no token but the freshly-issued one is
    // current — verify via the first `next()` call below rather than
    // poking at private state.
    const token = guard.next();
    expect(guard.isCurrent(token)).toBe(true);
  });

  it('advancing with next() makes the old token stale and the new one current', () => {
    const guard = new RequestGuard();
    const first = guard.next();
    expect(guard.isCurrent(first)).toBe(true);

    const second = guard.next();
    expect(guard.isCurrent(first)).toBe(false);
    expect(guard.isCurrent(second)).toBe(true);
  });

  it('discards a stale out-of-order response: newer request resolves first, older resolves after and is stale', () => {
    // Reproduces the Session 10-11 scenario: two refreshes are started back
    // to back (e.g. two rapid control changes), each grabbing a token at
    // the START of its fetch. The network then resolves them out of order
    // — the second-started (newer) request's response comes back first,
    // and the first-started (older) request's response comes back after.
    const guard = new RequestGuard();

    const firstToken = guard.next(); // first request starts
    const secondToken = guard.next(); // second request starts before first resolves

    // Second (newer) request's callback fires first.
    expect(guard.isCurrent(secondToken)).toBe(true);

    // First (older) request's callback fires afterward and must be
    // recognized as stale, since a newer token was issued after it started.
    expect(guard.isCurrent(firstToken)).toBe(false);
  });
});
