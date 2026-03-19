use std::collections::{HashMap, HashSet};

use h3o::{LatLng, Resolution};
use spatiad_types::Coordinates;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct SpatialIndex {
    resolution: u8,
    buckets: HashMap<String, HashSet<Uuid>>,
    driver_cells: HashMap<Uuid, String>,
}

impl SpatialIndex {
    pub fn new(resolution: u8) -> Self {
        Self {
            resolution,
            buckets: HashMap::new(),
            driver_cells: HashMap::new(),
        }
    }

    pub fn upsert_driver(&mut self, driver_id: Uuid, point: Coordinates) {
        if let Some(old_cell) = self.driver_cells.remove(&driver_id) {
            if let Some(drivers) = self.buckets.get_mut(&old_cell) {
                drivers.remove(&driver_id);
                if drivers.is_empty() {
                    self.buckets.remove(&old_cell);
                }
            }
        }

        let cell = self.cell_key(point);
        self.buckets.entry(cell.clone()).or_default().insert(driver_id);
        self.driver_cells.insert(driver_id, cell);
    }

    pub fn remove_driver(&mut self, driver_id: Uuid) {
        if let Some(cell) = self.driver_cells.remove(&driver_id) {
            if let Some(drivers) = self.buckets.get_mut(&cell) {
                drivers.remove(&driver_id);
                if drivers.is_empty() {
                    self.buckets.remove(&cell);
                }
            }
        }
    }

    pub fn candidates_in_same_cell(&self, point: Coordinates) -> Vec<Uuid> {
        let key = self.cell_key(point);
        self.buckets
            .get(&key)
            .map(|drivers| drivers.iter().copied().collect())
            .unwrap_or_default()
    }

    fn cell_key(&self, point: Coordinates) -> String {
        let resolution = Resolution::try_from(self.resolution).unwrap_or(Resolution::Eight);
        let fallback = format!("{}:{:.5}:{:.5}", self.resolution, point.latitude, point.longitude);

        let lat_lng = match LatLng::new(point.latitude, point.longitude) {
            Ok(value) => value,
            Err(_) => return fallback,
        };

        let cell = lat_lng.to_cell(resolution);
        cell.to_string()
    }
}
