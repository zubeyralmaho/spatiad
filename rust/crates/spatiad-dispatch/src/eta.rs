//! ETA provider trait and built-in implementations.
//!
//! An `EtaProvider` estimates how long it will take a driver at position
//! `from` to reach a pickup at position `to`. The result is in **seconds**.
//!
//! Two providers ship out of the box:
//!
//! * [`StraightLineEtaProvider`] — uses the Haversine formula and a
//!   configurable average speed. Zero external dependencies; suitable for
//!   production when road-network accuracy is not required.
//!
//! * [`OsrmEtaProvider`] — calls an [OSRM](http://project-osrm.org/) routing
//!   engine over HTTP. Falls back to straight-line ETA when the request fails
//!   so dispatch is never blocked by a routing outage.

use std::fmt;

/// Latitude / longitude pair used by ETA providers.
#[derive(Debug, Clone, Copy)]
pub struct LatLon {
    pub latitude: f64,
    pub longitude: f64,
}

/// An object that estimates the travel time (in seconds) between two points.
pub trait EtaProvider: Send + Sync + fmt::Debug {
    /// Return the estimated travel time in seconds from `from` to `to`.
    fn estimate_secs(&self, from: LatLon, to: LatLon) -> f64;
}

// ─── Straight-line provider ───────────────────────────────────────────────────

/// ETA estimation based on straight-line (Haversine) distance and a fixed
/// average speed.
///
/// `ETA = distance_km / average_speed_kmh * 3600`
#[derive(Debug, Clone)]
pub struct StraightLineEtaProvider {
    /// Assumed average speed in km/h (default: 30).
    pub average_speed_kmh: f64,
}

impl Default for StraightLineEtaProvider {
    fn default() -> Self {
        Self {
            average_speed_kmh: 30.0,
        }
    }
}

impl EtaProvider for StraightLineEtaProvider {
    fn estimate_secs(&self, from: LatLon, to: LatLon) -> f64 {
        let dist = haversine_km(from, to);
        let speed = self.average_speed_kmh.max(0.1);
        (dist / speed) * 3600.0
    }
}

// ─── OSRM provider ────────────────────────────────────────────────────────────

/// ETA estimation using an OSRM routing engine.
///
/// Performs a synchronous blocking HTTP GET against the OSRM Route API
/// (`/route/v1/driving/{lon},{lat};{lon},{lat}?overview=false`). The request
/// times out after `timeout_secs` and falls back to straight-line ETA on any
/// error so dispatch is never blocked by a routing outage.
#[derive(Debug)]
pub struct OsrmEtaProvider {
    /// Base URL of the OSRM instance, e.g. `http://router.project-osrm.org`.
    pub base_url: String,
    /// Timeout for each OSRM HTTP request in seconds (default: 2).
    pub timeout_secs: u64,
    /// Fallback used when OSRM is unavailable.
    pub fallback: StraightLineEtaProvider,
}

impl OsrmEtaProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            timeout_secs: 2,
            fallback: StraightLineEtaProvider::default(),
        }
    }

    /// Attempt to query OSRM. Returns `None` if the request fails.
    fn query_osrm(&self, from: LatLon, to: LatLon) -> Option<f64> {
        let url = format!(
            "{}/route/v1/driving/{},{};{},{}?overview=false",
            self.base_url, from.longitude, from.latitude, to.longitude, to.latitude,
        );

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .build()
            .ok()?;

        let body: serde_json::Value = client.get(&url).send().ok()?.json().ok()?;

        body["routes"][0]["duration"].as_f64()
    }
}

impl EtaProvider for OsrmEtaProvider {
    fn estimate_secs(&self, from: LatLon, to: LatLon) -> f64 {
        self.query_osrm(from, to)
            .unwrap_or_else(|| self.fallback.estimate_secs(from, to))
    }
}

// ─── Haversine helper ─────────────────────────────────────────────────────────

fn haversine_km(a: LatLon, b: LatLon) -> f64 {
    const R: f64 = 6371.0;
    let d_lat = (b.latitude - a.latitude).to_radians();
    let d_lon = (b.longitude - a.longitude).to_radians();
    let lat_a = a.latitude.to_radians();
    let lat_b = b.latitude.to_radians();
    let h = (d_lat / 2.0).sin().powi(2) + lat_a.cos() * lat_b.cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn izmir() -> LatLon {
        LatLon { latitude: 38.433, longitude: 26.768 }
    }

    fn izmir_north() -> LatLon {
        LatLon { latitude: 38.443, longitude: 26.768 }
    }

    #[test]
    fn straight_line_eta_is_positive() {
        let provider = StraightLineEtaProvider::default();
        let eta = provider.estimate_secs(izmir(), izmir_north());
        assert!(eta > 0.0, "ETA should be positive, got {eta}");
    }

    #[test]
    fn same_location_gives_zero_eta() {
        let provider = StraightLineEtaProvider::default();
        let eta = provider.estimate_secs(izmir(), izmir());
        assert!(eta < 0.01, "ETA for same location should be ~0, got {eta}");
    }

    #[test]
    fn faster_speed_reduces_eta() {
        let slow = StraightLineEtaProvider { average_speed_kmh: 10.0 };
        let fast = StraightLineEtaProvider { average_speed_kmh: 100.0 };
        let eta_slow = slow.estimate_secs(izmir(), izmir_north());
        let eta_fast = fast.estimate_secs(izmir(), izmir_north());
        assert!(eta_fast < eta_slow, "faster speed should reduce ETA");
    }
}
