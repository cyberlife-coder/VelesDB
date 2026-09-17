# Build stage. Unversioned on purpose: the image carries rustup and a stable
# toolchain this build does not use; the build runs on the toolchain
# rust-toolchain.toml pins, installed below.
FROM rust:bookworm AS builder

LABEL maintainer="VelesDB Team <contact@wiscale.fr>"
LABEL version="6.0.0"

WORKDIR /app

# Install build dependencies
# hadolint ignore=DL3008
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# The toolchain rust-toolchain.toml pins -- the one CI tests -- installed
# from that file before anything reaches cargo.
COPY rust-toolchain.toml ./
RUN rustup toolchain install --no-self-update --profile minimal

# Copy manifests and source
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY integrations ./integrations

# Build the application
RUN cargo build --release --bin velesdb-server

# Runtime stage
FROM debian:bookworm-slim

LABEL maintainer="VelesDB Team <contact@wiscale.fr>"
LABEL version="6.0.0"

WORKDIR /app

# Install runtime dependencies
# hadolint ignore=DL3008
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -s /bin/false velesdb

# Copy binary from builder
COPY --from=builder /app/target/release/velesdb-server /usr/local/bin/

# Embed the license so the redistributed image carries its notices
# (VelesDB Core License 1.0 / ELv2 requires the license to travel with the Software).
COPY LICENSE /usr/local/share/doc/velesdb/LICENSE

# Create data directory
RUN mkdir -p /data && chown velesdb:velesdb /data

# Switch to non-root user
USER velesdb

# Expose port
EXPOSE 8080

# Set environment variables
ENV VELESDB_DATA_DIR=/data
ENV VELESDB_HOST=0.0.0.0
ENV VELESDB_PORT=8080
ENV RUST_LOG=info

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Run the server
ENTRYPOINT ["velesdb-server"]
