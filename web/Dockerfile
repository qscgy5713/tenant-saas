# syntax=docker/dockerfile:1

# ---- 建置 ----
FROM node:22-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run build

# ---- 執行:nginx 提供靜態檔,並代理 /api/ 到後端 ----
FROM nginx:1.27-alpine
COPY --from=build /app/dist /usr/share/nginx/html
# 官方映像會把 /etc/nginx/templates/*.template 以環境變數展開成 /etc/nginx/conf.d/
COPY nginx.conf.template /etc/nginx/templates/default.conf.template
ENV API_UPSTREAM=http://app:3001
# 只展開我們要的變數,否則 nginx 自己的 $host、$remote_addr 會被當成空字串
ENV NGINX_ENVSUBST_FILTER=API_UPSTREAM
EXPOSE 80
