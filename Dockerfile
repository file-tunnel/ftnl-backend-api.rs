FROM rust:1.96-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY schema ./schema
RUN cargo build --locked --release

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/ftnl-backend-api /usr/local/bin/ftnl-backend-api
EXPOSE 8080
ENV FTNL_BIND=0.0.0.0:8080
ENTRYPOINT ["/usr/local/bin/ftnl-backend-api"]
