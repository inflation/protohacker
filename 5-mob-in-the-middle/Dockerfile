# syntax=docker/dockerfile:1.2

ARG APP=mob-in-the-middle

FROM --platform=amd64 rust:latest as build
ARG APP
ADD . /app
WORKDIR /app
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && cp /app/target/release/$APP /$APP

FROM --platform=amd64 gcr.io/distroless/cc:latest
ARG APP
COPY --from=build /$APP /app
ENTRYPOINT [ "/app" ]
