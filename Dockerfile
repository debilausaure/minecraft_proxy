# Build from debian rust image
# see this issue on why we don't build from alpine
# https://github.com/rust-lang/rust/issues/40174#issuecomment-538791091
FROM rust:alpine as builder

RUN apk add musl-dev --no-cache

COPY proxy/ .

# Compilation must be fully static, see
# https://doc.rust-lang.org/edition-guide/rust-2018/platform-and-target-support/musl-support-for-fully-static-binaries.html
RUN rustup target add x86_64-unknown-linux-musl\
 && cargo build --target x86_64-unknown-linux-musl --release \
 && cp -L /target/x86_64-unknown-linux-musl/release/minecraft_proxy /minecraft_proxy\
 && strip /minecraft_proxy

# Copy to an alpine image
FROM alpine:latest

RUN apk update && apk add curl --no-cache
# && addgroup -g 985 -S docker\
# && adduser minecraft_dockerd -S -G docker -u 973 -s /sbin/nologin

#USER minecraft_dockerd

#COPY --from=builder --chown=minecraft_dockerd:docker /minecraft_proxy .
COPY --from=builder /minecraft_proxy .

ENTRYPOINT ["./minecraft_proxy"]
# default listen addr, default forward addr
CMD ["minecraft_proxy_traefik:25565", "minecraft_server:25565"]
