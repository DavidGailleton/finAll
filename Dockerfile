# syntax=docker/dockerfile:1.7

########################################
# Build stage
########################################

FROM rust:1.92.0-trixie AS builder

ARG APP_NAME=my-app
ARG CARGO_LEPTOS_VERSION=0.3.6

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        clang \
        curl \
        libssl-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install cargo-leptos.
RUN curl \
      --proto '=https' \
      --tlsv1.2 \
      -LsSf \
      "https://github.com/leptos-rs/cargo-leptos/releases/download/v${CARGO_LEPTOS_VERSION}/cargo-leptos-installer.sh" \
    | sh

RUN rustup target add wasm32-unknown-unknown

WORKDIR /app

COPY . .

RUN mkdir -p /out \
    && --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
       --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
       --mount=type=cache,id=leptos-target,target=/app/target \
       cargo leptos build --release --locked \
    && install -Dm755 \
         "/app/target/release/${APP_NAME}" \
         /out/app \
    && cp -R /app/target/site /out/site

########################################
# Runtime stage
########################################

FROM debian:trixie-slim AS runtime

ARG APP_NAME=my-app

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        openssl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 app \
    && useradd \
         --system \
         --uid 10001 \
         --gid app \
         --home-dir /app \
         --create-home \
         app

WORKDIR /app

COPY --from=builder --chown=app:app /out/app /app/app
COPY --from=builder --chown=app:app /out/site /app/site

USER app

ENV LEPTOS_OUTPUT_NAME=${APP_NAME}
ENV LEPTOS_SITE_ADDR=0.0.0.0:8080
ENV LEPTOS_SITE_ROOT=/app/site
ENV LEPTOS_SITE_PKG_DIR=pkg
ENV LEPTOS_ENV=PROD
ENV RUST_LOG=info

EXPOSE 8080

HEALTHCHECK \
    --interval=15s \
    --timeout=5s \
    --start-period=10s \
    --retries=5 \
    CMD curl --fail --silent http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/app/app"]
