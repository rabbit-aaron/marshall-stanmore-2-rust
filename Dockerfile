FROM rust:1.87-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    libdbus-1-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
# Cache dependency compilation.
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

COPY src ./src
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libdbus-1-3 dbus bluez dumb-init ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/stanmore2 /usr/local/bin/stanmore2

ENTRYPOINT ["dumb-init", "--"]
CMD ["stanmore2"]
