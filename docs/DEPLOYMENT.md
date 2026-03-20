# Docker & Deployment Guide

This guide covers containerizing and deploying Spatiad in production environments.

## Docker Setup

### Build the Docker Image

```bash
# Build with default settings
docker build -f Dockerfile -t spatiad:latest .

# Build with specific Rust version
docker build -f Dockerfile --build-arg RUST_VERSION=1.75 -t spatiad:1.75 .
```

### Dockerfile

Create a `Dockerfile` in the repository root:

```dockerfile
# Stage 1: Builder
FROM rust:1.75-slim as builder

WORKDIR /build

# Install dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace
COPY rust/ ./rust/
WORKDIR /build/rust

# Build release binary
RUN cargo build -p spatiad-bin --release

# Stage 2: Runtime
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 spatiad

WORKDIR /home/spatiad

# Copy binary from builder
COPY --from=builder /build/rust/target/release/spatiad-bin /usr/local/bin/spatiad-bin
RUN chmod +x /usr/local/bin/spatiad-bin

# Switch to non-root user
USER spatiad

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

# Default port
EXPOSE 3000

# Run spatiad
ENTRYPOINT ["spatiad-bin"]
```

### Docker Compose

Create a `docker-compose.yml`:

```yaml
version: '3.8'

services:
  spatiad:
    build:
      context: .
      dockerfile: Dockerfile
    container_name: spatiad
    ports:
      - "3000:3000"
    environment:
      SPATIAD_LOG_LEVEL: info
      SPATIAD_BIND_ADDR: 0.0.0.0:3000
      SPATIAD_H3_RESOLUTION: 8
      SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN: 240
      SPATIAD_WS_RECONNECT_MAX_PER_MIN: 30
      SPATIAD_WEBHOOK_TIMEOUT_MS: 3000
      # Optional webhook configuration
      # SPATIAD_WEBHOOK_URL: http://webhook-receiver:4000/webhooks/spatiad
      # SPATIAD_WEBHOOK_SECRET: your-secret-key
      # Optional authentication
      # SPATIAD_DISPATCHER_TOKEN: dispatcher-token
      # SPATIAD_DRIVER_TOKEN: driver-token
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3000/health"]
      interval: 30s
      timeout: 3s
      retries: 3
      start_period: 5s

  # Optional: Webhook receiver example
  # webhook-receiver:
  #   build:
  #     context: typescript/examples/ride-dispatch
  #   environment:
  #     SPATIAD_WEBHOOK_SECRET: your-secret-key
  #   ports:
  #     - "4000:4000"
  #   depends_on:
  #     - spatiad
```

### Run with Docker Compose

```bash
# Start services (from repo root)
docker compose -f deploy/docker-compose.yml up -d

# View logs
docker compose -f deploy/docker-compose.yml logs -f spatiad

# Stop services
docker compose -f deploy/docker-compose.yml down
```

### Manual Docker Run

```bash
# Run with environment variables
docker run \
  -e SPATIAD_LOG_LEVEL=debug \
  -e SPATIAD_BIND_ADDR=0.0.0.0:3000 \
  -e SPATIAD_WEBHOOK_URL=http://example.com/webhooks/spatiad \
  -e SPATIAD_WEBHOOK_SECRET=dev-secret \
  -e SPATIAD_WEBHOOK_TIMEOUT_MS=3000 \
  -e SPATIAD_DISPATCHER_TOKEN=dispatcher-token \
  -e SPATIAD_DRIVER_TOKEN=driver-token \
  -p 3000:3000 \
  --name spatiad \
  spatiad:latest

# Test container
curl http://localhost:3000/health
```

---

## Kubernetes Deployment

### Namespace & ConfigMap

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: spatiad

---
apiVersion: v1
kind: ConfigMap
metadata:
  name: spatiad-config
  namespace: spatiad
data:
  SPATIAD_LOG_LEVEL: "info"
  SPATIAD_H3_RESOLUTION: "8"
  SPATIAD_BIND_ADDR: "0.0.0.0:3000"
  SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN: "240"
  SPATIAD_WS_RECONNECT_MAX_PER_MIN: "30"

---
apiVersion: v1
kind: Secret
metadata:
  name: spatiad-secrets
  namespace: spatiad
type: Opaque
data:
  # base64 encoded values
  SPATIAD_WEBHOOK_SECRET: ZGV2LXNlY3JldA==
  SPATIAD_DISPATCHER_TOKEN: ZGlzcGF0Y2hlci10b2tlbg==
  SPATIAD_DRIVER_TOKEN: ZHJpdmVyLXRva2Vu
