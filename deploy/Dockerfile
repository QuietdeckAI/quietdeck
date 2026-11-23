# The deck's image: one static-ish binary (glibc, SQLite compiled in) on distroless, running as uid 65532.
# Build from the repository root: docker build -f deploy/Dockerfile -t quietdeck-deck .
# The web app is embedded in the binary at build time (crates/quietdeck-deck/src/web.rs).

FROM rust:1.99.1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p quietdeck-deck \
    && cp target/release/quietdeck-deck /quietdeck-deck

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /quietdeck-deck /quietdeck-deck
USER 65532:65532
ENV QUIETDECK_DATA_DIR=/data
VOLUME /data
# The app (behind Tailscale Serve) and the webhook receiver (behind the public tunnel).
EXPOSE 8080 8081
ENTRYPOINT ["/quietdeck-deck"]
CMD ["serve"]
