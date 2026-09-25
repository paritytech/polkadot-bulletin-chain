# Keep in sync with CI_IMAGE in .github/env — the workspace tracks the toolchain
# shipped by paritytech/ci-unified.
FROM rust:1.93-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN cargo build --release --locked -p bulletin-stress-test && \
    cp target/release/bulletin-stress-test /usr/local/bin/

# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/bulletin-stress-test /usr/local/bin/bulletin-stress-test

ENTRYPOINT ["bulletin-stress-test"]
