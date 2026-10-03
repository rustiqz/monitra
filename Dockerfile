FROM scratch

ARG TARGETARCH
COPY --chown=10001:10001 docker/data/ /data/
COPY docker/release/${TARGETARCH}/monitra /usr/local/bin/monitra

ENV XDG_DATA_HOME=/data XDG_CONFIG_HOME=/data/config
USER 10001:10001
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/monitra"]
CMD ["start", "--bind", "0.0.0.0:8080"]
