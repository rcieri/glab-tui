FROM rust:alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release

FROM alpine:latest AS glab-fetch
ARG GLAB_VERSION=v1.106.0
ARG TARGETOS
ARG TARGETARCH
RUN apk add --no-cache curl tar
RUN case "${TARGETARCH}" in \
      amd64|x86_64) arch="amd64" ;; \
      arm64|aarch64) arch="arm64" ;; \
      *) echo "Unsupported architecture: ${TARGETARCH}"; exit 1 ;; \
    esac && \
    url="https://gitlab.com/gitlab-org/cli/-/releases/${GLAB_VERSION}/downloads/glab_${GLAB_VERSION#v}_${TARGETOS}_${arch}.tar.gz" && \
    curl -sSfL "$url" | tar -xz -C /tmp && \
    mv /tmp/bin/glab /usr/local/bin/

FROM alpine:latest
RUN apk add --no-cache github-cli
COPY --from=build /app/target/release/glab-tui /usr/local/bin/
COPY --from=glab-fetch /usr/local/bin/glab /usr/local/bin/
RUN adduser -D glab-tui
USER glab-tui
ENTRYPOINT ["glab-tui"]
