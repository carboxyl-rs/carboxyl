FROM rust:bookworm AS build

WORKDIR /app
COPY . .

RUN cargo build --release --bin carbonyl

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates fontconfig && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /app/target/release/carbonyl /usr/local/bin/carbonyl

ENTRYPOINT ["/usr/local/bin/carbonyl"]
