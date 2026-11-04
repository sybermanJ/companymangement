FROM rust:1.97-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p cm-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 cm
COPY --from=build /src/target/release/cm-server /usr/local/bin/cm-server
USER cm
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/cm-server"]
