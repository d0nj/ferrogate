FROM node:22-bookworm-slim AS web-builder
WORKDIR /app
COPY package.json package-lock.json ./
COPY apps/web/package.json ./apps/web/package.json
RUN npm ci
COPY apps/web ./apps/web
RUN npm run build

FROM rust:bookworm AS rust-builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY --from=web-builder /app/apps/web/dist ./apps/web/dist
RUN cargo build --release --locked --bin ferrogate

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --shell /usr/sbin/nologin ferrogate \
    && mkdir -p /data \
    && chown ferrogate:ferrogate /data

COPY --from=rust-builder /app/target/release/ferrogate /usr/local/bin/ferrogate

ENV HOST=0.0.0.0 \
    PORT=8080 \
    DATABASE_URL=sqlite:///data/ferrogate.db?mode=rwc

VOLUME ["/data"]
EXPOSE 8080
USER ferrogate

ENTRYPOINT ["ferrogate"]
