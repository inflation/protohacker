FROM otel/opentelemetry-collector:latest

COPY otelcol.yaml /etc/otelcol/config.yaml

ENTRYPOINT [ "/otelcol", "--config", "/etc/otelcol/config.yaml"]
