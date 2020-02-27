# Build from debian rust image
# see this issue on why we don't build from alpine
# https://github.com/rust-lang/rust/issues/40174#issuecomment-538791091
FROM rust:slim as builder

COPY proxy/ .

# Compilation must be fully static, see
# https://doc.rust-lang.org/edition-guide/rust-2018/platform-and-target-support/musl-support-for-fully-static-binaries.html
RUN rustup target add x86_64-unknown-linux-musl\
 && cargo build --target x86_64-unknown-linux-musl --release -q\
 && cp -L /target/x86_64-unknown-linux-musl/release/minecraft_proxy /minecraft_proxy 

# Copy to a scratch image
FROM scratch

COPY --from=builder /minecraft_proxy .

ENTRYPOINT ["./minecraft_proxy"]
# default listen addr, default forward addr
CMD ["mc_proxy:25565", "mc_server:25565"]
