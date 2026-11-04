FROM node:22-slim AS web
WORKDIR /web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

FROM rust:1.97-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p cm-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 cm
COPY --from=build /src/target/release/cm-server /usr/local/bin/cm-server
COPY --from=web /web/dist /srv/web
ENV WEB_DIR=/srv/web
USER cm
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/cm-server"]
