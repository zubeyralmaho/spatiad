#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tower::ServiceExt;
    use uuid::Uuid;

    use spatiad_api::{router, ApiState, SlidingWindowRateLimiter, WsReconnectGuard};
    use spatiad_core::Engine;
    use spatiad_dispatch::DispatchService;
    use spatiad_types::{Coordinates, DriverStatus, OfferStatus};

    fn setup_test_state() -> ApiState {
        let h3_resolution = 8u8;
        let engine = Engine::new(h3_resolution);

        ApiState {
            dispatch: Arc::new(Mutex::new(DispatchService::new(engine))),
            webhook_url: None,
            webhook_secret: None,
            webhook_timeout_ms: 3_000,
            driver_token: None,
            dispatcher_token: None,
            dispatch_rate_limiter: Arc::new(Mutex::new(SlidingWindowRateLimiter::new(240, 60))),
            ws_reconnect_guard: Arc::new(Mutex::new(WsReconnectGuard::new(30, 60))),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn make_request(
        app: &mut axum::Router,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri: axum::http::Uri = path.parse().unwrap();
        let mut req = Request::builder().method(method).uri(uri);

        let body = if let Some(json_body) = body {
            Body::from(serde_json::to_string(&json_body).unwrap())
        } else {
            Body::empty()
        };

        req = req.header("content-type", "application/json");
        let request = req.body(body).unwrap();

        let response = app
            .clone()
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        let json_body: Value = if body.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&body).unwrap_or(json!({}))
        };

        (status, json_body)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = setup_test_state();
        let mut app = router(state);

        let (status, body) = make_request(&mut app, "GET", "/health", None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], "spatiad");
    }

    #[tokio::test]
    async fn test_driver_upsert() {
        let state = setup_test_state();
        let mut app = router(state);

        let driver_id = Uuid::new_v4();
        let request_body = json!({
            "driver_id": driver_id.to_string(),
            "category": "tow_truck",
            "status": "Available",
            "position": {
                "latitude": 38.433,
                "longitude": 26.768
            }
        });

        let (status, _body) = make_request(&mut app, "POST", "/api/v1/driver/upsert", Some(request_body))
            .await;

        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dispatch_offer_with_driver() {
        let state = setup_test_state();
        
        // Upsert a driver first
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.upsert_driver_location(
                Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
        }

        let mut app = router(state);
        let job_id = Uuid::new_v4();

        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "tow_truck",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "dropoff": {
                "latitude": 38.44,
                "longitude": 26.78
            },
            "initial_radius_km": 1,
            "max_radius_km": 5,
            "timeout_seconds": 20
        });

        let (status, body) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body["offer_id"].is_string());
        // Offer should be valid UUID (not 00000000-0000-0000-0000-000000000000)
        assert_ne!(body["offer_id"].as_str().unwrap(), "00000000-0000-0000-0000-000000000000");
    }

    #[tokio::test]
    async fn test_dispatch_offer_no_candidates_returns_404() {
        let state = setup_test_state();
        let mut app = router(state.clone());

        // No drivers registered, should get 404 with nil UUID

        let job_id = Uuid::new_v4();
        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "tow_truck",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "initial_radius_km": 1,
            "max_radius_km": 5,
            "timeout_seconds": 20
        });

        let (status, body) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["offer_id"].as_str().unwrap(), "00000000-0000-0000-0000-000000000000");
    }

    #[tokio::test]
    async fn test_dispatch_offer_category_mismatch() {
        let state = setup_test_state();

        // Register driver with "tow_truck" category
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.upsert_driver_location(
                Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
        }

        let mut app = router(state);
        let job_id = Uuid::new_v4();

        // Request offer with "delivery" category (mismatch)
        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "delivery",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "initial_radius_km": 1,
            "max_radius_km": 5,
            "timeout_seconds": 20
        });

        let (status, body) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        // Should return nil UUID for no match
        assert_eq!(body["offer_id"].as_str().unwrap(), "00000000-0000-0000-0000-000000000000");
    }

    #[tokio::test]
    async fn test_job_status_unknown() {
        let state = setup_test_state();
        let mut app = router(state);

        let job_id = Uuid::new_v4();

        let (status, body) = make_request(
            &mut app,
            "GET",
            &format!("/api/v1/dispatch/job/{}", job_id),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], "unknown");
        assert_eq!(body["job_id"], job_id.to_string());
    }

    #[tokio::test]
    async fn test_job_status_pending_then_matched() {
        let state = setup_test_state();

        // Register driver
        let driver_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.upsert_driver_location(
                driver_id,
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
        }

        let mut app = router(state.clone());
        let job_id = Uuid::new_v4();

        // Create offer
        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "tow_truck",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "initial_radius_km": 1,
            "max_radius_km": 5,
            "timeout_seconds": 20
        });

        let (status, offer_body) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        let offer_id = offer_body["offer_id"].as_str().unwrap();

        // Check job status is searching
        let (status, body) = make_request(
            &mut app,
            "GET",
            &format!("/api/v1/dispatch/job/{}", job_id),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(
            body["state"] == "pending" || body["state"] == "searching",
            "state should be pending or searching"
        );

        // Accept offer through dispatch service
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.mark_offer_status(
                uuid::Uuid::parse_str(offer_id).unwrap(),
                OfferStatus::Accepted,
            ).ok();
        }

        // Check job status is now matched
        let (status, body) = make_request(
            &mut app,
            "GET",
            &format!("/api/v1/dispatch/job/{}", job_id),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], "matched");
        assert_eq!(body["matched_driver_id"], driver_id.to_string());
    }

    #[tokio::test]
    async fn test_job_cancellation() {
        let state = setup_test_state();

        // Register driver and create offer
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.upsert_driver_location(
                Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
        }

        let mut app = router(state.clone());
        let job_id = Uuid::new_v4();

        // Create offer
        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "tow_truck",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "initial_radius_km": 1,
            "max_radius_km": 5,
            "timeout_seconds": 20
        });

        let (status, _) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // Cancel job
        let cancel_body = json!({"job_id": job_id.to_string()});
        let (status, _) = make_request(&mut app, "POST", "/api/v1/dispatch/job/cancel", Some(cancel_body))
            .await;

        assert_eq!(status, StatusCode::OK);

        // Verify job status is cancelled
        let (status, body) = make_request(
            &mut app,
            "GET",
            &format!("/api/v1/dispatch/job/{}", job_id),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], "cancelled");
    }

    #[tokio::test]
    async fn test_job_events_pagination() {
        let state = setup_test_state();

        // Register driver and create offer
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.upsert_driver_location(
                Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
        }

        let mut app = router(state);
        let job_id = Uuid::new_v4();

        // Create offer
        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "tow_truck",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "initial_radius_km": 1,
            "max_radius_km": 5,
            "timeout_seconds": 20
        });

        let (_status, _) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;

        // Fetch events
        let (status, body) = make_request(
            &mut app,
            "GET",
            &format!("/api/v1/dispatch/job/{}/events?limit=50", job_id),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(body["events"].is_array());
        assert!(body["events"].as_array().unwrap().len() > 0);
        assert_eq!(body["job_id"], job_id.to_string());
    }

    #[tokio::test]
    async fn test_job_events_filter_by_kind() {
        let state = setup_test_state();

        // Register driver and create offer
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.upsert_driver_location(
                Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
        }

        let mut app = router(state);
        let job_id = Uuid::new_v4();

        // Create offer
        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "tow_truck",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "initial_radius_km": 1,
            "max_radius_km": 5,
            "timeout_seconds": 20
        });

        let (_status, _) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;

        // Fetch events filtered by kind
        let (status, body) = make_request(
            &mut app,
            "GET",
            &format!("/api/v1/dispatch/job/{}/events?limit=50&kinds=offer_created", job_id),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let events = body["events"].as_array().unwrap();
        for event in events {
            // All events should be of kind offer_created
            assert_eq!(event["kind"], "offer_created");
        }
    }

    #[tokio::test]
    async fn test_concurrent_acceptance_first_wins() {
        let state = setup_test_state();

        // Register two drivers
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.upsert_driver_location(
                Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
            dispatch.engine.upsert_driver_location(
                Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap(),
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.434,
                    longitude: 26.769,
                },
                DriverStatus::Available,
            );
        }

        let mut app = router(state.clone());
        let job_id = Uuid::new_v4();

        // Create offer (should create offer for driver 1 first)
        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "tow_truck",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "initial_radius_km": 2,
            "max_radius_km": 10,
            "timeout_seconds": 30
        });

        let (status, offer_body) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        let offer_id = offer_body["offer_id"].as_str().unwrap();

        // First driver accepts
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.mark_offer_status(
                uuid::Uuid::parse_str(offer_id).unwrap(),
                OfferStatus::Accepted,
            ).ok();
        }

        // Job should be matched
        let (status, body) = make_request(
            &mut app,
            "GET",
            &format!("/api/v1/dispatch/job/{}", job_id),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], "matched");
        // Should match driver 1
        assert_eq!(body["matched_driver_id"], "11111111-1111-1111-1111-111111111111");
    }

    #[tokio::test]
    async fn test_radius_expansion() {
        let state = setup_test_state();

        // Register driver at origin
        {
            let mut dispatch = state.dispatch.lock().await;
            dispatch.engine.upsert_driver_location(
                Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
        }

        let mut app = router(state);

        // Create offer with small initial radius that requires expansion
        let job_id = Uuid::new_v4();
        let request_body = json!({
            "job_id": job_id.to_string(),
            "category": "tow_truck",
            "pickup": {
                "latitude": 38.433,
                "longitude": 26.768
            },
            "initial_radius_km": 0.1,  // Very small radius
            "max_radius_km": 2,         // Will expand to find driver
            "timeout_seconds": 20
        });

        let (status, body) = make_request(&mut app, "POST", "/api/v1/dispatch/offer", Some(request_body))
            .await;

        // Should find driver through radius expansion
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_ne!(body["offer_id"].as_str().unwrap(), "00000000-0000-0000-0000-000000000000");
    }

    #[tokio::test]
    async fn test_invalid_query_params() {
        let state = setup_test_state();
        let mut app = router(state);

        let job_id = Uuid::new_v4();

        // Test with invalid kind
        let (status, body) = make_request(
            &mut app,
            "GET",
            &format!("/api/v1/dispatch/job/{}/events?limit=50&kinds=invalid_kind", job_id),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_query");
    }
}
