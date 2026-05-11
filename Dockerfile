# Build Stage
FROM rust:1.95-slim-bookworm as builder

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the source code
COPY . .

# Build the application
# We use --release for a production-optimized binary
RUN cargo build --release

# Runtime Stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from the builder stage
COPY --from=builder /app/target/release/nexus_balancer /app/nexus_balancer

# Copy migrations (embedded in binary by sqlx::migrate!, but good to have if needed)
# COPY --from=builder /app/migrations /app/migrations

# Expose the default port
EXPOSE 3317

# Set environment variables
ENV RUST_LOG=info
ENV DATABASE_URL=sqlite:/app/data/nexus.db

# Create data directory for SQLite
RUN mkdir -p /app/data

# Run the binary
CMD ["/app/nexus_balancer"]
