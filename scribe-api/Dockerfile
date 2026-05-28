FROM rust:1.95-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.69
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin scribe

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin scribe
WORKDIR /app
COPY --from=builder /app/target/release/scribe /usr/local/bin/scribe
COPY config.toml /app/config.toml
USER scribe
EXPOSE 8080
ENV RUST_LOG=scribe=info,scribe_api=info,scribe_services=info,scribe_providers=info,tower_http=info
ENTRYPOINT ["scribe"]
CMD ["serve"]
