# Use the official Python image from the Docker Hub
FROM python:3.10

# Set environment variables
ENV PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig
ENV PATH="/root/.cargo/bin:${PATH}"

# Install dependencies
RUN apt-get update && \
    apt-get install -y cmake libssl-dev pkg-config

# Install rust dependencies
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && \
    rustup default stable
RUN pip install maturin[patchelf]

COPY . /hussh
WORKDIR /hussh

# Build hussh with maturin
RUN maturin build
RUN pip install .[dev] --force-reinstall
