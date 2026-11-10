# syntax=docker/dockerfile:1

# ---- stage 1: build the React client --------------------------------------
FROM node:22-slim AS frontend
WORKDIR /app
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# ---- stage 2: build the Rust backend ---------------------------------------
FROM rust:1.97 AS backend
RUN apt-get update \
    && apt-get install -y --no-install-recommends libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/data/polar-heights.i16.gz ./data/polar-heights.i16.gz
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && cp target/release/hug-gamer-backend /app/hug-gamer-backend

# ---- stage 3: minimal runtime ----------------------------------------------
FROM debian:bookworm-slim AS runtime-base
RUN apt-get update \
    && apt-get install -y --no-install-recommends libssl3 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=backend /app/hug-gamer-backend /app/hug-gamer-backend
ENV HUG_GAMER_BIND=0.0.0.0:8787 \
    HUG_GAMER_STATIC=/app/static
EXPOSE 8787
ENTRYPOINT ["/app/hug-gamer-backend"]

# Local releases reuse the native frontend build, avoiding a second compilation.
# docker build --target local-runtime --build-context frontend-dist=frontend/dist .
FROM runtime-base AS local-runtime
COPY --from=frontend-dist / /app/static

# The default target remains a self-contained, reproducible container build.
FROM runtime-base AS runtime
COPY --from=frontend /app/dist /app/static
