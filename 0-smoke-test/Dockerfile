FROM --platform=amd64 rust:latest as build
ADD . /smoke-test
WORKDIR /smoke-test
RUN cargo build --release

FROM --platform=amd64 gcr.io/distroless/cc
# ADD target/x86_64-unknown-linux-gnu/release/smoke-test /smoke-test
COPY --from=build /smoke-test/target/release/smoke-test /smoke-test
ENTRYPOINT [ "/smoke-test" ]
