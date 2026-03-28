FROM debian:bookworm

RUN dpkg --add-architecture amd64 && apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    build-essential \
    pkg-config \
    libssl-dev \
    mingw-w64 \
    nsis \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install Rust via rustup for cross targets
ENV RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
RUN rustup target add x86_64-pc-windows-gnu

WORKDIR /work
