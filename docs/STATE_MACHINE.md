# Dispatch State Machine

## Job states

- pending
- offering
- matched
- failed
- cancelled

## Offer states

- pending
- accepted
- rejected
- expired
- cancelled

## Lifecycle outline

1. Job is created by dispatcher.
2. Matching selects candidate drivers from spatial index.
3. Search begins at `initial_radius_km` and expands in +2 km steps up to `max_radius_km`.
4. Offers are sent with timeout.
5. If first acceptance arrives, job transitions to matched.
6. Remaining pending offers for the same job are immediately marked `cancelled`.
7. If timeout or rejection occurs for all candidates, next batch begins until max radius is reached.

Detailed timer wheel and arbitration logic will be implemented in dispatch phase.