```

### Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: spatiad
  namespace: spatiad
  labels:
    app: spatiad
spec:
  replicas: 1  # Single instance for MVP (in-memory state)
  selector:
    matchLabels:
      app: spatiad
  template:
    metadata:
      labels:
        app: spatiad
    spec:
      containers:
      - name: spatiad
        image: spatiad:latest
        imagePullPolicy: IfNotPresent
        ports:
        - containerPort: 3000
          name: http
          protocol: TCP
        envFrom:
        - configMapRef:
            name: spatiad-config
        - secretRef:
            name: spatiad-secrets
        livenessProbe:
          httpGet:
            path: /health
            port: http
          initialDelaySeconds: 5
          periodSeconds: 30
          timeoutSeconds: 3
          failureThreshold: 3
        readinessProbe:
          httpGet:
            path: /ready
            port: http
          initialDelaySeconds: 2
          periodSeconds: 10
          timeoutSeconds: 3
          failureThreshold: 1
        resources:
          requests:
            cpu: 100m
            memory: 128Mi
          limits:
            cpu: 500m
            memory: 512Mi
        securityContext:
          runAsNonRoot: true
          runAsUser: 1000
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: false
          capabilities:
            drop:
            - ALL

---
apiVersion: v1
kind: Service
metadata:
  name: spatiad
  namespace: spatiad
  labels:
    app: spatiad
spec:
  type: ClusterIP
  ports:
  - port: 3000
    targetPort: http
    protocol: TCP
    name: http
  selector:
    app: spatiad

---
apiVersion: v1
kind: Service
metadata:
  name: spatiad-nodeport
  namespace: spatiad
  labels:
    app: spatiad
spec:
  type: NodePort
  ports:
  - port: 3000
    targetPort: http
    protocol: TCP
    nodePort: 30000
    name: http
  selector:
    app: spatiad
```

### Deploy to Kubernetes

```bash
# Create namespace and secrets
kubectl apply -f deploy/kubernetes/deployment.yaml

# Check status
kubectl get pods -n spatiad
kubectl logs -n spatiad -f deployment/spatiad

# Port forward for local testing
kubectl port-forward -n spatiad svc/spatiad 3000:3000

# Access service
curl http://localhost:3000/health
```

### Horizontal Scaling (Post-MVP)

**Current limitation:** Spatiad stores all state in-memory. Single instance only (no data loss tolerated).

**For horizontal scaling, requires:**
1. Event persistence layer (PostgreSQL, Redis)
2. Shared state backend (Redis, DynamoDB)
3. Event sourcing architecture
4. Distributed lock mechanism

---

## Systemd Service

Create `/etc/systemd/system/spatiad.service`:

```ini
[Unit]
Description=Spatiad Spatial Dispatch Engine
Documentation=https://github.com/zubeyralmaho/spatiad
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=spatiad
Group=spatiad
WorkingDirectory=/opt/spatiad

# Environment variables
EnvironmentFile=/etc/spatiad/spatiad.env

# Start command
ExecStart=/usr/local/bin/spatiad-bin

# Restart policy
Restart=on-failure
RestartSec=10s

# Resource limits
LimitNOFILE=65536
LimitNPROC=4096

# Security
NoNewPrivileges=true
PrivateTmp=true

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=spatiad

[Install]
WantedBy=multi-user.target
```

Create `/etc/spatiad/spatiad.env`:

```bash
SPATIAD_LOG_LEVEL=info
SPATIAD_BIND_ADDR=0.0.0.0:3000
SPATIAD_H3_RESOLUTION=8
SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN=240
SPATIAD_WS_RECONNECT_MAX_PER_MIN=30
SPATIAD_WEBHOOK_URL=http://localhost:4000/webhooks/spatiad
SPATIAD_WEBHOOK_SECRET=dev-secret
SPATIAD_DISPATCHER_TOKEN=dispatcher-token
SPATIAD_DRIVER_TOKEN=driver-token
```

### Start Service

```bash
# Create non-root user
sudo useradd -m -s /bin/false spatiad

# Create application directory
sudo mkdir -p /opt/spatiad
sudo chown spatiad:spatiad /opt/spatiad

# Copy binary
sudo cp target/release/spatiad-bin /usr/local/bin/
sudo chmod +x /usr/local/bin/spatiad-bin

# Create config directory
sudo mkdir -p /etc/spatiad
sudo chown spatiad:spatiad /etc/spatiad
sudo chmod 750 /etc/spatiad

# Copy environment file
sudo cp /etc/spatiad/spatiad.env /etc/spatiad/
sudo chown spatiad:spatiad /etc/spatiad/spatiad.env
sudo chmod 600 /etc/spatiad/spatiad.env

# Enable and start service
sudo systemctl daemon-reload
sudo systemctl enable spatiad
sudo systemctl start spatiad

# Check status
sudo systemctl status spatiad
sudo journalctl -u spatiad -f
```

---

## Environment Validation

Before starting Spatiad, validate configuration:

