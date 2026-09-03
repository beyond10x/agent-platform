# syntax=docker/dockerfile:1.7
FROM rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922 AS builder
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=secret,id=github_token,required=true \
    --mount=type=cache,id=b10x-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=b10x-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=agent-platform-target,target=/source/target,sharing=locked \
    token="$(cat /run/secrets/github_token)" && \
    git config --global url."https://x-access-token:${token}@github.com/".insteadOf "ssh://git@github.com/" && \
    cargo build --locked --release -p agent-platform && \
    git config --global --unset-all url."https://x-access-token:${token}@github.com/".insteadOf && \
    install -D /source/target/release/agent-platform /out/agent-platform

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:adcd20c7b4c988b73cbfbddb26d2eee574571e6d7c9ffea29b3821e0690efb77
ARG SOURCE_SHA=unknown
LABEL org.opencontainers.image.revision=$SOURCE_SHA \
      org.opencontainers.image.source="https://github.com/beyond10x/agent-platform"
COPY --from=builder /out/agent-platform /usr/local/bin/agent-platform
EXPOSE 8090
ENTRYPOINT ["/usr/local/bin/agent-platform"]
