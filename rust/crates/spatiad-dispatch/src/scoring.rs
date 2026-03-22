/// Weights controlling how each factor contributes to the candidate score.
/// All weights must be non-negative; they do not need to sum to 1.0.
#[derive(Debug, Clone)]
pub struct ScoringWeights {
    /// Reward for proximity — higher weight means closer drivers are strongly preferred.
    pub distance: f32,
    /// Reward for high driver rating (1.0–5.0 scale).
    pub rating: f32,
    /// Penalty for drivers that already have pending offers (busy drivers).
    pub workload: f32,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            distance: 0.5,
            rating: 0.3,
            workload: 0.2,
        }
    }
}

/// Top-level scoring configuration passed to `DispatchService`.
#[derive(Debug, Clone, Default)]
pub struct ScoringConfig {
    pub weights: ScoringWeights,
}

/// Compute a composite score for a single candidate driver.
///
/// Returns a value in `[0.0, 1.0]` where higher is better.
///
/// # Arguments
/// * `distance_km` – straight-line distance from pickup to driver
/// * `radius_km`   – current search radius (used to normalise distance)
/// * `rating`      – driver rating on a 1.0–5.0 scale
/// * `workload`    – number of offers currently pending for this driver
/// * `weights`     – factor weights
pub fn score_candidate(
    distance_km: f64,
    radius_km: f64,
    rating: f32,
    workload: f32,
    weights: &ScoringWeights,
) -> f32 {
    // Proximity score: 1.0 when at pickup, 0.0 at the edge of the radius.
    let proximity = if radius_km > 0.0 {
        (1.0 - (distance_km / radius_km).min(1.0)) as f32
    } else {
        1.0
    };

    // Normalised rating score: 0.0 for rating=1.0, 1.0 for rating=5.0.
    let rating_score = ((rating - 1.0) / 4.0).clamp(0.0, 1.0);

    // Workload penalty: 1.0 when idle, approaches 0.0 with many pending offers.
    let workload_score = 1.0 / (1.0 + workload);

    let total_weight = weights.distance + weights.rating + weights.workload;
    if total_weight == 0.0 {
        return proximity; // sensible fallback
    }

    (weights.distance * proximity
        + weights.rating * rating_score
        + weights.workload * workload_score)
        / total_weight
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closer_driver_scores_higher() {
        let w = ScoringWeights::default();
        let close = score_candidate(0.5, 5.0, 4.5, 0.0, &w);
        let far = score_candidate(4.5, 5.0, 4.5, 0.0, &w);
        assert!(close > far, "close={close} far={far}");
    }

    #[test]
    fn higher_rating_scores_higher_at_same_distance() {
        let w = ScoringWeights::default();
        let good = score_candidate(2.0, 5.0, 5.0, 0.0, &w);
        let bad = score_candidate(2.0, 5.0, 1.0, 0.0, &w);
        assert!(good > bad, "good={good} bad={bad}");
    }

    #[test]
    fn idle_driver_scores_higher_than_busy() {
        let w = ScoringWeights::default();
        let idle = score_candidate(2.0, 5.0, 4.0, 0.0, &w);
        let busy = score_candidate(2.0, 5.0, 4.0, 3.0, &w);
        assert!(idle > busy, "idle={idle} busy={busy}");
    }

    #[test]
    fn score_is_in_unit_range() {
        let w = ScoringWeights::default();
        for (dist, rating, workload) in [(0.0, 5.0, 0.0), (5.0, 1.0, 10.0), (2.5, 3.0, 1.0)] {
            let s = score_candidate(dist, 5.0, rating, workload, &w);
            assert!(s >= 0.0 && s <= 1.0, "score={s} out of range");
        }
    }
}
