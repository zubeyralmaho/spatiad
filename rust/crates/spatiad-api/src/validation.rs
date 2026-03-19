use spatiad_types::Coordinates;

#[derive(Debug)]
pub enum ValidationError {
    InvalidRadius(String),
    InvalidCategory(String),
    InvalidCoordinates(String),
    EmptyCategory,
}

impl ValidationError {
    pub fn message(&self) -> String {
        match self {
            Self::InvalidRadius(msg) => msg.clone(),
            Self::InvalidCategory(msg) => msg.clone(),
            Self::InvalidCoordinates(msg) => msg.clone(),
            Self::EmptyCategory => "category cannot be empty".to_string(),
        }
    }
}

pub fn validate_radius(initial_km: f64, max_km: f64) -> Result<(), ValidationError> {
    if initial_km.is_nan() || initial_km.is_infinite() {
        return Err(ValidationError::InvalidRadius(
            "initial_radius_km must be a valid number".to_string(),
        ));
    }

    if max_km.is_nan() || max_km.is_infinite() {
        return Err(ValidationError::InvalidRadius(
            "max_radius_km must be a valid number".to_string(),
        ));
    }

    if initial_km < 0.0 {
        return Err(ValidationError::InvalidRadius(
            "initial_radius_km must be >= 0".to_string(),
        ));
    }

    if max_km < 0.0 {
        return Err(ValidationError::InvalidRadius(
            "max_radius_km must be >= 0".to_string(),
        ));
    }

    if initial_km > max_km {
        return Err(ValidationError::InvalidRadius(
            "initial_radius_km must be <= max_radius_km".to_string(),
        ));
    }

    if max_km > 1000.0 {
        return Err(ValidationError::InvalidRadius(
            "max_radius_km must be <= 1000 km".to_string(),
        ));
    }

    Ok(())
}

pub fn validate_category(category: &str) -> Result<(), ValidationError> {
    if category.is_empty() {
        return Err(ValidationError::EmptyCategory);
    }

    if category.len() > 50 {
        return Err(ValidationError::InvalidCategory(
            "category must be <= 50 characters".to_string(),
        ));
    }

    // Check for valid characters (alphanumeric, underscore, hyphen)
    if !category
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ValidationError::InvalidCategory(
            "category can only contain alphanumeric characters, hyphens, and underscores"
                .to_string(),
        ));
    }

    Ok(())
}

pub fn validate_coordinates(coords: &Coordinates) -> Result<(), ValidationError> {
    let lat = coords.latitude;
    let lng = coords.longitude;

    if !(-90.0..=90.0).contains(&lat) {
        return Err(ValidationError::InvalidCoordinates(
            format!("latitude must be between -90 and 90, got {}", lat),
        ));
    }

    if !(-180.0..=180.0).contains(&lng) {
        return Err(ValidationError::InvalidCoordinates(
            format!("longitude must be between -180 and 180, got {}", lng),
        ));
    }

    if !lat.is_finite() || !lng.is_finite() {
        return Err(ValidationError::InvalidCoordinates(
            "coordinates must be finite numbers".to_string(),
        ));
    }

    Ok(())
}

#[allow(dead_code)]
pub fn validate_h3_resolution(resolution: u8) -> Result<(), ValidationError> {
    if resolution > 15 {
        return Err(ValidationError::InvalidRadius(format!(
            "H3 resolution must be 0-15, got {}",
            resolution
        )));
    }
    Ok(())
}

pub fn validate_timeout_seconds(timeout: u64) -> Result<(), ValidationError> {
    if timeout == 0 {
        return Err(ValidationError::InvalidRadius(
            "timeout_seconds must be > 0".to_string(),
        ));
    }

    if timeout > 3600 {
        return Err(ValidationError::InvalidRadius(
            "timeout_seconds must be <= 3600 (1 hour)".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_radius_valid() {
        assert!(validate_radius(1.0, 5.0).is_ok());
        assert!(validate_radius(0.0, 0.0).is_ok());
        assert!(validate_radius(0.1, 100.0).is_ok());
    }

    #[test]
    fn test_validate_radius_invalid() {
        assert!(validate_radius(-1.0, 5.0).is_err());
        assert!(validate_radius(5.0, 1.0).is_err());
        assert!(validate_radius(0.0, 1001.0).is_err());
    }

    #[test]
    fn test_validate_category_valid() {
        assert!(validate_category("tow_truck").is_ok());
        assert!(validate_category("delivery-express").is_ok());
        assert!(validate_category("a").is_ok());
    }

    #[test]
    fn test_validate_category_invalid() {
        assert!(validate_category("").is_err());
        assert!(validate_category("invalid@category").is_err());
        assert!(validate_category("a".repeat(51).as_str()).is_err());
    }

    #[test]
    fn test_validate_coordinates_valid() {
        assert!(validate_coordinates(&Coordinates {
            latitude: 38.433,
            longitude: 26.768,
        })
        .is_ok());
        assert!(validate_coordinates(&Coordinates {
            latitude: 90.0,
            longitude: 180.0,
        })
        .is_ok());
        assert!(validate_coordinates(&Coordinates {
            latitude: -90.0,
            longitude: -180.0,
        })
        .is_ok());
    }

    #[test]
    fn test_validate_coordinates_invalid() {
        assert!(validate_coordinates(&Coordinates {
            latitude: 91.0,
            longitude: 26.768,
        })
        .is_err());
        assert!(validate_coordinates(&Coordinates {
            latitude: 38.433,
            longitude: 181.0,
        })
        .is_err());
    }

    #[test]
    fn test_validate_timeout_valid() {
        assert!(validate_timeout_seconds(1).is_ok());
        assert!(validate_timeout_seconds(20).is_ok());
        assert!(validate_timeout_seconds(3600).is_ok());
    }

    #[test]
    fn test_validate_timeout_invalid() {
        assert!(validate_timeout_seconds(0).is_err());
        assert!(validate_timeout_seconds(3601).is_err());
    }

    #[test]
    fn test_validate_h3_resolution() {
        assert!(validate_h3_resolution(8).is_ok());
        assert!(validate_h3_resolution(0).is_ok());
        assert!(validate_h3_resolution(15).is_ok());
        assert!(validate_h3_resolution(16).is_err());
    }
}
