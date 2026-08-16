FROM rust:latest as builder
RUN apt-get update && apt-get install -y musl-tools
WORKDIR /usr/src/app
RUN rustup target add x86_64-unknown-linux-musl
COPY . .
RUN mkdir -p ./big_paragon_build/src
RUN echo 'fn main() {}' > ./big_paragon_build/src/main.rs
RUN cp ./Cargo.toml ./big_paragon_build/Cargo.toml
RUN cd ./big_paragon_build && cargo build --target=x86_64-unknown-linux-musl --release
COPY ./src ./big_paragon_build/src
RUN cd ./big_paragon_build && touch src/main.rs && cargo build --target=x86_64-unknown-linux-musl --release --bin big_paragon
RUN mkdir -p ./linkers/

FROM alpine:latest
WORKDIR /app
COPY --from=builder /usr/src/app/big_paragon_build/target/x86_64-unknown-linux-musl/release/big_paragon .
COPY --from=builder /usr/src/app/datasets/ ./datasets/
RUN mkdir -p ./linkers/
CMD ["./big_paragon", "api", "--port", "5054", "--matrix"]
