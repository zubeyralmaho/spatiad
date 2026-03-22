//! `spatiad-zones` — geographic zone definitions and enforcement for Spatiad.
//!
//! A **zone** is a named polygon that classifies a geographic area. Three zone
//! types are supported:
//!
//! * [`ZoneType::ServiceArea`] — dispatch is only allowed *inside* service areas
//!   (when at least one service area exists). A pickup outside all service areas
//!   is rejected.
//! * [`ZoneType::SurgeZone`] — pickups inside a surge zone carry a multiplier
//!   (e.g. priority boost or fare multiplier) returned to the caller.
//! * [`ZoneType::RestrictedZone`] — pickups inside a restricted zone are always
//!   rejected regardless of any service area.
//!
//! ## Point-in-polygon
//!
//! Containment is tested with the standard **ray-casting** algorithm (even–odd
//! rule). The implementation handles crossing edges and degenerate cases
//! correctly for convex and concave polygons.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A geographic coordinate (latitude / longitude in decimal degrees).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub latitude: f64,
    pub longitude: f64,
}

/// Classification of a zone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ZoneType {
    /// Pickup must be inside at least one service area (when any exist).
    ServiceArea,
    /// Pickup is allowed but the zone carries a multiplier.
    SurgeZone { multiplier: u32 },
    /// Pickup is never allowed inside this zone.
    RestrictedZone,
}

/// A named, typed geographic polygon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    pub id: Uuid,
    pub name: String,
    pub zone_type: ZoneType,
    /// Polygon vertices in order (clockwise or counter-clockwise). At least 3
    /// points are required to define a valid polygon.
    pub polygon: Vec<Point>,
}

impl Zone {
    pub fn new(name: impl Into<String>, zone_type: ZoneType, polygon: Vec<Point>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            zone_type,
            polygon,
        }
    }

    /// Return `true` if `point` is inside this zone's polygon.
    ///
    /// Uses the ray-casting (even-odd rule) algorithm.
    pub fn contains(&self, point: Point) -> bool {
        point_in_polygon(point, &self.polygon)
    }
}

/// Result of checking whether a pickup is allowed and what adjustments apply.
#[derive(Debug, Clone, PartialEq)]
pub enum ZoneCheckResult {
    /// Pickup is permitted with no adjustments.
    Allowed,
    /// Pickup is permitted inside a surge zone; the `multiplier` may be applied
    /// to the offer priority or fare estimate by the dispatcher.
    Surge { multiplier: u32 },
    /// Pickup is not permitted (outside all service areas, or in a restricted
    /// zone). The `reason` field describes why.
    Denied { reason: DenialReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenialReason {
    /// The pickup is inside a restricted zone.
    RestrictedZone { zone_id: Uuid, zone_name: String },
    /// The pickup is outside every defined service area.
    OutsideServiceArea,
}

/// Registry of all defined zones. Thread-safe to clone; intended to be shared
/// via `Arc` in `ApiState`.
#[derive(Debug, Clone, Default)]
pub struct ZoneRegistry {
    zones: Vec<Zone>,
}

impl ZoneRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a zone to the registry.
    pub fn add_zone(&mut self, zone: Zone) {
        self.zones.push(zone);
    }

    /// Remove a zone by ID. Returns `true` if a zone was removed.
    pub fn remove_zone(&mut self, id: Uuid) -> bool {
        let before = self.zones.len();
        self.zones.retain(|z| z.id != id);
        self.zones.len() < before
    }

    /// Return an iterator over all zones.
    pub fn zones(&self) -> impl Iterator<Item = &Zone> {
        self.zones.iter()
    }

    /// Return all zones whose polygon contains `point`.
    pub fn zones_containing(&self, point: Point) -> Vec<&Zone> {
        self.zones.iter().filter(|z| z.contains(point)).collect()
    }

    /// Evaluate whether a pickup at `point` is allowed given the current set of
    /// zones. Restricted zones take priority over everything; service-area
    /// enforcement only activates when at least one service area is defined;
    /// surge multipliers are returned for the *highest* multiplier that applies.
    pub fn check(&self, point: Point) -> ZoneCheckResult {
        let containing = self.zones_containing(point);

        // 1. Restricted zones are an immediate denial.
        for zone in &containing {
            if zone.zone_type == ZoneType::RestrictedZone {
                return ZoneCheckResult::Denied {
                    reason: DenialReason::RestrictedZone {
                        zone_id: zone.id,
                        zone_name: zone.name.clone(),
                    },
                };
            }
        }

        // 2. Service-area enforcement — only when at least one service area is
        //    registered globally.
        let has_service_areas = self
            .zones
            .iter()
            .any(|z| z.zone_type == ZoneType::ServiceArea);

        if has_service_areas {
            let inside_service_area = containing
                .iter()
                .any(|z| z.zone_type == ZoneType::ServiceArea);

            if !inside_service_area {
                return ZoneCheckResult::Denied {
                    reason: DenialReason::OutsideServiceArea,
                };
            }
        }

        // 3. Return the highest surge multiplier, if any.
        let max_surge = containing.iter().filter_map(|z| {
            if let ZoneType::SurgeZone { multiplier } = z.zone_type {
                Some(multiplier)
            } else {
                None
            }
        }).max();

        if let Some(multiplier) = max_surge {
            return ZoneCheckResult::Surge { multiplier };
        }

        ZoneCheckResult::Allowed
    }
}

// ─── Point-in-polygon (ray casting) ──────────────────────────────────────────

/// Returns `true` if `point` is inside the polygon defined by `vertices`.
///
/// The algorithm fires a ray from `point` in the +x (east) direction and counts
/// how many polygon edges it crosses. An odd count means inside.
///
/// Edge cases handled:
/// - Polygons with fewer than 3 vertices → always `false`.
/// - The polygon is automatically "closed" (last vertex connects to first).
pub fn point_in_polygon(point: Point, vertices: &[Point]) -> bool {
    let n = vertices.len();
    if n < 3 {
        return false;
    }

    let (px, py) = (point.longitude, point.latitude);
    let mut inside = false;

    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (vertices[i].longitude, vertices[i].latitude);
        let (xj, yj) = (vertices[j].longitude, vertices[j].latitude);

        // Check if the horizontal ray from (px, py) crosses edge (xj,yj)-(xi,yi).
        let crosses = ((yi > py) != (yj > py))
            && (px < (xj - xi) * (py - yi) / (yj - yi) + xi);
        if crosses {
            inside = !inside;
        }
        j = i;
    }

    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> Vec<Point> {
        vec![
            Point { latitude: min_lat, longitude: min_lon },
            Point { latitude: max_lat, longitude: min_lon },
            Point { latitude: max_lat, longitude: max_lon },
            Point { latitude: min_lat, longitude: max_lon },
        ]
    }

