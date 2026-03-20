"""Error types for the Spatiad Python SDK."""

from __future__ import annotations

from typing import Any, Dict, Optional

_RETRYABLE_STATUSES = frozenset({408, 429, 502, 503, 504})


class SpatiadApiError(Exception):
    """Raised when the Spatiad API returns a non-success response."""

    def __init__(
        self,
        status: int,
        code: Optional[str] = None,
        message: Optional[str] = None,
        retryable: Optional[bool] = None,
        details: Optional[Dict[str, Any]] = None,
    ) -> None:
        self.status = status
        self.code = code
        self.message = message or f"API request failed with status {status}"
        self.retryable = retryable if retryable is not None else (status in _RETRYABLE_STATUSES)
        self.details = details
        super().__init__(self.message)

    def __repr__(self) -> str:
        return (
            f"SpatiadApiError(status={self.status!r}, code={self.code!r}, "
            f"message={self.message!r}, retryable={self.retryable!r})"
        )
