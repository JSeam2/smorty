# Stage 1: Build dependencies only (cacheable layer)
FROM rust:1.89-bookworm AS chef

RUN cargo install cargo-chef
WORKDIR /app

# Stage 2: Prepare recipe
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Build dependencies (cached unless dependencies change)
FROM chef AS dependencies
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Stage 4: Build application
FROM chef AS builder
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY --from=dependencies /app/target target
COPY --from=dependencies /usr/local/cargo /usr/local/cargo

# Build the application
RUN cargo build --release --bin smorty

# Stage 5: Runtime - minimal image
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -m -u 1000 -s /bin/bash appuser

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/smorty /usr/local/bin/smorty

# Create directories for config and data
RUN mkdir -p /app/config /app/abi && \
    chown -R appuser:appuser /app

# Switch to non-root user
USER appuser

# Expose the application port (adjust if needed)
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

# Default command - can be overridden in docker-compose or k8s
CMD ["smorty", "serve"]