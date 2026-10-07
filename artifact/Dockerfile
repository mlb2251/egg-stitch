# Build context is artifact/build/ (see artifact/prepare.sh).
FROM ubuntu:24.04

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git python3 python3-venv \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/venv/bin:/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.93.1

WORKDIR /artifact
COPY . .

RUN sed 's|"vendor"|"/artifact/vendor"|' cargo-config.toml > $CARGO_HOME/config.toml \
    && printf '\n[net]\noffline = true\n' >> $CARGO_HOME/config.toml

RUN python3 -m venv /opt/venv \
    && pip install --no-cache-dir --no-index --find-links wheels -r egg-stitch/requirements-lock.txt

RUN cd egg-stitch && cargo build --release --bin egg-stitch \
    && cd ../babble && cargo build --release --bin drawings --bin benchmark --bin molecules --bin circuits \
    && cd ../stitch && cargo build --release --bin compress

WORKDIR /artifact/egg-stitch
CMD ["bash"]
