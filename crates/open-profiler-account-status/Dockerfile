FROM rust:bookworm AS build
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY src-tauri/ src-tauri/
RUN cargo build --locked --release -p opensoft-open-profiler-account-status

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /source/target/release/opensoft-open-profiler-account-status /usr/local/bin/openprofiler-account-status
USER 65532:65532
ENV STATUS_LISTEN=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/openprofiler-account-status"]
