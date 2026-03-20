# Build stage
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

# Runtime stage
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
