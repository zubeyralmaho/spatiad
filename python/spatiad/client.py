"""Sync and async HTTP clients for the Spatiad spatial dispatch engine."""

from __future__ import annotations

import time
from typing import Any, Dict, List, Optional, Sequence

import httpx

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

_DEFAULT_RETRY = RetryOptions()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _auth_headers(token: Optional[str]) -> Dict[str, str]:
    if token is None:
        return {}
    return {"Authorization": f"Bearer {token}"}


def _parse_error(response: httpx.Response, prefix: str) -> SpatiadApiError:
    body: Optional[Dict[str, Any]] = None
    try:
        body = response.json()
    except Exception:
        pass

    message = None
    code = None
    if isinstance(body, dict):
        message = body.get("message")
        code = body.get("error")

    return SpatiadApiError(
        status=response.status_code,
        code=code,
        message=message or f"{prefix} with status {response.status_code}",
        details=body if isinstance(body, dict) else None,
    )


def _job_status_from_dict(data: Dict[str, Any]) -> JobStatusResponse:
    return JobStatusResponse(
        job_id=data["job_id"],
        state=data["state"],
        matched_driver_id=data.get("matched_driver_id"),
        matched_offer_id=data.get("matched_offer_id"),
    )


def _job_events_from_dict(data: Dict[str, Any]) -> JobEventsResponse:
    events = [
        JobEvent(
            at=e["at"],
            kind=e["kind"],
            offer_id=e.get("offer_id"),
            driver_id=e.get("driver_id"),
            status=e.get("status"),
        )
        for e in data.get("events", [])
    ]
    return JobEventsResponse(
        job_id=data["job_id"],
        events=events,
        next_cursor=data.get("next_cursor"),
        next_before_cursor=data.get("next_before_cursor"),
    )


def _events_query_params(
    limit: Optional[int],
    cursor: Optional[str],
    kinds: Optional[Sequence[JobEventKind]],
) -> Dict[str, str]:
    params: Dict[str, str] = {}
    if limit is not None:
        params["limit"] = str(limit)
    if cursor is not None:
        params["cursor"] = cursor
    if kinds:
        params["kinds"] = ",".join(kinds)
    return params


def _backoff_seconds(attempt: int, opts: RetryOptions) -> float:
    delay = opts.backoff_seconds * (2 ** attempt)
    return min(delay, opts.max_backoff_seconds)


# ---------------------------------------------------------------------------
# Synchronous client
# ---------------------------------------------------------------------------

