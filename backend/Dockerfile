# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later

# Stage 1: Preparación con cargo-chef (versión fijada)
FROM lukemathwalker/cargo-chef:0.1.71-rust-1.85-bookworm AS chef
WORKDIR /app

# Stage 2: Generación del recipe de dependencias
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Compilación con caché de dependencias
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Compilar solo las dependencias externas para maximizar el uso de caché
RUN cargo chef cook --release --recipe-path recipe.json

# Compilar el código fuente del proyecto
COPY . .
RUN cargo build --release --bin baze-server

# Stage 4: Imagen de ejecución mínima con usuario sin privilegios
FROM debian:bookworm-slim AS runtime

# Instalar certificados CA para llamadas HTTPS salientes
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Crear usuario y grupo no root
RUN groupadd -g 10001 baze && \
    useradd -u 10001 -g baze -s /bin/sh -m baze

WORKDIR /app

# Copiar el binario compilado
COPY --from=builder /app/target/release/baze-server /usr/local/bin/baze-server

# Variables de entorno por defecto
ENV PORT=8080
ENV HOST=0.0.0.0
ENV ENVIRONMENT=production

# Cambiar a usuario no privilegiado
USER baze:baze

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/baze-server"]
