# syntax=docker/dockerfile:1

# ---- 建置 ----
FROM rust:1.99-slim-bookworm AS builder
WORKDIR /app

# 先只放依賴描述檔建一次,讓依賴層能被快取(原始碼改動時不必重編所有依賴)
COPY Cargo.toml Cargo.lock build.rs ./
RUN mkdir -p src migrations \
    && echo 'fn main() {}' > src/main.rs \
    && : > src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY migrations ./migrations
COPY src ./src
# 時間戳要更新,否則 Cargo 可能認為上面那份空殼還是最新的
RUN touch src/main.rs src/lib.rs && cargo build --release --locked

# ---- 執行 ----
FROM debian:bookworm-slim
# 先升級基底映像裡已有的套件:映像標籤是移動的,但建置快取可能讓基底落後,CI 的漏洞掃描(Trivy)會抓到
RUN apt-get update \
    && apt-get upgrade -y --no-install-recommends \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin app

COPY --from=builder /app/target/release/tenant-saas /usr/local/bin/tenant-saas

USER app
ENV BIND_ADDR=0.0.0.0:3001 \
    APP_ENV=production \
    LOG_FORMAT=json
EXPOSE 3001

# 容器內沒有 curl,由程式自己送一個最小的 HTTP 請求到 /health
HEALTHCHECK --interval=15s --timeout=5s --start-period=20s --retries=3 \
    CMD ["tenant-saas", "healthcheck"]

# 子命令:serve(預設)、migrate(一次性,用有建表權限的帳號)、healthcheck
ENTRYPOINT ["tenant-saas"]
CMD ["serve"]
