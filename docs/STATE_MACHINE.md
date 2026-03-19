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
3. Offers are sent with timeout.
4. If first acceptance arrives, job transitions to matched.
5. If timeout or rejection occurs for all candidates, radius expands and next batch begins.

Detailed timer wheel and arbitration logic will be implemented in dispatch phase.
