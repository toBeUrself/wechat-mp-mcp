FROM rust:1.94-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates

RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 wechat-mcp \
    && useradd --uid 10001 --gid wechat-mcp --no-create-home --shell /usr/sbin/nologin wechat-mcp

COPY --from=builder /app/target/release/wechat-mp-mcp /usr/local/bin/wechat-mp-mcp

USER wechat-mcp

ENV WECHAT_TRANSPORT=http \
    WECHAT_HTTP_BIND=0.0.0.0:8000 \
    WECHAT_MEDIA_ROOT=/data/media

EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8000/healthz"]

ENTRYPOINT ["wechat-mp-mcp"]
