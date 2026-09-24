ARG FLAVOR=cu126

FROM ghcr.io/astral-sh/uv:0.12 AS uv

FROM python:3.12-bookworm AS builder
ARG FLAVOR
ENV FLAVOR=${FLAVOR} \
    UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy \
    UV_PYTHON=/usr/local/bin/python3.12 \
    UV_PYTHON_DOWNLOADS=never \
    UV_PROJECT_ENVIRONMENT=/app/.venv
COPY --from=uv /uv /usr/local/bin/uv
COPY --chmod=0755 docker/uv-sync.sh /usr/local/bin/uv-sync
WORKDIR /app
COPY pyproject.toml uv.lock .python-version ./
RUN --mount=type=cache,target=/root/.cache/uv uv-sync --no-install-project
COPY src ./src
RUN --mount=type=cache,target=/root/.cache/uv uv-sync --no-editable

FROM python:3.12-slim-bookworm
ARG FLAVOR
ARG APP_VERSION=dev
RUN apt-get update \
    && if [ "$FLAVOR" = cpu ]; then triton_cc=""; else triton_cc="gcc libc6-dev"; fi \
    && apt-get install -y --no-install-recommends ffmpeg libchromaprint1 tini $triton_cc \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --user-group --no-create-home --home-dir /tmp --shell /usr/sbin/nologin worker \
    && install -d -o worker -g worker /models /work /run/worker
COPY --from=builder /app/.venv /app/.venv
COPY config /app/config
COPY contract/worker-contract.json /app/contract/worker-contract.json
ENV PATH=/app/.venv/bin:${PATH} \
    HOME=/tmp \
    PYTHONUNBUFFERED=1 \
    HF_HOME=/models \
    HF_HUB_OFFLINE=1 \
    MELBAND_ROFORMER_MODELS_PATH=/models/roformer \
    FASTTEXT_HOME=/models/fasttext \
    PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True \
    TOKENIZERS_PARALLELISM=false \
    WORKER__WORKER__BUILD=${APP_VERSION}-${FLAVOR}
WORKDIR /app
USER 10001:10001
HEALTHCHECK --interval=30s --timeout=5s --start-period=600s CMD ["python", "-m", "worker", "health"]
ENTRYPOINT ["tini", "--", "python", "-m", "worker"]
CMD ["serve"]