class SpatiadClient:
    """Synchronous client for the Spatiad REST API."""

    def __init__(
        self,
        base_url: str,
        token: Optional[str] = None,
        timeout: float = 30.0,
        retry: Optional[RetryOptions] = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._token = token
        self._retry = retry or _DEFAULT_RETRY
        self._client = httpx.Client(
            base_url=self._base_url,
            headers=_auth_headers(token),
            timeout=timeout,
        )

    # -- lifecycle -----------------------------------------------------------

    def close(self) -> None:
        self._client.close()

    def __enter__(self) -> "SpatiadClient":
        return self

    def __exit__(self, *args: Any) -> None:
        self.close()

    # -- dispatch ------------------------------------------------------------

    def create_offer(
        self,
        job_id: str,
        category: str,
        pickup: Coordinates,
        dropoff: Optional[Coordinates] = None,
        initial_radius_km: float = 3.0,
        max_radius_km: float = 10.0,
        timeout_seconds: int = 30,
        retry: Optional[RetryOptions] = None,
    ) -> Dict[str, str]:
        body: Dict[str, Any] = {
            "job_id": job_id,
            "category": category,
            "pickup": {"latitude": pickup.latitude, "longitude": pickup.longitude},
            "initial_radius_km": initial_radius_km,
            "max_radius_km": max_radius_km,
            "timeout_seconds": timeout_seconds,
        }
        if dropoff is not None:
            body["dropoff"] = {"latitude": dropoff.latitude, "longitude": dropoff.longitude}

        resp = self._request_with_retry("POST", "/api/v1/dispatch/offer", json=body, retry=retry)
        return resp.json()  # type: ignore[no-any-return]

    def upsert_driver(
        self,
        driver_id: str,
        category: str,
        status: DriverStatus,
        position: Coordinates,
        retry: Optional[RetryOptions] = None,
    ) -> None:
        body = {
            "driver_id": driver_id,
            "category": category,
            "status": status,
            "position": {"latitude": position.latitude, "longitude": position.longitude},
        }
        self._request_with_retry("POST", "/api/v1/driver/upsert", json=body, retry=retry)

    def cancel_offer(
        self,
        offer_id: str,
        retry: Optional[RetryOptions] = None,
    ) -> None:
        self._request_with_retry(
            "POST", "/api/v1/dispatch/cancel", json={"offer_id": offer_id}, retry=retry,
        )

    def cancel_job(
        self,
        job_id: str,
        retry: Optional[RetryOptions] = None,
    ) -> None:
        self._request_with_retry(
            "POST", "/api/v1/dispatch/job/cancel", json={"job_id": job_id}, retry=retry,
        )

    def get_job_status(
        self,
        job_id: str,
        retry: Optional[RetryOptions] = None,
    ) -> JobStatusResponse:
        resp = self._request_with_retry("GET", f"/api/v1/dispatch/job/{job_id}", retry=retry)
        return _job_status_from_dict(resp.json())

    def get_job_events(
        self,
        job_id: str,
        limit: Optional[int] = None,
        cursor: Optional[str] = None,
        kinds: Optional[Sequence[JobEventKind]] = None,
        retry: Optional[RetryOptions] = None,
    ) -> JobEventsResponse:
        params = _events_query_params(limit, cursor, kinds)
        resp = self._request_with_retry(
            "GET", f"/api/v1/dispatch/job/{job_id}/events", params=params, retry=retry,
        )
        return _job_events_from_dict(resp.json())

    # -- internal ------------------------------------------------------------

    def _request_with_retry(
        self,
        method: str,
        path: str,
        retry: Optional[RetryOptions] = None,
        **kwargs: Any,
    ) -> httpx.Response:
        opts = retry or self._retry
        last_exc: Optional[Exception] = None

        for attempt in range(opts.max_attempts):
            try:
                resp = self._client.request(method, path, **kwargs)
            except httpx.TransportError as exc:
                last_exc = exc
                if attempt < opts.max_attempts - 1:
                    time.sleep(_backoff_seconds(attempt, opts))
                    continue
                raise

            if resp.is_success:
                return resp

            if resp.status_code not in opts.retry_on_statuses or attempt == opts.max_attempts - 1:
                raise _parse_error(resp, f"{method} {path} failed")

            time.sleep(_backoff_seconds(attempt, opts))

        # Should not be reached, but satisfies the type checker.
        raise last_exc or SpatiadApiError(status=0, message="request failed after retries")


# ---------------------------------------------------------------------------
# Asynchronous client
# ---------------------------------------------------------------------------

class AsyncSpatiadClient:
    """Asynchronous client for the Spatiad REST API."""

    def __init__(
        self,
        base_url: str,
        token: Optional[str] = None,
        timeout: float = 30.0,
        retry: Optional[RetryOptions] = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._token = token
        self._retry = retry or _DEFAULT_RETRY
        self._client = httpx.AsyncClient(
            base_url=self._base_url,
            headers=_auth_headers(token),
            timeout=timeout,
        )

    # -- lifecycle -----------------------------------------------------------

    async def close(self) -> None:
        await self._client.aclose()

    async def __aenter__(self) -> "AsyncSpatiadClient":
        return self

    async def __aexit__(self, *args: Any) -> None:
        await self.close()

    # -- dispatch ------------------------------------------------------------

    async def create_offer(
        self,
        job_id: str,
        category: str,
        pickup: Coordinates,
        dropoff: Optional[Coordinates] = None,
        initial_radius_km: float = 3.0,
        max_radius_km: float = 10.0,
        timeout_seconds: int = 30,
        retry: Optional[RetryOptions] = None,
    ) -> Dict[str, str]:
        body: Dict[str, Any] = {
            "job_id": job_id,
            "category": category,
            "pickup": {"latitude": pickup.latitude, "longitude": pickup.longitude},
            "initial_radius_km": initial_radius_km,
            "max_radius_km": max_radius_km,
            "timeout_seconds": timeout_seconds,
        }
        if dropoff is not None:
            body["dropoff"] = {"latitude": dropoff.latitude, "longitude": dropoff.longitude}

        resp = await self._request_with_retry("POST", "/api/v1/dispatch/offer", json=body, retry=retry)
        return resp.json()  # type: ignore[no-any-return]

    async def upsert_driver(
        self,
        driver_id: str,
        category: str,
        status: DriverStatus,
        position: Coordinates,
        retry: Optional[RetryOptions] = None,
    ) -> None:
        body = {
            "driver_id": driver_id,
            "category": category,
            "status": status,
            "position": {"latitude": position.latitude, "longitude": position.longitude},
        }
        await self._request_with_retry("POST", "/api/v1/driver/upsert", json=body, retry=retry)

    async def cancel_offer(
        self,
        offer_id: str,
        retry: Optional[RetryOptions] = None,
    ) -> None:
        await self._request_with_retry(
            "POST", "/api/v1/dispatch/cancel", json={"offer_id": offer_id}, retry=retry,
        )

    async def cancel_job(
        self,
        job_id: str,
        retry: Optional[RetryOptions] = None,
    ) -> None:
        await self._request_with_retry(
            "POST", "/api/v1/dispatch/job/cancel", json={"job_id": job_id}, retry=retry,
        )

    async def get_job_status(
        self,
        job_id: str,
        retry: Optional[RetryOptions] = None,
    ) -> JobStatusResponse:
        resp = await self._request_with_retry("GET", f"/api/v1/dispatch/job/{job_id}", retry=retry)
        return _job_status_from_dict(resp.json())

    async def get_job_events(
        self,
        job_id: str,
        limit: Optional[int] = None,
        cursor: Optional[str] = None,
        kinds: Optional[Sequence[JobEventKind]] = None,
        retry: Optional[RetryOptions] = None,
    ) -> JobEventsResponse:
        params = _events_query_params(limit, cursor, kinds)
        resp = await self._request_with_retry(
            "GET", f"/api/v1/dispatch/job/{job_id}/events", params=params, retry=retry,
        )
        return _job_events_from_dict(resp.json())

    # -- internal ------------------------------------------------------------

    async def _request_with_retry(
        self,
        method: str,
        path: str,
        retry: Optional[RetryOptions] = None,
        **kwargs: Any,
    ) -> httpx.Response:
        import asyncio

        opts = retry or self._retry
        last_exc: Optional[Exception] = None

        for attempt in range(opts.max_attempts):
            try:
                resp = await self._client.request(method, path, **kwargs)
            except httpx.TransportError as exc:
                last_exc = exc
                if attempt < opts.max_attempts - 1:
                    await asyncio.sleep(_backoff_seconds(attempt, opts))
                    continue
                raise

            if resp.is_success:
                return resp

            if resp.status_code not in opts.retry_on_statuses or attempt == opts.max_attempts - 1:
                raise _parse_error(resp, f"{method} {path} failed")

            await asyncio.sleep(_backoff_seconds(attempt, opts))

        raise last_exc or SpatiadApiError(status=0, message="request failed after retries")
