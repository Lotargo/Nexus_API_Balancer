# syntax=docker/dockerfile:1
# Build Stage
FROM rust:1.95-slim-bookworm as builder

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Copy source code and migrations (sqlx::migrate! needs them at compile time)
COPY src ./src
COPY migrations ./migrations

# Build with cache mounts for cargo registry and target directory
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    cp /app/target/release/nexus_balancer /app/nexus_balancer

# Runtime Stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from the builder stage
COPY --from=builder /app/nexus_balancer /app/nexus_balancer

# Expose the default port
EXPOSE 3317

# Set environment variables
ENV RUST_LOG=info
ENV DATABASE_URL=sqlite:/app/data/nexus.db

# Create data directory for SQLite
RUN mkdir -p /app/data

# Run the binary
CMD ["/app/nexus_balancer"]