```bash
#!/bin/bash
set -e

echo "Validating Spatiad configuration..."

# Check required environment
echo "✓ Checking SPATIAD_LOG_LEVEL..."
LOG_LEVEL=${SPATIAD_LOG_LEVEL:-info}
if [[ ! "$LOG_LEVEL" =~ ^(trace|debug|info|warn|error)$ ]]; then
    echo "✗ Invalid SPATIAD_LOG_LEVEL: $LOG_LEVEL"
    exit 1
fi

echo "✓ Checking SPATIAD_BIND_ADDR..."
BIND_ADDR=${SPATIAD_BIND_ADDR:-0.0.0.0:3000}
if [[ ! "$BIND_ADDR" =~ ^[0-9.]+:[0-9]+$ ]]; then
    echo "✗ Invalid SPATIAD_BIND_ADDR: $BIND_ADDR"
    exit 1
fi

echo "✓ Checking SPATIAD_H3_RESOLUTION..."
H3_RES=${SPATIAD_H3_RESOLUTION:-8}
if [[ ! "$H3_RES" =~ ^[0-9]+$ ]] || [ "$H3_RES" -gt 15 ]; then
    echo "✗ Invalid SPATIAD_H3_RESOLUTION: $H3_RES (must be 0-15)"
    exit 1
fi

echo "✓ Checking rate limits..."
DISPATCH_LIMIT=${SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN:-240}
WS_LIMIT=${SPATIAD_WS_RECONNECT_MAX_PER_MIN:-30}
if [[ ! "$DISPATCH_LIMIT" =~ ^[0-9]+$ ]] || [[ ! "$WS_LIMIT" =~ ^[0-9]+$ ]]; then
    echo "✗ Invalid rate limit configuration"
    exit 1
fi

echo "✓ All configuration valid!"
```

---

## Monitoring & Logging

### Prometheus Metrics (Post-MVP)

```yaml
# prometheus.yml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'spatiad'
    static_configs:
      - targets: ['localhost:3000']
    metrics_path: '/metrics'
```

### Log Aggregation

Example with ELK Stack:

```yaml
# filebeat.yml
filebeat.inputs:
  - type: log
    enabled: true
    paths:
      - /var/log/spatiad.log

output.elasticsearch:
  hosts: ["elasticsearch:9200"]

processors:
  - add_kubernetes_metadata:
      namespace: spatiad
```

### Health Check Script

```bash
#!/bin/bash

# Health check endpoint
HEALTH_URL="${SPATIAD_URL:-http://localhost:3000}/health"

echo "Checking Spatiad health..."
RESPONSE=$(curl -s -w "\n%{http_code}" "$HEALTH_URL")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | head -n-1)

if [ "$HTTP_CODE" -ne 200 ]; then
    echo "❌ Health check failed: HTTP $HTTP_CODE"
    exit 1
fi

if echo "$BODY" | jq -e '.status == "ok"' > /dev/null 2>&1; then
    echo "✅ Spatiad is healthy"
    exit 0
else
    echo "❌ Invalid health response: $BODY"
    exit 1
fi
```

---

## Production Checklist

Before deploying to production:

- [ ] Dockerfile optimized for security (non-root user, read-only root filesystem)
- [ ] Environment variables externalized from code
- [ ] Webhook URL configured and reachable from container
- [ ] Authentication tokens configured (if using SPATIAD_DISPATCHER_TOKEN, SPATIAD_DRIVER_TOKEN)
- [ ] Logging level set appropriately (debug/info)
- [ ] Rate limits configured for expected load
- [ ] Health checks configured (startup, liveness, readiness)
- [ ] Resource limits set (CPU, memory)
- [ ] Security context applied (non-root, no escalation)
- [ ] Monitoring/alerting configured
- [ ] Backup/recovery plan for in-memory state
- [ ] Graceful shutdown handling tested
- [ ] Network policies configured (if using Kubernetes)

---

## Troubleshooting Deployment

### Container fails to start

```bash
docker run -it spatiad:latest /bin/bash
# Inside container:
spatiad-bin  # try running directly to see errors
```

### Port already in use

```bash
# Find process using port 3000
lsof -i :3000

# Change bind address
docker run -e SPATIAD_BIND_ADDR=0.0.0.0:3001 -p 3001:3001 spatiad:latest
```

### Out of memory

```bash
# Reduce H3 resolution or max radius to reduce spatial index size
docker run -e SPATIAD_H3_RESOLUTION=7 spatiad:latest

# Check actual usage
docker stats spatiad
```

### WebSocket connections failing

```bash
# Ensure WebSocket port (3000) is open
docker run -p 3000:3000 spatiad:latest

# For Kubernetes, check service routing:
kubectl port-forward svc/spatiad 3000:3000
```
