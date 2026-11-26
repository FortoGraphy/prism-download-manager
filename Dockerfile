FROM ubuntu:22.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system prism && useradd --system --gid prism --create-home --home-dir /home/prism prism

COPY target/x86_64-unknown-linux-gnu/release/prism-web /usr/local/bin/prism-web

ENV PRISM_WEB_DATA_DIR=/data \
    PRISM_WEB_DOWNLOAD_DIR=/downloads

RUN mkdir -p /data /downloads && chown -R prism:prism /data /downloads

# timeout bounds a failed tool download so image builds cannot hang.
RUN timeout --kill-after=30s 300s /usr/local/bin/prism-web --warmup \
    && chown -R prism:prism /data /downloads

USER prism
VOLUME ["/data", "/downloads"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/prism-web", "--host", "0.0.0.0", "--port", "8080"]
