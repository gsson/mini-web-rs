ARG DIST_IMAGE=gcr.io/distroless/cc-debian12
ARG BUILD_IMAGE=rust:1.78-slim-bookworm

FROM $BUILD_IMAGE as cargo-chef
WORKDIR app
RUN cargo install cargo-chef --locked

FROM cargo-chef as planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM cargo-chef as cacher
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

FROM $BUILD_IMAGE as builder
WORKDIR app
COPY . .
COPY --from=cacher /app/target target
COPY --from=cacher $CARGO_HOME $CARGO_HOME
RUN cargo build --release

FROM $DIST_IMAGE
EXPOSE 3000
COPY --from=builder /app/target/release/mini-web /
CMD [ "./mini-web" ]
