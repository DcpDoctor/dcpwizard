# Headless dcpwizard. Build: docker build -t dcpwizard .
# Create:   docker run -v /path/to/media:/data dcpwizard create --title "My Film" --video /data/master.mov --output /data/out
# REST API: docker run -p 8080:8080 -v /path/to/media:/data dcpwizard serve --bind 0.0.0.0:8080
# Watch:    docker run -v /path/to/incoming:/in -v /path/to/out:/out dcpwizard watch /in --output /out

ARG GROK_REF=v20.4.6
ARG FFMPEG_URL=https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-27-16-45/ffmpeg-n8.1.2-47-g156bb4d299-linux64-gpl-8.1.tar.xz

FROM ubuntu:24.04 AS grok
ARG GROK_REF
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN git init -q /tmp/grok-src \
    && git -C /tmp/grok-src fetch --depth 1 https://github.com/GrokImageCompression/grok.git "$GROK_REF" \
    && git -C /tmp/grok-src checkout -q FETCH_HEAD \
    && git -C /tmp/grok-src submodule update --init --depth 1 \
    && cmake -S /tmp/grok-src -B /tmp/grok-build \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/grok \
        -DGRK_BUILD_CORE_SWIG_BINDINGS=OFF \
    && cmake --build /tmp/grok-build --parallel "$(nproc)" \
    && cmake --install /tmp/grok-build \
    && mkdir -p /opt/grok-runtime \
    && cp -a /opt/grok/lib*/libgrokj2k*.so* /opt/grok-runtime/

FROM ubuntu:24.04 AS builder
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake curl ca-certificates pkg-config git libclang-dev \
    libssl-dev libxml2-dev libxerces-c-dev \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=/root/.cargo/bin:$PATH
COPY --from=grok /opt/grok /opt/grok
ENV PKG_CONFIG_PATH=/opt/grok/lib/pkgconfig:/opt/grok/lib64/pkgconfig
WORKDIR /src
COPY . .
RUN cargo build --release -p dcpwizard-cli --manifest-path rust/Cargo.toml

FROM ubuntu:24.04
ARG FFMPEG_URL
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3t64 libxml2 libxerces-c3.2t64 xmlsec1 fonts-dejavu-core ca-certificates curl xz-utils \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL --retry 5 --retry-all-errors -o /tmp/ffmpeg.tar.xz "$FFMPEG_URL" \
    && tar -xJf /tmp/ffmpeg.tar.xz -C /tmp \
    && install -m 755 /tmp/ffmpeg-*/bin/ffmpeg /tmp/ffmpeg-*/bin/ffprobe /usr/local/bin/ \
    && rm -rf /tmp/ffmpeg.tar.xz /tmp/ffmpeg-*
COPY --from=grok /opt/grok-runtime/ /usr/local/lib/
RUN ldconfig
COPY --from=builder /src/rust/target/release/dcpwizard /usr/local/bin/dcpwizard
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN useradd -m -s /bin/bash dcpwizard
USER dcpwizard
WORKDIR /data
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["--help"]
