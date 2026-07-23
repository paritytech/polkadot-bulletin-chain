FROM rust:1.88-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN cargo build --release -p bulletin-stress-test && \
    cp target/release/bulletin-stress-test /usr/local/bin/

# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/bulletin-stress-test /usr/local/bin/bulletin-stress-test

ENTRYPOINT ["bulletin-stress-test"]