    #[test]
    fn point_inside_square() {
        let poly = square(0.0, 10.0, 0.0, 10.0);
        assert!(point_in_polygon(Point { latitude: 5.0, longitude: 5.0 }, &poly));
    }

    #[test]
    fn point_outside_square() {
        let poly = square(0.0, 10.0, 0.0, 10.0);
        assert!(!point_in_polygon(Point { latitude: 15.0, longitude: 5.0 }, &poly));
    }

    #[test]
    fn restricted_zone_denies_pickup() {
        let mut reg = ZoneRegistry::new();
        reg.add_zone(Zone::new(
            "downtown_restricted",
            ZoneType::RestrictedZone,
            square(38.4, 38.5, 26.7, 26.8),
        ));

        let inside = Point { latitude: 38.45, longitude: 26.75 };
        match reg.check(inside) {
            ZoneCheckResult::Denied {
                reason: DenialReason::RestrictedZone { .. },
            } => {}
            other => panic!("expected denial, got {other:?}"),
        }
    }

    #[test]
    fn outside_service_area_denies_pickup() {
        let mut reg = ZoneRegistry::new();
        reg.add_zone(Zone::new(
            "izmir_center",
            ZoneType::ServiceArea,
            square(38.4, 38.5, 26.7, 26.8),
        ));

        let outside = Point { latitude: 39.0, longitude: 27.0 };
        assert_eq!(
            reg.check(outside),
            ZoneCheckResult::Denied {
                reason: DenialReason::OutsideServiceArea
            }
        );
    }

    #[test]
    fn inside_service_area_allowed() {
        let mut reg = ZoneRegistry::new();
        reg.add_zone(Zone::new(
            "izmir_center",
            ZoneType::ServiceArea,
            square(38.4, 38.5, 26.7, 26.8),
        ));

        let inside = Point { latitude: 38.45, longitude: 26.75 };
        assert_eq!(reg.check(inside), ZoneCheckResult::Allowed);
    }

    #[test]
    fn surge_zone_returns_multiplier() {
        let mut reg = ZoneRegistry::new();
        reg.add_zone(Zone::new(
            "airport_surge",
            ZoneType::SurgeZone { multiplier: 2 },
            square(38.4, 38.5, 26.7, 26.8),
        ));

        let inside = Point { latitude: 38.45, longitude: 26.75 };
        assert_eq!(reg.check(inside), ZoneCheckResult::Surge { multiplier: 2 });
    }

    #[test]
    fn no_zones_always_allowed() {
        let reg = ZoneRegistry::new();
        let point = Point { latitude: 38.45, longitude: 26.75 };
        assert_eq!(reg.check(point), ZoneCheckResult::Allowed);
    }

    #[test]
    fn remove_zone_works() {
        let mut reg = ZoneRegistry::new();
        let zone = Zone::new("test", ZoneType::RestrictedZone, square(0.0, 1.0, 0.0, 1.0));
        let id = zone.id;
        reg.add_zone(zone);
        assert!(reg.remove_zone(id));
        assert!(reg.zones().next().is_none());
    }
}
