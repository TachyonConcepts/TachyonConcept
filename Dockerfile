FROM rust:latest
ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /app
RUN rustup install nightly
COPY . .
RUN RUSTFLAGS="-C target-cpu=native" cargo install --path .
RUN cargo clean
EXPOSE 8080
CMD /app/run_ub.sh