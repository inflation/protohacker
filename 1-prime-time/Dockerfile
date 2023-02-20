FROM --platform=amd64 rust:latest as build
ADD . /app
WORKDIR /app
RUN apt-get update && apt-get install -y protobuf-compiler
RUN cargo build --release

FROM --platform=amd64 gcr.io/distroless/cc
COPY --from=build /app/target/release/prime-time /prime-time
ENTRYPOINT [ "/prime-time" ]
