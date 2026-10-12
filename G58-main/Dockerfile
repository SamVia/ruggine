# --- Stage 1: Chef (Dependency Caching) ---
# We use 'bullseye' (Debian 11) instead of 'latest' (Debian 12)
# to fix the GPG signature errors on your host.
FROM rust:bullseye as chef 
WORKDIR /app
RUN cargo install cargo-chef

# --- Stage 2: Planner ---
FROM chef as planner
COPY ruggine/ . 
RUN cargo chef prepare --recipe-path recipe.json

# --- Stage 3: Builder ---
FROM chef as builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies
RUN cargo chef cook --release --recipe-path recipe.json

# Copy source and build
COPY ruggine/ . 
RUN cargo build --release --bin server

# --- Stage 4: Runtime ---
# MUST match the builder's OS version (Bullseye) to avoid "glibc" errors
FROM debian:bullseye-slim as runtime
WORKDIR /app

# Install OpenSSL
RUN apt-get update && apt-get install -y \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary
COPY --from=builder /app/target/release/server /app/server

# Create persistence folders
RUN mkdir -p /app/data /app/logs

# Security: Run as non-root user
RUN useradd -m -u 1000 -U ruggine_user && \
    chown -R ruggine_user:ruggine_user /app

USER ruggine_user

EXPOSE 8080

CMD ["./server"]