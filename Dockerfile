FROM rust:alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release

FROM alpine:latest
RUN apk add --no-cache github-cli
COPY --from=build /app/target/release/glab-tui /usr/local/bin/
RUN adduser -D glab-tui
USER glab-tui
ENTRYPOINT ["glab-tui"]
