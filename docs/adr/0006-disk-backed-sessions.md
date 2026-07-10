# ADR 0006: Disk-backed Session Store

## Status

Accepted

## Context

Sessions are Discord threads that may span long coding work. A Raspberry Pi bot will restart (deploys, power, crashes). Memory-only Sessions would force users to re-establish context manually even though Discord still shows the thread.

## Decision

Persist each Session’s transcript and Read Snapshots on disk, keyed by Discord thread id (pi-inspired append-only or equivalent under a Saku data dir).

## Consequences

- Restarts resume the same Harness context for a thread.
- Need retention/compaction policy later if transcripts grow large.
- Data dir layout and format are implementation details (not fixed here beyond “durable per thread”).
