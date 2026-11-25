# The deck's image: one static-ish binary (glibc, SQLite compiled in) on distroless, running as uid 65532.
# Build from the repository root: docker build -f deploy/Dockerfile -t quietdeck .
# The web app is built first, then embedded in the binary (crates/quietdeck-server/src/web.rs).

FROM node:24.22.2-bookworm-slim AS web
WORKDIR /web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml ./
RUN --mount=type=cache,target=/root/.local/share/pnpm/store pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

FROM rust:1.99.1-bookworm AS build
# The release's commit and build time (Unix seconds), for Settings: scripts/build-images passes both.
ARG QUIETDECK_COMMIT=
ARG QUIETDECK_BUILT_AT=
ENV QUIETDECK_COMMIT=$QUIETDECK_COMMIT QUIETDECK_BUILT_AT=$QUIETDECK_BUILT_AT
WORKDIR /src
COPY . .
COPY --from=web /web/dist web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quietdeck-deck-target,target=/src/target \
    cargo build --release --locked -p quietdeck-server \
    && cp target/release/quietdeck-server /quietdeck-server

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /quietdeck-server /usr/local/bin/quietdeck-server
USER 65532:65532
ENV QUIETDECK_DATA_DIR=/data
VOLUME /data
# The app (behind a private HTTPS ingress) and the webhook receiver (behind the public tunnel).
EXPOSE 8080 8081
ENTRYPOINT ["/usr/local/bin/quietdeck-server"]
CMD ["serve"]
