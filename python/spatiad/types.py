"""Data types for the Spatiad Python SDK."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Literal, Optional


DriverStatus = Literal["Available", "Offline", "Busy"]

JobDispatchState = Literal["unknown", "pending", "searching", "matched", "exhausted"]

JobEventKind = Literal[
    "job_registered",
    "job_cancelled",
    "webhook_delivery_failed",
    "offer_created",
    "offer_expired",
    "offer_cancelled",
    "offer_rejected",
    "offer_accepted",
    "match_confirmed",
    "offer_status_updated",
]


@dataclass(frozen=True)
class Coordinates:
    latitude: float
    longitude: float


@dataclass(frozen=True)
class RetryOptions:
    max_attempts: int = 3
    backoff_seconds: float = 0.15
    max_backoff_seconds: float = 2.0
    retry_on_statuses: List[int] = field(
        default_factory=lambda: [408, 429, 500, 502, 503, 504]
    )


@dataclass(frozen=True)
class JobStatusResponse:
    job_id: str
    state: JobDispatchState
    matched_driver_id: Optional[str]
    matched_offer_id: Optional[str]


@dataclass(frozen=True)
class JobEvent:
    at: str
    kind: JobEventKind
    offer_id: Optional[str]
    driver_id: Optional[str]
    status: Optional[str]


@dataclass(frozen=True)
class JobEventsResponse:
    job_id: str
    events: List[JobEvent]
    next_cursor: Optional[str]
    next_before_cursor: Optional[str]
