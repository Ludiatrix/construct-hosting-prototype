FROM rust:1.99-slim-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM python:3.12-slim-bookworm
WORKDIR /app
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt \
    && useradd --uid 10001 --create-home registry \
    && mkdir /data && chown registry:registry /data
COPY --from=build /app/target/release/construct-registry /usr/local/bin/construct-registry
COPY validate_usd.py ./
USER registry
ENV CONSTRUCT_BIND=0.0.0.0:8080 CONSTRUCT_DATA_DIR=/data
VOLUME ["/data"]
EXPOSE 8080
CMD ["construct-registry"]
