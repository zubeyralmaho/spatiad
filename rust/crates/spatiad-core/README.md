# spatiad-core

In-memory runtime state for active drivers, jobs, and offers.

This first scaffold keeps data in simple maps and is intended to evolve toward:

- lock-safe concurrent access
- event stream support
- offer arbitration and idempotency controls
