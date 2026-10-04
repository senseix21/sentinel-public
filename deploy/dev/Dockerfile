# Development image for the parts of the gate that need a platform `ort`
# supports.
#
# Debian rather than Alpine on purpose: ONNX Runtime's prebuilt binaries are
# glibc builds, and a musl image would fail to link them at the point where the
# whole reason for this image is to link them.
#
# linux/amd64 is native on an Intel development host, so this is a full-speed
# build rather than an emulated one, and x86_64-unknown-linux-gnu is a target
# `ort` publishes prebuilts for. See docs/adr/015-onnx-person-inference.md.
# Trixie rather than bookworm: ONNX Runtime's prebuilt binaries reference
# glibc 2.38 symbols (`__isoc23_strtol` and friends), and bookworm ships 2.36,
# so linking fails with undefined symbols that look like a code fault and are
# not one. Trixie carries glibc 2.41.
FROM rust:1.92-trixie

# Everything `make verify` invokes, so a container run is the same gate rather
# than a subset of it: the shell harnesses, both Python suites, the Node UI
# tests, and FFmpeg for the tests that are ignored by default but runnable here.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        ffmpeg \
        # ONNX Runtime is C++; without libstdc++ the link fails on
        # std::__cxx11::basic_string symbols.
        g++ \
        nodejs \
        python3 \
    && rm -rf /var/lib/apt/lists/*

# ONNX Runtime, supplied rather than linked.
#
# `ort` is built with `load-dynamic`, so it does not fetch or link the runtime:
# ONNX Runtime and libwebrtc each statically vendor protobuf and Abseil, and
# every binary above `crates/config` depends on both, so static linking collides
# on `google::protobuf::...` and the whole Abseil surface. Loading at runtime
# keeps ONNX Runtime's symbols in its own object.
#
# 1.28.3 matches the API level `ort` 2.0.0-rc.13 targets. Pinned, because the
# runtime is part of the contract rather than an incidental dependency.
ARG ONNXRUNTIME_VERSION=1.28.3
RUN curl -fsSL -o /tmp/onnxruntime.tgz \
        "https://github.com/microsoft/onnxruntime/releases/download/v${ONNXRUNTIME_VERSION}/onnxruntime-linux-x64-${ONNXRUNTIME_VERSION}.tgz" \
    && mkdir -p /opt/onnxruntime \
    && tar -xzf /tmp/onnxruntime.tgz -C /opt/onnxruntime --strip-components=1 \
    && rm /tmp/onnxruntime.tgz
# `ort` reads this to find the library. Without it, or with it pointing at a
# file that will not load, `ort` panics rather than returning an error, so the
# path is fixed here instead of being discovered.
ENV ORT_DYLIB_PATH=/opt/onnxruntime/lib/libonnxruntime.so

# A LiveKit server, so publication can be qualified against a real room rather
# than against a test double.
#
# The publication path was previously provable only up to the point where it
# hands media to a `Room`, because a `Session` holds a connected one: recovery
# after a failure could be tested, re-publishing a track after that recovery
# could not. `--dev` mode needs no TURN, no certificates and no external
# network, which is what makes this runnable inside the gate.
#
# Pinned for the same reason the ONNX runtime is: a server version is part of
# what an integration result means.
ARG LIVEKIT_VERSION=1.13.7
RUN curl -fsSL -o /tmp/livekit.tgz \
        "https://github.com/livekit/livekit/releases/download/v${LIVEKIT_VERSION}/livekit_${LIVEKIT_VERSION}_linux_amd64.tar.gz" \
    && tar -xzf /tmp/livekit.tgz -C /usr/local/bin livekit-server \
    && rm /tmp/livekit.tgz \
    && livekit-server --version

# The toolchain file pins the compiler; fetch it at build time so a container
# run does not pay for it, and fail the image build rather than the gate if the
# pin is unavailable.
COPY rust-toolchain.toml /tmp/rust-toolchain.toml
RUN cd /tmp && rustup show active-toolchain && rustup component add clippy rustfmt

# The workspace is bind-mounted at /work, so the host's `target/` is visible to
# this container. Both would otherwise write x86_64-apple-darwin and
# x86_64-unknown-linux-gnu artifacts into the same `target/debug`, and each
# build would evict the other's. Give the container its own directory under the
# same ignored tree, so host and container builds coexist and neither rebuilds
# because of the other.
ENV CARGO_TARGET_DIR=/work/target/container

WORKDIR /work

CMD ["bash"]
