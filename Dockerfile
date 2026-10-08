FROM rust:1.99-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends cmake pkg-config clang && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home registry \
    && mkdir -p /app/certs \
    && curl -fsSL https://truststore.pki.rds.amazonaws.com/global/global-bundle.pem -o /app/certs/rds-ca-bundle.pem
COPY --from=build /app/target/release/construct-registry /usr/local/bin/construct-registry
USER registry
ENV CONSTRUCT_BIND=0.0.0.0:8080 CONSTRUCT_DATABASE_CA=/app/certs/rds-ca-bundle.pem
EXPOSE 8080
CMD ["construct-registry"]
