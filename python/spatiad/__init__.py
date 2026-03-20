"""Spatiad Python SDK -- lightweight HTTP client for the Spatiad spatial dispatch engine."""

from .client import AsyncSpatiadClient, SpatiadClient
from .errors import SpatiadApiError
from .types import (
    Coordinates,
    DriverStatus,
    JobDispatchState,
    JobEvent,
    JobEventKind,
    JobEventsResponse,
    JobStatusResponse,
    RetryOptions,
)

__all__ = [
    "AsyncSpatiadClient",
    "Coordinates",
    "DriverStatus",
    "JobDispatchState",
    "JobEvent",
    "JobEventKind",
    "JobEventsResponse",
    "JobStatusResponse",
    "RetryOptions",
    "SpatiadApiError",
    "SpatiadClient",
]

__version__ = "0.1.0"
